//! The allowlist: who it keeps out at connection, and what the staff does with it: its two lists,
//! the answers to requests, the licences approved ahead, and its settings.

use jiff::Timestamp;
use observer_domain::allowlist::{
    self as rules, AllowlistSearch, AllowlistSettings, Approval, ApproveRequest, ApprovedPage,
    DeniedRequests, DenyRequests, MAX_NOTE_CHARS, NO_REQUEST, RequestPage,
};
use observer_domain::audit::AuditAction;
use observer_domain::bans::{MAX_MESSAGE_CHARS, clean_line};
use observer_domain::error::{ApiError, ErrorCode};
use observer_domain::players::SearchRefusal;
use observer_fxserver::bridge::{AllowlistEvent, Refusal, Refused};
use observer_store::allowlist::{self as stored, ListQuery, NewApproval, RequestCursor};
use serde_json::json;
use tracing::warn;

use super::Admission;
use crate::audit::Record;
use crate::bans::Seen;
use crate::secrets::random;
use crate::{Caller, Error};

const PAGE: u32 = 50;
/// The most requests kept waiting: past it, those tried longest ago go.
const MAX_WAITING: u32 = 2000;

/// What became of a refused connection's request.
enum Asked {
    /// The licence was approved meanwhile: the player gets in.
    Approved,
    /// The request's code, or none when it could not be kept.
    Waiting(Option<String>),
}

fn search(text: Option<&str>) -> Result<Option<AllowlistSearch>, Error> {
    match text.map(AllowlistSearch::parse).transpose() {
        Ok(search) => Ok(search.flatten()),
        Err(SearchRefusal::Address) => Err(ErrorCode::AddressSearch.into()),
        Err(SearchRefusal::Malformed) => Err(ErrorCode::InvalidSearch.into()),
    }
}

/// Where a page of requests ended, as the panel hands it back: the last try in milliseconds and
/// the row's id.
fn request_cursor(after: Option<&str>) -> Result<Option<RequestCursor>, Error> {
    let Some(after) = after else {
        return Ok(None);
    };
    after
        .split_once('.')
        .and_then(|(last_at, id)| {
            Some(RequestCursor {
                last_at: Timestamp::from_millisecond(last_at.parse().ok()?).ok()?,
                id: id.parse().ok()?,
            })
        })
        .map(Some)
        .ok_or_else(|| ErrorCode::InvalidRequest.into())
}

impl Admission {
    pub(super) fn zone(&self) -> String {
        self.inner.schedule.borrow().zone.clone()
    }

    /// The refusal of a player the allowlist keeps out, their request made or counted once more;
    /// `None` while it is off, for an approved player, or when the database cannot tell, so that a
    /// failing disk never locks everyone out.
    pub(super) async fn unapproved(&self, seen: &Seen) -> Option<Refusal> {
        let license = seen.license.clone();
        let found = self
            .inner
            .db
            .read({
                let license = license.clone();
                move |conn| -> Result<_, Error> {
                    let settings = stored::settings(conn)?;
                    if !settings.enabled {
                        return Ok(None);
                    }
                    let approved = match &license {
                        Some(license) => stored::is_approved(conn, license)?,
                        None => false,
                    };
                    Ok((!approved).then_some(settings))
                }
            })
            .await;
        let settings = match found {
            Ok(found) => found?,
            Err(error) => {
                warn!(%error, "a player could not be checked against the allowlist and gets in");
                return None;
            }
        };
        let request = match &license {
            Some(license) => match self.asked(license, seen.name.as_deref()).await {
                Asked::Approved => return None,
                Asked::Waiting(code) => code,
            },
            None => None,
        };
        let message = rules::refusal(
            &settings.message,
            request.as_deref().unwrap_or(NO_REQUEST),
            seen.name.as_deref().unwrap_or_default(),
        );
        Some(Refusal {
            message,
            event: Refused::Allowlist(AllowlistEvent {
                request,
                name: seen.name.clone(),
                license,
            }),
        })
    }

    /// Makes the licence's request, or counts this connection on it. A request that cannot be
    /// written leaves the player refused all the same, with no code to quote.
    async fn asked(&self, license: &str, name: Option<&str>) -> Asked {
        let now = self.inner.clock.now();
        let (license, name) = (license.to_owned(), name.map(str::to_owned));
        let asked = self
            .inner
            .db
            .write(move |tx| -> Result<_, Error> {
                // Approved since the allowlist was read: the staff answered while they connected.
                if stored::is_approved(tx, &license)? {
                    return Ok(None);
                }
                let mut code = rules::request_code(random()?);
                while stored::code_taken(tx, &code)? {
                    code = rules::request_code(random()?);
                }
                Ok(Some(stored::ask(
                    tx,
                    &license,
                    name.as_deref(),
                    &code,
                    now,
                    MAX_WAITING,
                )?))
            })
            .await;
        match asked {
            Ok(None) => Asked::Approved,
            Ok(Some(asked)) => {
                if asked.new {
                    self.refreshed().await;
                }
                Asked::Waiting(Some(asked.code))
            }
            Err(error) => {
                warn!(%error, "a refused player's request could not be kept");
                Asked::Waiting(None)
            }
        }
    }

    /// A page of the requests waiting, latest try first.
    pub async fn requests(&self, text: Option<&str>, after: Option<&str>) -> Result<RequestPage, Error> {
        let query = ListQuery {
            search: search(text)?,
            limit: PAGE + 1,
        };
        let after = request_cursor(after)?;
        let mut rows = self
            .inner
            .db
            .read(move |conn| stored::requests(conn, &query, after))
            .await?;
        let next = (rows.len() > PAGE as usize).then(|| {
            rows.truncate(PAGE as usize);
            rows.last()
                .map(|(_, cursor)| format!("{}.{}", cursor.last_at.as_millisecond(), cursor.id))
        });
        Ok(RequestPage {
            requests: rows.into_iter().map(|(request, _)| request).collect(),
            next: next.flatten(),
            zone: self.zone(),
        })
    }

    /// A page of the licences approved, latest first.
    pub async fn approved(&self, text: Option<&str>, after: Option<&str>) -> Result<ApprovedPage, Error> {
        let query = ListQuery {
            search: search(text)?,
            limit: PAGE + 1,
        };
        let after = after
            .map(|after| {
                after
                    .parse::<i64>()
                    .map_err(|_| Error::from(ErrorCode::InvalidRequest))
            })
            .transpose()?;
        let mut approved = self
            .inner
            .db
            .read(move |conn| stored::approvals(conn, &query, after))
            .await?;
        let next = (approved.len() > PAGE as usize).then(|| {
            approved.truncate(PAGE as usize);
            approved.last().map(|approval| approval.id.to_string())
        });
        Ok(ApprovedPage {
            approved,
            next: next.flatten(),
            zone: self.zone(),
        })
    }

    /// Approves the player of a request: they get in from their next connection.
    pub async fn approve_request(&self, caller: &Caller, code: &str) -> Result<Approval, Error> {
        let code = rules::parse_request_code(code).ok_or(ErrorCode::AllowlistRequestNotFound)?;
        let record = Record::by(caller, AuditAction::AllowlistRequestApproved).target(code.clone());
        let (by_user, by_name) = (caller.user_id, caller.username.clone());
        let now = self.inner.clock.now();
        let approval = self
            .inner
            .db
            .write(move |tx| -> Result<_, Error> {
                let request =
                    stored::remove_request(tx, &code)?.ok_or(ErrorCode::AllowlistRequestNotFound)?;
                // A licence approved ahead while its request waited keeps that approval: the
                // request only goes.
                stored::approve(
                    tx,
                    &NewApproval {
                        license: &request.license,
                        name: request.name.as_deref(),
                        note: None,
                        request: Some(&code),
                        by_user: Some(by_user),
                        by_name: &by_name,
                        at: now,
                    },
                )?;
                record
                    .details(json!({ "license": request.license, "playerName": request.name }))
                    .write(tx, now)?;
                stored::approval_of(tx, &request.license)?.ok_or_else(|| ErrorCode::ApprovalNotFound.into())
            })
            .await?;
        self.refreshed().await;
        Ok(approval)
    }

    /// Denies a request: it goes, and the player who tries again makes a new one.
    pub async fn deny_request(&self, caller: &Caller, code: &str) -> Result<(), Error> {
        let code = rules::parse_request_code(code).ok_or(ErrorCode::AllowlistRequestNotFound)?;
        let record = Record::by(caller, AuditAction::AllowlistRequestDenied).target(code.clone());
        let now = self.inner.clock.now();
        self.inner
            .db
            .write(move |tx| -> Result<(), Error> {
                let request =
                    stored::remove_request(tx, &code)?.ok_or(ErrorCode::AllowlistRequestNotFound)?;
                Ok(record
                    .details(json!({ "license": request.license, "playerName": request.name }))
                    .write(tx, now)?)
            })
            .await?;
        self.refreshed().await;
        Ok(())
    }

    /// Denies every request last tried at `until` or before.
    pub async fn deny_requests(
        &self,
        caller: &Caller,
        request: DenyRequests,
    ) -> Result<DeniedRequests, Error> {
        let record = Record::by(caller, AuditAction::AllowlistRequestsDenied);
        let now = self.inner.clock.now();
        let denied = self
            .inner
            .db
            .write(move |tx| -> Result<u32, Error> {
                let denied = stored::remove_requests_until(tx, request.until)?;
                if denied > 0 {
                    record
                        .details(json!({ "denied": denied, "until": request.until }))
                        .write(tx, now)?;
                }
                Ok(denied)
            })
            .await?;
        self.refreshed().await;
        Ok(DeniedRequests { denied })
    }

    /// Approves a licence ahead of its player's first visit, with a note for the staff. A request
    /// the licence had waiting goes with it.
    pub async fn approve(&self, caller: &Caller, request: ApproveRequest) -> Result<Approval, Error> {
        let invalid = |field: &str| {
            Error::from(
                ApiError::new(ErrorCode::InvalidApproval)
                    .with("field", field)
                    .with("maxNote", MAX_NOTE_CHARS),
            )
        };
        let license = rules::parse_license(&request.license).ok_or_else(|| invalid("license"))?;
        let note = match request.note.as_deref().map(str::trim) {
            None | Some("") => None,
            Some(text) => Some(clean_line(text, MAX_NOTE_CHARS).ok_or_else(|| invalid("note"))?),
        };
        let record = Record::by(caller, AuditAction::AllowlistLicenceApproved)
            .details(json!({ "license": license, "note": note }));
        let (by_user, by_name) = (caller.user_id, caller.username.clone());
        let now = self.inner.clock.now();
        let approval = self
            .inner
            .db
            .write(move |tx| -> Result<_, Error> {
                let id = stored::approve(
                    tx,
                    &NewApproval {
                        license: &license,
                        name: None,
                        note: note.as_deref(),
                        request: None,
                        by_user: Some(by_user),
                        by_name: &by_name,
                        at: now,
                    },
                )?
                .ok_or(ErrorCode::AlreadyApproved)?;
                record.write(tx, now)?;
                stored::approval_by_id(tx, id)?.ok_or_else(|| ErrorCode::ApprovalNotFound.into())
            })
            .await?;
        self.refreshed().await;
        Ok(approval)
    }

    /// Takes an approval back: the player is refused from their next connection. One who plays
    /// stays until they leave.
    pub async fn remove_approval(&self, caller: &Caller, id: i64) -> Result<(), Error> {
        let record = Record::by(caller, AuditAction::AllowlistApprovalRemoved);
        let now = self.inner.clock.now();
        self.inner
            .db
            .write(move |tx| -> Result<(), Error> {
                let approval = stored::remove_approval(tx, id)?.ok_or(ErrorCode::ApprovalNotFound)?;
                Ok(record
                    .details(json!({
                        "license": approval.license,
                        "playerName": approval.name,
                        "note": approval.note,
                    }))
                    .write(tx, now)?)
            })
            .await?;
        self.refreshed().await;
        Ok(())
    }

    pub async fn allowlist_settings(&self) -> Result<AllowlistSettings, Error> {
        Ok(self.inner.db.read(stored::settings).await?)
    }

    /// Turns the allowlist on or off, or changes its message, from the next connection.
    pub async fn change_allowlist_settings(
        &self,
        caller: &Caller,
        settings: AllowlistSettings,
    ) -> Result<(), Error> {
        let settings = settings.checked().ok_or_else(|| {
            Error::from(
                ApiError::new(ErrorCode::InvalidAllowlistSettings).with("maxMessage", MAX_MESSAGE_CHARS),
            )
        })?;
        let before = self.allowlist_settings().await?;
        if before == settings {
            return Ok(());
        }
        let record = Record::by(caller, AuditAction::AllowlistSettingsChanged)
            .details(json!({ "from": before, "to": settings }));
        let now = self.inner.clock.now();
        self.inner
            .db
            .write(move |tx| -> Result<(), Error> {
                stored::save_settings(tx, &settings)?;
                Ok(record.write(tx, now)?)
            })
            .await?;
        self.refreshed().await;
        Ok(())
    }
}

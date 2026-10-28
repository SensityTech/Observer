//! Who may change whose account. The rules only look at permission sets; the caller loads the
//! accounts, checks the version and stores the result.

use crate::permissions::Permissions;

/// An account as the delegation rules see it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Account {
    pub id: i64,
    pub is_owner: bool,
    pub permissions: Permissions,
}

impl Account {
    /// What the account holds: every permission for the owner, the closed set for anyone else.
    pub fn effective(&self) -> Permissions {
        if self.is_owner {
            Permissions::all()
        } else {
            self.permissions.closed()
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// Nobody manages their own account through these rules.
    SelfEdit,
    /// The owner is out of everyone else's reach.
    OwnerProtected,
    /// The change grants or revokes these permissions, which the editor does not hold.
    Escalation(Permissions),
    /// The target holds permissions the editor does not.
    Hierarchy,
}

/// The permissions a new account gets, when `editor` may grant them.
pub fn grant(editor: &Account, requested: Permissions) -> Result<Permissions, Refusal> {
    let granted = requested.closed();
    let beyond = granted & !editor.effective();
    if beyond.is_empty() {
        Ok(granted)
    } else {
        Err(Refusal::Escalation(beyond))
    }
}

/// The permissions `target` ends up with, when `editor` may set them to `requested`. Only the bits
/// that change must be held by the editor: whatever else the target has stays as it is.
pub fn change(editor: &Account, target: &Account, requested: Permissions) -> Result<Permissions, Refusal> {
    out_of_reach(editor, target)?;
    let next = requested.closed();
    let beyond = (target.effective() ^ next) & !editor.effective();
    if beyond.is_empty() {
        Ok(next)
    } else {
        Err(Refusal::Escalation(beyond))
    }
}

/// Whether `editor` may delete `target`, reset their password or end their sessions: nobody takes
/// over an account that holds more than they do.
pub fn manage(editor: &Account, target: &Account) -> Result<(), Refusal> {
    out_of_reach(editor, target)?;
    if editor.is_owner || editor.effective().contains(target.effective()) {
        Ok(())
    } else {
        Err(Refusal::Hierarchy)
    }
}

fn out_of_reach(editor: &Account, target: &Account) -> Result<(), Refusal> {
    if editor.id == target.id {
        Err(Refusal::SelfEdit)
    } else if target.is_owner {
        Err(Refusal::OwnerProtected)
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permissions::Permissions as P;

    const OWNER: Account = Account {
        id: 1,
        is_owner: true,
        permissions: P::NONE,
    };

    fn user(id: i64, permissions: P) -> Account {
        Account {
            id,
            is_owner: false,
            permissions,
        }
    }

    /// A user manager who also runs the console and reads the audit log.
    fn manager() -> Account {
        user(2, P::USERS_MANAGE | P::CONSOLE_WRITE | P::AUDIT_VIEW)
    }

    #[test]
    fn granting_permissions_to_a_new_account() {
        let cases = [
            (
                "the owner grants anything",
                OWNER,
                P::SETTINGS_MANAGE | P::USERS_MANAGE,
                Ok(P::SETTINGS_MANAGE | P::SETTINGS_VIEW | P::USERS_MANAGE),
            ),
            (
                "a manager grants what they hold",
                manager(),
                P::CONSOLE_VIEW | P::AUDIT_VIEW,
                Ok(P::CONSOLE_VIEW | P::AUDIT_VIEW),
            ),
            (
                "implications of a held permission are held too",
                manager(),
                P::CONSOLE_WRITE,
                Ok(P::CONSOLE_WRITE | P::CONSOLE_VIEW),
            ),
            ("nothing is always grantable", manager(), P::NONE, Ok(P::NONE)),
            (
                "a manager cannot grant what they lack",
                manager(),
                P::AUDIT_VIEW | P::SERVER_CONTROL | P::LOGS_DOWNLOAD,
                Err(Refusal::Escalation(P::SERVER_CONTROL | P::LOGS_DOWNLOAD)),
            ),
            (
                "an implied permission the manager lacks is refused with it",
                manager(),
                P::SETTINGS_MANAGE,
                Err(Refusal::Escalation(P::SETTINGS_MANAGE | P::SETTINGS_VIEW)),
            ),
        ];

        for (name, editor, requested, expected) in cases {
            assert_eq!(grant(&editor, requested), expected, "{name}");
        }
    }

    #[test]
    fn changing_the_permissions_of_an_account() {
        let operator = user(3, P::CONSOLE_VIEW | P::SERVER_CONTROL);
        let cases = [
            (
                "the owner changes anything",
                OWNER,
                operator,
                P::SETTINGS_MANAGE,
                Ok(P::SETTINGS_MANAGE | P::SETTINGS_VIEW),
            ),
            (
                "a manager adds what they hold and keeps what they lack",
                manager(),
                operator,
                P::CONSOLE_WRITE | P::SERVER_CONTROL | P::AUDIT_VIEW,
                Ok(P::CONSOLE_WRITE | P::CONSOLE_VIEW | P::SERVER_CONTROL | P::AUDIT_VIEW),
            ),
            (
                "a manager removes what they hold",
                manager(),
                operator,
                P::SERVER_CONTROL,
                Ok(P::SERVER_CONTROL),
            ),
            (
                "an unchanged set is always allowed",
                manager(),
                operator,
                P::CONSOLE_VIEW | P::SERVER_CONTROL,
                Ok(P::CONSOLE_VIEW | P::SERVER_CONTROL),
            ),
            (
                "a manager cannot grant what they lack",
                manager(),
                operator,
                P::CONSOLE_VIEW | P::SERVER_CONTROL | P::ARTIFACTS_MANAGE,
                Err(Refusal::Escalation(P::ARTIFACTS_MANAGE | P::ARTIFACTS_VIEW)),
            ),
            (
                "a manager cannot revoke what they lack",
                manager(),
                operator,
                P::CONSOLE_VIEW,
                Err(Refusal::Escalation(P::SERVER_CONTROL)),
            ),
            (
                "nobody edits their own permissions, not even the owner",
                OWNER,
                OWNER,
                P::NONE,
                Err(Refusal::SelfEdit),
            ),
            (
                "a manager cannot edit their own permissions",
                manager(),
                manager(),
                P::USERS_MANAGE,
                Err(Refusal::SelfEdit),
            ),
            (
                "nobody else edits the owner",
                manager(),
                OWNER,
                P::NONE,
                Err(Refusal::OwnerProtected),
            ),
        ];

        for (name, editor, target, requested, expected) in cases {
            assert_eq!(change(&editor, &target, requested), expected, "{name}");
        }
    }

    #[test]
    fn deleting_resetting_or_signing_out_an_account() {
        let cases = [
            ("the owner manages anyone", OWNER, user(3, P::all()), Ok(())),
            (
                "a manager manages an account with less",
                manager(),
                user(3, P::CONSOLE_VIEW | P::AUDIT_VIEW),
                Ok(()),
            ),
            (
                "a manager manages an account with as much",
                manager(),
                user(3, manager().permissions),
                Ok(()),
            ),
            (
                "a manager cannot manage an account with more",
                manager(),
                user(3, P::CONSOLE_VIEW | P::SERVER_CONTROL),
                Err(Refusal::Hierarchy),
            ),
            (
                "nobody manages their own account",
                manager(),
                manager(),
                Err(Refusal::SelfEdit),
            ),
            ("not even the owner", OWNER, OWNER, Err(Refusal::SelfEdit)),
            (
                "nobody else manages the owner",
                manager(),
                OWNER,
                Err(Refusal::OwnerProtected),
            ),
        ];

        for (name, editor, target, expected) in cases {
            assert_eq!(manage(&editor, &target), expected, "{name}");
        }
    }

    #[test]
    fn the_owner_holds_every_permission_whatever_is_stored() {
        assert_eq!(OWNER.effective(), P::all());
        assert_eq!(
            user(3, P::ARTIFACTS_MANAGE).effective(),
            P::ARTIFACTS_MANAGE | P::ARTIFACTS_VIEW
        );
    }
}

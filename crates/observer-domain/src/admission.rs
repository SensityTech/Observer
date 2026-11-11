//! Who the server lets in: a ban refuses first, then maintenance, then the allowlist.

use serde::Serialize;
use ts_rs::TS;

use crate::allowlist::AllowlistStatus;
use crate::maintenance::MaintenanceStatus;

/// Who the server lets in, as it changes: the `server.admission` topic.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AdmissionStatus {
    pub allowlist: AllowlistStatus,
    pub maintenance: MaintenanceStatus,
}

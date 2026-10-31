use std::fmt;
use std::ops::{BitAnd, BitOr, BitXor, Not};

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use ts_rs::TS;

/// A set of permissions, one bit each. Bits are append-only: a bit is never renumbered and a
/// retired one is never reused, since stored sets and the panel both depend on them.
///
/// On the wire it is lowercase hexadecimal (`"4c"`), which the panel reads as a `BigInt`. In the
/// database it is a big-endian BLOB without leading zero bytes, so it widens without a migration.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, TS)]
#[ts(export, type = "string")]
pub struct Permissions(u128);

impl Permissions {
    pub const NONE: Self = Self(0);
    pub const DASHBOARD_VIEW: Self = Self(1 << 0);
    pub const CONSOLE_VIEW: Self = Self(1 << 1);
    pub const CONSOLE_WRITE: Self = Self(1 << 2);
    pub const SERVER_CONTROL: Self = Self(1 << 3);
    pub const SCHEDULE_MANAGE: Self = Self(1 << 4);
    pub const ARTIFACTS_VIEW: Self = Self(1 << 5);
    pub const ARTIFACTS_MANAGE: Self = Self(1 << 6);
    pub const USERS_MANAGE: Self = Self(1 << 7);
    pub const AUDIT_VIEW: Self = Self(1 << 8);
    pub const SETTINGS_VIEW: Self = Self(1 << 9);
    pub const SETTINGS_MANAGE: Self = Self(1 << 10);
    pub const LOGS_DOWNLOAD: Self = Self(1 << 11);
    pub const DIAGNOSTICS_VIEW: Self = Self(1 << 12);
    pub const FILES_MANAGE: Self = Self(1 << 13);
    pub const PLAYERS_VIEW: Self = Self(1 << 14);
    pub const PLAYER_IDENTIFIERS_VIEW: Self = Self(1 << 15);
    pub const CHANGES_VIEW: Self = Self(1 << 16);
    pub const CHANGES_CONTENT: Self = Self(1 << 17);

    /// Every permission in the catalog, which is what the owner holds.
    pub fn all() -> Self {
        CATALOG
            .iter()
            .fold(Self::NONE, |all, info| all | info.permission())
    }

    pub const fn bits(self) -> u128 {
        self.0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// The bits the catalog does not know.
    #[must_use]
    pub fn unknown(self) -> Self {
        self & !Self::all()
    }

    /// Drops unknown bits and adds every permission the remaining ones imply, directly or not.
    #[must_use]
    pub fn closed(self) -> Self {
        let mut closed = self & Self::all();
        loop {
            let next = CATALOG
                .iter()
                .filter(|info| closed.contains(info.permission()))
                .fold(closed, |set, info| set | info.implies);
            if next == closed {
                return closed;
            }
            closed = next;
        }
    }

    /// Reads a stored set. Unknown bits are dropped rather than trusted, and implications added
    /// later apply to sets stored before them.
    pub fn from_blob(blob: &[u8]) -> Self {
        // Bytes before the last sixteen could only hold bits far beyond the catalog.
        let start = blob.len().saturating_sub(16);
        let bits = blob[start..]
            .iter()
            .fold(0u128, |bits, &byte| bits << 8 | u128::from(byte));
        Self(bits).closed()
    }

    pub fn to_blob(self) -> Vec<u8> {
        let bytes = self.0.to_be_bytes();
        let first = bytes.iter().position(|&byte| byte != 0).unwrap_or(bytes.len());
        bytes[first..].to_vec()
    }

    pub fn to_hex(self) -> String {
        format!("{:x}", self.0)
    }

    /// Parses the wire form. Unknown bits are kept, so the caller can refuse them by name.
    pub fn from_hex(text: &str) -> Result<Self, InvalidPermissions> {
        if text.is_empty() || text.len() > 32 || !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(InvalidPermissions);
        }
        u128::from_str_radix(text, 16)
            .map(Self)
            .map_err(|_| InvalidPermissions)
    }

    /// The catalog entries of the permissions in this set, by bit.
    pub fn iter(self) -> impl Iterator<Item = &'static PermissionInfo> {
        CATALOG
            .iter()
            .filter(move |info| self.contains(info.permission()))
    }
}

impl BitOr for Permissions {
    type Output = Self;

    fn bitor(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

impl BitAnd for Permissions {
    type Output = Self;

    fn bitand(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }
}

impl BitXor for Permissions {
    type Output = Self;

    fn bitxor(self, other: Self) -> Self {
        Self(self.0 ^ other.0)
    }
}

impl Not for Permissions {
    type Output = Self;

    fn not(self) -> Self {
        Self(!self.0)
    }
}

impl Serialize for Permissions {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for Permissions {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        Self::from_hex(&text).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidPermissions;

impl fmt::Display for InvalidPermissions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("permissions are a hexadecimal bitset of at most 128 bits")
    }
}

impl std::error::Error for InvalidPermissions {}

/// How much harm a permission can do in the wrong hands. The panel marks it in the permission
/// editor, and refusals of sensitive or critical actions are audited.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum Danger {
    Normal,
    Sensitive,
    Critical,
}

/// The group a permission belongs to in the permission editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub enum PermissionArea {
    Dashboard,
    Console,
    Server,
    Schedule,
    Artifacts,
    Users,
    Audit,
    Settings,
    Logs,
    Diagnostics,
    Files,
    Players,
    Changes,
}

/// One permission of the catalog, as `GET /api/permissions` returns it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct PermissionInfo {
    pub bit: u32,
    /// The Rust constant, such as `CONSOLE_WRITE`. The panel's generated one is in `PascalCase`.
    #[serde(skip)]
    pub constant: &'static str,
    /// The permission's key in the panel's message catalog, such as `consoleWrite`.
    pub key: &'static str,
    pub area: PermissionArea,
    pub danger: Danger,
    /// Permissions granted along with this one.
    pub implies: Permissions,
}

impl PermissionInfo {
    pub const fn permission(&self) -> Permissions {
        Permissions(1 << self.bit)
    }
}

/// Every permission, by bit.
pub const CATALOG: &[PermissionInfo] = &[
    PermissionInfo {
        bit: 0,
        constant: "DASHBOARD_VIEW",
        key: "dashboardView",
        area: PermissionArea::Dashboard,
        danger: Danger::Normal,
        implies: Permissions::NONE,
    },
    PermissionInfo {
        bit: 1,
        constant: "CONSOLE_VIEW",
        key: "consoleView",
        area: PermissionArea::Console,
        // Resources print player addresses and the occasional secret.
        danger: Danger::Sensitive,
        implies: Permissions::NONE,
    },
    PermissionInfo {
        bit: 2,
        constant: "CONSOLE_WRITE",
        key: "consoleWrite",
        area: PermissionArea::Console,
        // `exec`, `add_ace` and friends reach everything the server can.
        danger: Danger::Critical,
        implies: Permissions::CONSOLE_VIEW,
    },
    PermissionInfo {
        bit: 3,
        constant: "SERVER_CONTROL",
        key: "serverControl",
        area: PermissionArea::Server,
        danger: Danger::Sensitive,
        implies: Permissions::NONE,
    },
    PermissionInfo {
        bit: 4,
        constant: "SCHEDULE_MANAGE",
        key: "scheduleManage",
        area: PermissionArea::Schedule,
        danger: Danger::Sensitive,
        implies: Permissions::NONE,
    },
    PermissionInfo {
        bit: 5,
        constant: "ARTIFACTS_VIEW",
        key: "artifactsView",
        area: PermissionArea::Artifacts,
        danger: Danger::Normal,
        implies: Permissions::NONE,
    },
    PermissionInfo {
        bit: 6,
        constant: "ARTIFACTS_MANAGE",
        key: "artifactsManage",
        area: PermissionArea::Artifacts,
        // It changes the binary the host runs.
        danger: Danger::Critical,
        implies: Permissions::ARTIFACTS_VIEW,
    },
    PermissionInfo {
        bit: 7,
        constant: "USERS_MANAGE",
        key: "usersManage",
        area: PermissionArea::Users,
        danger: Danger::Critical,
        implies: Permissions::NONE,
    },
    PermissionInfo {
        bit: 8,
        constant: "AUDIT_VIEW",
        key: "auditView",
        area: PermissionArea::Audit,
        danger: Danger::Sensitive,
        implies: Permissions::NONE,
    },
    PermissionInfo {
        bit: 9,
        constant: "SETTINGS_VIEW",
        key: "settingsView",
        area: PermissionArea::Settings,
        danger: Danger::Sensitive,
        implies: Permissions::NONE,
    },
    PermissionInfo {
        bit: 10,
        constant: "SETTINGS_MANAGE",
        key: "settingsManage",
        area: PermissionArea::Settings,
        danger: Danger::Critical,
        implies: Permissions::SETTINGS_VIEW,
    },
    PermissionInfo {
        bit: 11,
        constant: "LOGS_DOWNLOAD",
        key: "logsDownload",
        area: PermissionArea::Logs,
        danger: Danger::Sensitive,
        implies: Permissions::NONE,
    },
    PermissionInfo {
        bit: 12,
        constant: "DIAGNOSTICS_VIEW",
        key: "diagnosticsView",
        area: PermissionArea::Diagnostics,
        // Command lines, full paths and what FXServer's memory held when it crashed.
        danger: Danger::Sensitive,
        implies: Permissions::NONE,
    },
    PermissionInfo {
        bit: 13,
        constant: "FILES_MANAGE",
        key: "filesManage",
        area: PermissionArea::Files,
        // Gives other accounts reach into the server's files; changing them can run code on the host.
        danger: Danger::Critical,
        implies: Permissions::NONE,
    },
    PermissionInfo {
        bit: 14,
        constant: "PLAYERS_VIEW",
        key: "playersView",
        area: PermissionArea::Players,
        // Players' names, licences and play time, and the texts resources wrote when they
        // dropped someone.
        danger: Danger::Sensitive,
        implies: Permissions::NONE,
    },
    PermissionInfo {
        bit: 15,
        constant: "PLAYER_IDENTIFIERS_VIEW",
        key: "playerIdentifiersView",
        area: PermissionArea::Players,
        // Every other identifier, hardware tokens, and which accounts are linked.
        danger: Danger::Sensitive,
        implies: Permissions::PLAYERS_VIEW,
    },
    PermissionInfo {
        bit: 16,
        constant: "CHANGES_VIEW",
        key: "changesView",
        area: PermissionArea::Changes,
        // Paths, versions and attributions say who changed what on the server.
        danger: Danger::Sensitive,
        implies: Permissions::NONE,
    },
    PermissionInfo {
        bit: 17,
        constant: "CHANGES_CONTENT",
        key: "changesContent",
        area: PermissionArea::Changes,
        // The server's code: custom scripts and the plain parts of paid ones.
        danger: Danger::Critical,
        implies: Permissions::CHANGES_VIEW,
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_bits_are_frozen() {
        // Append a new permission at the end, with the next bit. Never edit a line here.
        let frozen = [
            (0, "DASHBOARD_VIEW", Permissions::DASHBOARD_VIEW),
            (1, "CONSOLE_VIEW", Permissions::CONSOLE_VIEW),
            (2, "CONSOLE_WRITE", Permissions::CONSOLE_WRITE),
            (3, "SERVER_CONTROL", Permissions::SERVER_CONTROL),
            (4, "SCHEDULE_MANAGE", Permissions::SCHEDULE_MANAGE),
            (5, "ARTIFACTS_VIEW", Permissions::ARTIFACTS_VIEW),
            (6, "ARTIFACTS_MANAGE", Permissions::ARTIFACTS_MANAGE),
            (7, "USERS_MANAGE", Permissions::USERS_MANAGE),
            (8, "AUDIT_VIEW", Permissions::AUDIT_VIEW),
            (9, "SETTINGS_VIEW", Permissions::SETTINGS_VIEW),
            (10, "SETTINGS_MANAGE", Permissions::SETTINGS_MANAGE),
            (11, "LOGS_DOWNLOAD", Permissions::LOGS_DOWNLOAD),
            (12, "DIAGNOSTICS_VIEW", Permissions::DIAGNOSTICS_VIEW),
            (13, "FILES_MANAGE", Permissions::FILES_MANAGE),
            (14, "PLAYERS_VIEW", Permissions::PLAYERS_VIEW),
            (
                15,
                "PLAYER_IDENTIFIERS_VIEW",
                Permissions::PLAYER_IDENTIFIERS_VIEW,
            ),
            (16, "CHANGES_VIEW", Permissions::CHANGES_VIEW),
            (17, "CHANGES_CONTENT", Permissions::CHANGES_CONTENT),
        ];

        assert_eq!(CATALOG.len(), frozen.len());
        for ((bit, constant, permission), info) in frozen.into_iter().zip(CATALOG) {
            assert_eq!(info.bit, bit, "{constant}");
            assert_eq!(info.constant, constant);
            assert_eq!(permission.bits(), 1 << bit, "{constant}");
            assert_eq!(info.permission(), permission, "{constant}");
        }
    }

    #[test]
    fn catalog_keys_are_unique() {
        for (index, info) in CATALOG.iter().enumerate() {
            assert!(
                CATALOG[index + 1..].iter().all(|other| other.key != info.key),
                "{} appears twice",
                info.key
            );
        }
    }

    #[test]
    fn closing_a_set_adds_what_it_implies_and_drops_unknown_bits() {
        let stray = Permissions(1 << 100);

        assert_eq!(
            Permissions::CONSOLE_WRITE.closed(),
            Permissions::CONSOLE_WRITE | Permissions::CONSOLE_VIEW
        );
        assert_eq!(
            (Permissions::SETTINGS_MANAGE | Permissions::DASHBOARD_VIEW | stray).closed(),
            Permissions::SETTINGS_MANAGE | Permissions::SETTINGS_VIEW | Permissions::DASHBOARD_VIEW
        );
        assert_eq!(Permissions::CONSOLE_VIEW.closed(), Permissions::CONSOLE_VIEW);
        assert_eq!(stray.unknown(), stray);
        assert!(Permissions::all().unknown().is_empty());
    }

    #[test]
    fn blobs_are_big_endian_and_minimal() {
        let set = Permissions::USERS_MANAGE | Permissions::AUDIT_VIEW | Permissions::CONSOLE_VIEW;

        assert_eq!(set.to_blob(), vec![0x01, 0x82]);
        assert_eq!(Permissions::DASHBOARD_VIEW.to_blob(), vec![0x01]);
        assert_eq!(Permissions::NONE.to_blob(), Vec::<u8>::new());
        assert_eq!(Permissions::from_blob(&set.to_blob()), set);
        assert_eq!(Permissions::from_blob(&[]), Permissions::NONE);
    }

    #[test]
    fn a_stored_blob_is_masked_and_closed_on_read() {
        // Bit 100 is unknown; the 20-byte blob is longer than any set this build writes.
        let mut blob = vec![0u8; 20];
        blob[7] = 0x10;
        blob[19] = 0x04;

        assert_eq!(
            Permissions::from_blob(&blob),
            Permissions::CONSOLE_WRITE | Permissions::CONSOLE_VIEW
        );
    }

    #[test]
    fn hex_round_trips_and_rejects_anything_else() {
        let set = Permissions::LOGS_DOWNLOAD | Permissions::CONSOLE_WRITE | Permissions::CONSOLE_VIEW;

        assert_eq!(set.to_hex(), "806");
        assert_eq!(Permissions::NONE.to_hex(), "0");
        assert_eq!(Permissions::from_hex("806"), Ok(set));
        assert_eq!(Permissions::from_hex("0"), Ok(Permissions::NONE));
        for invalid in ["", "0x4c", "+4c", "4g", " 4c", &"f".repeat(33)] {
            assert_eq!(
                Permissions::from_hex(invalid),
                Err(InvalidPermissions),
                "{invalid:?}"
            );
        }
    }

    #[test]
    fn the_wire_form_is_a_lowercase_hex_string() {
        let set = Permissions::USERS_MANAGE | Permissions::AUDIT_VIEW;

        let json = serde_json::to_string(&set).unwrap();

        assert_eq!(json, "\"180\"");
        assert_eq!(serde_json::from_str::<Permissions>(&json).unwrap(), set);
        assert!(serde_json::from_str::<Permissions>("384").is_err());
    }
}

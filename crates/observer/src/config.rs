use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use anyhow::Context;
use observer_domain::features::Feature;
use observer_web::TrustedProxy;
use serde::Deserialize;

/// The bootstrap configuration, read from `observer.toml` before anything else starts. Everything
/// that can change at run time lives in the database instead.
#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Where this configuration was read from, or would have been.
    #[serde(skip)]
    pub path: PathBuf,
    /// Relative paths are resolved against the configuration file's directory, so a service
    /// started from anywhere finds the same data.
    pub data_dir: PathBuf,
    pub http: HttpConfig,
    /// Where SFTP listens, for a host that keeps the address out of the panel's reach. Without
    /// this section the owner gives it in the panel.
    pub sftp: Option<SftpConfig>,
    /// The optional features the host turns off, such as `sftp = "off"`: the panel then cannot
    /// turn them on, even from the owner's account.
    #[expect(
        clippy::zero_sized_map_values,
        reason = "a feature takes one value here, which only says it is named on purpose"
    )]
    pub features: BTreeMap<Feature, HostSwitch>,
    pub log: LogConfig,
}

/// What `observer.toml` can say of a feature. Turning one on is the panel's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HostSwitch {
    Off,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SftpConfig {
    /// Such as `0.0.0.0:40126`.
    pub listen: SocketAddr,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HttpConfig {
    pub listen: SocketAddr,
    /// The names the panel is reached by, such as `panel.example.com`. Addresses and `localhost`
    /// always work; the API refuses any other name, which a stranger's page could have pointed at
    /// this host.
    pub hosts: Vec<String>,
    /// The reverse proxies whose `X-Forwarded-For` and `X-Forwarded-Proto` are believed: addresses
    /// and CIDR blocks such as `10.0.0.0/8`. From any other peer those headers are ignored.
    pub trusted_proxies: Vec<TrustedProxy>,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LogConfig {
    /// A `tracing` filter, such as `info` or `info,observer_web=debug`. `OBSERVER_LOG` overrides it.
    pub level: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            path: PathBuf::from("observer.toml"),
            data_dir: PathBuf::from("data"),
            http: HttpConfig::default(),
            sftp: None,
            features: BTreeMap::new(),
            log: LogConfig::default(),
        }
    }
}

impl Default for HttpConfig {
    fn default() -> Self {
        // 40120 and 40122 are txAdmin's; staying clear of them lets both run on one host.
        Self {
            listen: SocketAddr::from(([0, 0, 0, 0], 40125)),
            hosts: Vec::new(),
            trusted_proxies: Vec::new(),
        }
    }
}

impl HttpConfig {
    /// Whether other hosts reach the panel over plain HTTP: it listens past loopback, and no proxy
    /// is trusted to bring HTTPS in front of it.
    pub fn exposed_over_plain_http(&self) -> bool {
        !self.listen.ip().is_loopback() && self.trusted_proxies.is_empty()
    }
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            level: "info".to_owned(),
        }
    }
}

/// Where Observer keeps its files, all under the data directory.
#[derive(Debug, Clone)]
pub struct DataPaths {
    pub root: PathBuf,
    pub database: PathBuf,
    /// The statistics, apart from the state: never backed up.
    pub metrics: PathBuf,
    /// What each start of FXServer ran from, apart from the state: never backed up either.
    pub changes: PathBuf,
    pub backups: PathBuf,
    pub logs: PathBuf,
    pub artifacts: PathBuf,
}

impl DataPaths {
    pub fn new(root: PathBuf) -> Self {
        Self {
            database: root.join("observer.sqlite3"),
            metrics: root.join("metrics.sqlite3"),
            changes: root.join("changes.sqlite3"),
            backups: root.join("backups"),
            logs: root.join("logs"),
            artifacts: root.join("artifacts"),
            root,
        }
    }
}

impl Config {
    /// Reads the file if it exists; a missing file means the defaults.
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let mut config: Self = match fs::read_to_string(path) {
            Ok(text) => toml::from_str(&text).with_context(|| format!("invalid {}", path.display()))?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => Self::default(),
            Err(error) => return Err(error).with_context(|| format!("cannot read {}", path.display())),
        };

        config.path = path.to_path_buf();
        if config.data_dir.is_relative() {
            let base = path.parent().unwrap_or_else(|| Path::new(""));
            config.data_dir = base.join(&config.data_dir);
        }
        Ok(config)
    }

    pub fn paths(&self) -> DataPaths {
        DataPaths::new(self.data_dir.clone())
    }

    pub fn host_off(&self) -> BTreeSet<Feature> {
        self.features.keys().copied().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_file_gives_the_defaults_next_to_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("observer.toml");

        let config = Config::load(&path).unwrap();

        assert_eq!(config.data_dir, dir.path().join("data"));
        assert_eq!(config.http.listen.port(), 40125);
        assert_eq!(config.http.trusted_proxies, Vec::<TrustedProxy>::new());
        assert_eq!(config.log.level, "info");
    }

    #[test]
    fn values_from_the_file_override_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("observer.toml");
        fs::write(
            &path,
            "data_dir = \"state\"\n[http]\nlisten = \"127.0.0.1:9000\"\nhosts = [\"panel.example.com\"]\n\
             trusted_proxies = [\"127.0.0.1\", \"10.0.0.0/8\", \"fd00::/8\"]\n",
        )
        .unwrap();

        let config = Config::load(&path).unwrap();

        assert_eq!(config.data_dir, dir.path().join("state"));
        assert_eq!(config.http.listen, SocketAddr::from(([127, 0, 0, 1], 9000)));
        assert_eq!(config.http.hosts, ["panel.example.com"]);
        let proxies: Vec<TrustedProxy> = ["127.0.0.1", "10.0.0.0/8", "fd00::/8"]
            .iter()
            .map(|proxy| proxy.parse().unwrap())
            .collect();
        assert_eq!(config.http.trusted_proxies, proxies);
        assert_eq!(config.log.level, "info");
    }

    #[test]
    fn the_sftp_section_is_absent_by_default_and_names_where_to_listen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("observer.toml");
        assert!(Config::load(&path).unwrap().sftp.is_none());

        fs::write(
            &path,
            "[sftp]
listen = \"0.0.0.0:40126\"
",
        )
        .unwrap();
        let sftp = Config::load(&path).unwrap().sftp.unwrap();
        assert_eq!(sftp.listen.port(), 40126);

        fs::write(
            &path, "[sftp]
",
        )
        .unwrap();
        assert!(
            Config::load(&path).is_err(),
            "a section without listen is a mistake, not SFTP off"
        );
    }

    #[test]
    fn the_host_turns_features_off_by_name_and_only_off() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("observer.toml");
        assert_eq!(Config::load(&path).unwrap().host_off(), BTreeSet::new());

        fs::write(
            &path,
            "[features]
sftp = \"off\"
diagnostics = \"off\"
",
        )
        .unwrap();
        assert_eq!(
            Config::load(&path).unwrap().host_off(),
            BTreeSet::from([Feature::Sftp, Feature::Diagnostics])
        );

        fs::write(
            &path,
            "[features]
sftp = \"on\"
",
        )
        .unwrap();
        assert!(Config::load(&path).is_err(), "the panel turns features on");
        fs::write(
            &path,
            "[features]
sfpt = \"off\"
",
        )
        .unwrap();
        assert!(Config::load(&path).is_err(), "a misspelt feature would stay on");
    }

    #[test]
    fn an_unknown_key_is_an_error_rather_than_a_silent_typo() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("observer.toml");
        fs::write(&path, "[http]\nlisten_addr = \"127.0.0.1:9000\"\n").unwrap();

        assert!(Config::load(&path).is_err());
    }

    #[test]
    fn a_trusted_proxy_that_is_not_an_address_or_a_block_stops_the_start_and_is_named() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("observer.toml");
        fs::write(
            &path,
            "[http]\ntrusted_proxies = [\"127.0.0.1\", \"10.0.0.0/33\"]\n",
        )
        .unwrap();

        let error = format!("{:#}", Config::load(&path).unwrap_err());

        assert!(
            error.contains("`10.0.0.0/33` is not an address or a CIDR block"),
            "{error}"
        );
    }

    #[test]
    fn only_a_panel_reached_past_loopback_without_a_trusted_proxy_is_exposed_over_plain_http() {
        let http = |listen: &str, proxies: &[&str]| HttpConfig {
            listen: listen.parse().unwrap(),
            hosts: Vec::new(),
            trusted_proxies: proxies.iter().map(|proxy| proxy.parse().unwrap()).collect(),
        };

        assert!(http("0.0.0.0:40125", &[]).exposed_over_plain_http());
        assert!(http("192.168.1.20:40125", &[]).exposed_over_plain_http());
        assert!(http("[::]:40125", &[]).exposed_over_plain_http());
        assert!(!http("127.0.0.1:40125", &[]).exposed_over_plain_http());
        assert!(!http("[::1]:40125", &[]).exposed_over_plain_http());
        assert!(!http("0.0.0.0:40125", &["172.18.0.0/16"]).exposed_over_plain_http());
    }
}

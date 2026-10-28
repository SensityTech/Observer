use std::fs;
use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::Deserialize;

/// The bootstrap configuration, read from `observer.toml` before anything else starts. Everything
/// that can change at run time lives in the database instead.
#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Relative paths are resolved against the configuration file's directory, so a service
    /// started from anywhere finds the same data.
    pub data_dir: PathBuf,
    pub http: HttpConfig,
    pub log: LogConfig,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HttpConfig {
    pub listen: SocketAddr,
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
            data_dir: PathBuf::from("data"),
            http: HttpConfig::default(),
            log: LogConfig::default(),
        }
    }
}

impl Default for HttpConfig {
    fn default() -> Self {
        // 40120 and 40122 are txAdmin's; staying clear of them lets both run on one host.
        Self {
            listen: SocketAddr::from(([0, 0, 0, 0], 40125)),
        }
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
    pub backups: PathBuf,
    pub logs: PathBuf,
}

impl DataPaths {
    pub fn new(root: PathBuf) -> Self {
        Self {
            database: root.join("observer.sqlite3"),
            backups: root.join("backups"),
            logs: root.join("logs"),
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

        if config.data_dir.is_relative() {
            let base = path.parent().unwrap_or_else(|| Path::new(""));
            config.data_dir = base.join(&config.data_dir);
        }
        Ok(config)
    }

    pub fn paths(&self) -> DataPaths {
        DataPaths::new(self.data_dir.clone())
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
        assert_eq!(config.log.level, "info");
    }

    #[test]
    fn values_from_the_file_override_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("observer.toml");
        fs::write(
            &path,
            "data_dir = \"state\"\n[http]\nlisten = \"127.0.0.1:9000\"\n",
        )
        .unwrap();

        let config = Config::load(&path).unwrap();

        assert_eq!(config.data_dir, dir.path().join("state"));
        assert_eq!(config.http.listen, SocketAddr::from(([127, 0, 0, 1], 9000)));
        assert_eq!(config.log.level, "info");
    }

    #[test]
    fn an_unknown_key_is_an_error_rather_than_a_silent_typo() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("observer.toml");
        fs::write(&path, "[http]\nlisten_addr = \"127.0.0.1:9000\"\n").unwrap();

        assert!(Config::load(&path).is_err());
    }
}

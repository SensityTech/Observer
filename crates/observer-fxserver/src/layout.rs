//! Where things are inside an FXServer build, which depends on how the build is packaged.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// How a build is packaged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flavour {
    /// `server.zip`: `FXServer.exe` at the root.
    Windows,
    /// `fx.tar.xz`: an Alpine tree run through its own musl loader.
    Linux,
}

impl Flavour {
    pub const HOST: Self = if cfg!(windows) { Self::Windows } else { Self::Linux };
}

/// Paths under the citizen root that FXServer and Observer write while a build runs, which a
/// verification of the build must skip.
pub const RUNTIME_PATHS: [&str; 4] = [
    "crashes",
    "citizen/system_resources/observer",
    "server-tls.crt",
    "server-tls.key",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layout {
    flavour: Flavour,
    build: PathBuf,
}

impl Layout {
    pub fn new(build: impl Into<PathBuf>, flavour: Flavour) -> Self {
        Self {
            flavour,
            build: build.into(),
        }
    }

    pub fn flavour(&self) -> Flavour {
        self.flavour
    }

    /// The folder FXServer calls `citizen_root`: it holds `citizen/` and gets `crashes/`.
    pub fn citizen_root(&self) -> PathBuf {
        match self.flavour {
            Flavour::Windows => self.build.clone(),
            Flavour::Linux => self.build.join("alpine/opt/cfx-server"),
        }
    }

    pub fn system_resources(&self) -> PathBuf {
        self.citizen_root().join("citizen").join("system_resources")
    }

    pub fn crashes(&self) -> PathBuf {
        self.citizen_root().join("crashes")
    }

    /// The program to start and the arguments that come before FXServer's own.
    pub(crate) fn program(&self) -> (PathBuf, Vec<OsString>) {
        match self.flavour {
            Flavour::Windows => (self.build.join("FXServer.exe"), Vec::new()),
            Flavour::Linux => {
                // The loader runs FXServer inside its own process, so the pid is FXServer's.
                // `run.sh` does the same but splits arguments that hold blanks.
                let alpine = self.build.join("alpine");
                let root = self.citizen_root();
                let mut libraries = OsString::new();
                for (index, dir) in ["usr/lib/v8/", "lib/", "usr/lib/"].into_iter().enumerate() {
                    if index > 0 {
                        libraries.push(":");
                    }
                    libraries.push(alpine.join(dir));
                }
                let mut citizen = root.join("citizen").into_os_string();
                citizen.push("/");
                (
                    root.join("ld-musl-x86_64.so.1"),
                    vec![
                        "--library-path".into(),
                        libraries,
                        "--".into(),
                        root.join("FXServer").into(),
                        "+set".into(),
                        "citizen_dir".into(),
                        citizen,
                    ],
                )
            }
        }
    }

    pub fn build(&self) -> &Path {
        &self.build
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_windows_build_runs_fxserver_from_its_root() {
        let layout = Layout::new("builds/35245", Flavour::Windows);

        assert_eq!(
            layout.program(),
            (Path::new("builds/35245/FXServer.exe").into(), Vec::new())
        );
        assert_eq!(layout.citizen_root(), Path::new("builds/35245"));
        assert_eq!(
            layout.system_resources(),
            Path::new("builds/35245/citizen/system_resources")
        );
        assert_eq!(layout.crashes(), Path::new("builds/35245/crashes"));
    }

    #[cfg(unix)]
    #[test]
    fn a_linux_build_runs_fxserver_through_its_musl_loader() {
        let layout = Layout::new("/srv/fx", Flavour::Linux);

        let (program, args) = layout.program();

        assert_eq!(
            program,
            Path::new("/srv/fx/alpine/opt/cfx-server/ld-musl-x86_64.so.1")
        );
        assert_eq!(
            args,
            [
                "--library-path",
                "/srv/fx/alpine/usr/lib/v8/:/srv/fx/alpine/lib/:/srv/fx/alpine/usr/lib/",
                "--",
                "/srv/fx/alpine/opt/cfx-server/FXServer",
                "+set",
                "citizen_dir",
                "/srv/fx/alpine/opt/cfx-server/citizen/",
            ]
        );
        assert_eq!(
            layout.crashes(),
            Path::new("/srv/fx/alpine/opt/cfx-server/crashes")
        );
        assert_eq!(
            layout.system_resources(),
            Path::new("/srv/fx/alpine/opt/cfx-server/citizen/system_resources")
        );
    }
}

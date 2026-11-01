//! Observer's own folders and files, should a server folder hold them: its data folder, the folder
//! of its program and its configuration file. Neither SFTP sessions nor the server tools ever reach
//! them, wherever the server folder is, so both take the one list this module makes.

use std::path::{MAIN_SEPARATOR, Path, PathBuf};

/// Observer's own folders and files, as full paths.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OwnPaths(Vec<PathBuf>);

impl OwnPaths {
    /// The data folder `data_dir`, the configuration file `config`, and the folder of the program
    /// running now.
    pub fn new(data_dir: &Path, config: &Path) -> Self {
        let program = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(Path::to_path_buf));
        Self(
            [Some(data_dir.to_path_buf()), program, Some(config.to_path_buf())]
                .into_iter()
                .flatten()
                .collect(),
        )
    }

    /// These paths alone, for a test or a command that knows them.
    pub fn of(paths: Vec<PathBuf>) -> Self {
        Self(paths)
    }

    pub fn paths(&self) -> &[PathBuf] {
        &self.0
    }

    /// Those inside the folder `root`, as components below it: the list
    /// [`built_in_refusal`](observer_domain::files::built_in_refusal) takes.
    pub fn inside(&self, root: &Path) -> Vec<Vec<String>> {
        paths_inside(root, &self.0)
    }
}

/// The paths of `refused` that lie inside `root`, as components below it. Both sides are resolved
/// first, so a link or another spelling of the same folder is caught; on Windows the comparison
/// ignores case, as the file system does. A path that does not exist is never inside.
pub fn paths_inside(root: &Path, refused: &[PathBuf]) -> Vec<Vec<String>> {
    let Ok(root) = std::fs::canonicalize(root) else {
        return Vec::new();
    };
    let key = |path: &Path| {
        let text = path.to_string_lossy().into_owned();
        if cfg!(windows) { text.to_lowercase() } else { text }
    };
    let root_key = key(&root);
    refused
        .iter()
        .filter_map(|path| std::fs::canonicalize(path).ok())
        .filter_map(|path| {
            let path_key = key(&path);
            let rest = path_key.strip_prefix(&root_key)?;
            if !rest.starts_with(MAIN_SEPARATOR) {
                return None;
            }
            let components = path.strip_prefix(&root).ok().map_or_else(
                // Another case of the same root: split the lowercased rest instead.
                || {
                    rest.split(MAIN_SEPARATOR)
                        .filter(|part| !part.is_empty())
                        .map(str::to_owned)
                        .collect()
                },
                |relative| {
                    relative
                        .components()
                        .map(|part| part.as_os_str().to_string_lossy().into_owned())
                        .collect()
                },
            );
            Some(components)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    #[test]
    fn only_the_paths_inside_the_folder_are_listed_as_components_below_it() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("server-data");
        let data = root.join("resources").join("observer-data");
        let elsewhere = dir.path().join("elsewhere");
        let config = root.join("observer.toml");
        fs::create_dir_all(&data).unwrap();
        fs::create_dir_all(&elsewhere).unwrap();
        fs::write(&config, "").unwrap();
        let own = OwnPaths::of(vec![
            data,
            elsewhere,
            config,
            dir.path().join("server-data-2"),
            root.join("missing"),
        ]);

        assert_eq!(
            own.inside(&root),
            [
                vec!["resources".to_owned(), "observer-data".to_owned()],
                vec!["observer.toml".to_owned()]
            ]
        );
        assert_eq!(own.inside(&dir.path().join("missing")), Vec::<Vec<String>>::new());
    }

    #[test]
    fn observer_s_own_paths_hold_its_data_folder_program_folder_and_configuration() {
        let own = OwnPaths::new(
            Path::new("/srv/observer/data"),
            Path::new("/srv/observer/observer.toml"),
        );

        assert_eq!(own.paths().len(), 3);
        assert_eq!(own.paths()[0], Path::new("/srv/observer/data"));
        assert_eq!(own.paths()[2], Path::new("/srv/observer/observer.toml"));
    }
}

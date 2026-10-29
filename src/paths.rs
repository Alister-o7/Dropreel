//! Locations of Dropreel's data on disk (`%APPDATA%\Dropreel` on Windows).

use std::path::{Path, PathBuf};

use anyhow::Context as _;

#[derive(Clone, Debug)]
pub struct AppPaths {
    pub root: PathBuf,
    pub db: PathBuf,
    pub settings: PathBuf,
    pub cache: PathBuf,
    pub posters: PathBuf,
    pub logs: PathBuf,
}

impl AppPaths {
    /// Resolves the data directory. `DROPREEL_HOME` overrides it (useful for testing).
    pub fn resolve() -> anyhow::Result<Self> {
        let root = match std::env::var_os("DROPREEL_HOME") {
            Some(dir) => PathBuf::from(dir),
            None => {
                let config = dirs::config_dir().context("could not find the AppData folder")?;
                // The app was "DropReel" before 0.3.5.3. Windows ignores case, so it's the same
                // folder there; elsewhere an existing one is kept.
                let old = config.join("DropReel");
                let new = config.join("Dropreel");
                if !new.exists() && old.exists() {
                    old
                } else {
                    new
                }
            }
        };
        Self::at(&root)
    }

    /// Uses `root` as the data directory, creating the folder layout if needed.
    pub fn at(root: &Path) -> anyhow::Result<Self> {
        let cache = root.join("cache");
        let paths = Self {
            root: root.to_path_buf(),
            db: root.join("library.db"),
            settings: root.join("settings.toml"),
            posters: cache.join("posters"),
            cache,
            logs: root.join("logs"),
        };
        for dir in [&paths.root, &paths.cache, &paths.posters, &paths.logs] {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("could not create {}", dir.display()))?;
        }
        Ok(paths)
    }
}

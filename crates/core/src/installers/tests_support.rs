//! Test-only filesystem helpers shared by the installers halves.
//!
//! [`Scratch`], [`touch`], [`wine`] and [`install_steam`] were defined once in
//! the old `installers.rs` test module and used by every half's tests, so they
//! move here rather than being duplicated three times. `#[cfg(test)]`-gated
//! like the module that declares it, so no production build sees it.

use std::path::{Path, PathBuf};

/// A scratch directory that cleans itself up.
pub struct Scratch(PathBuf);

impl Scratch {
    pub fn new(label: &str) -> Self {
        let path =
            std::env::temp_dir().join(format!("gh-installers-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Write a file, creating parents as needed.
pub fn touch(path: &Path) -> PathBuf {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, "stub").unwrap();
    path.to_path_buf()
}

pub fn wine() -> crate::runners::WineRunner {
    crate::runners::WineRunner::with_binary(Some(PathBuf::from("/usr/bin/wine")))
}

/// Install `steam.exe` under `drive_c` and return it.
pub fn install_steam(drive_c: &Path) -> PathBuf {
    touch(&drive_c.join("Program Files (x86)/Steam/steam.exe"))
}

//! `which_in`'s current-directory rule, in the one test process that can enter
//! a directory.
//!
//! `PATH=":"` has an empty entry, and CPython does **not** skip it: its comment
//! says so explicitly, and `os.path.join("", name)` is a relative path, so the
//! entry names the current directory. The port reproduces that, and until this
//! file existed nothing proved it did — the unit test that claimed to check it
//! was named `which_checks_an_empty_path_entry_against_the_current_directory`
//! and never left the working directory, so it asserted about the *default*
//! path while its name promised the opposite.
//!
//! It cannot be fixed in place. A unit test shares its process with hundreds of
//! others running in parallel, and `std::env::set_current_dir` is process-wide:
//! changing the cwd there would move every concurrently running test's relative
//! paths under it. An integration test is a separate binary with its own
//! process, which is the only place this can be measured honestly.
//!
//! Measured against CPython 3.14.7 on this host before writing the assertions:
//! with an executable `sh` in the working directory, `shutil.which("sh",
//! path=":")` returns `'sh'`.

use std::fs;
use std::path::PathBuf;

use gamehandler_core::paths::Env;
use gamehandler_core::runners::env::which_in;

/// A `PATH` and nothing else — the whole of what `which_in` reads.
struct PathOnly(Option<String>);

impl Env for PathOnly {
    fn var(&self, name: &str) -> Option<String> {
        (name == "PATH").then(|| self.0.clone()).flatten()
    }
}

/// A scratch directory that is removed however the test ends.
struct Scratch(PathBuf);

impl Scratch {
    fn new(label: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("gh-which-cwd-{label}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        Self(root)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn make_executable(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    fs::write(path, "#!/bin/sh\n").unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

/// **An empty `PATH` entry is the current directory, not a skipped entry.**
///
/// Both directions are asserted, because `PATH=":"` with the probe present and
/// `PATH="/nonexistent"` with the probe present look identical to an
/// implementation that searches the cwd unconditionally, and the difference
/// between them is the whole rule.
#[test]
fn an_empty_path_entry_searches_the_current_directory() {
    let scratch = Scratch::new("colon");
    let probe = scratch.0.join("gh-cwd-probe");
    make_executable(&probe);

    // The process's cwd is this test binary's, and the probe is not in it —
    // `which_in` must therefore miss before the chdir below.
    let colon = PathOnly(Some(":".to_string()));
    assert_eq!(
        which_in("gh-cwd-probe", &colon),
        None,
        "the probe is not on the default path, so this must miss before the chdir"
    );

    let previous = std::env::current_dir().unwrap();
    std::env::set_current_dir(&scratch.0).unwrap();

    // `PATH=":"` — one empty entry — must find it.
    assert_eq!(
        which_in("gh-cwd-probe", &colon),
        Some(PathBuf::from("gh-cwd-probe")),
        "`PATH=\":\"` names the current directory; skipping the empty entry \
         loses a binary CPython finds"
    );

    // A colon-separated list of real directories must not, with the probe
    // sitting in the cwd and nowhere else: this is the same call with the one
    // character that matters changed, so it separates "searches the cwd" from
    // "searches the cwd whenever it feels like it".
    let dotted = PathOnly(Some("/nonexistent-a:/nonexistent-b".to_string()));
    assert_eq!(
        which_in("gh-cwd-probe", &dotted),
        None,
        "only the empty entry means the cwd; an absolute entry that is not the \
         cwd must not find a cwd-relative binary"
    );

    // And the same empty entry no longer finds it once the cwd has moved away,
    // so the assertion above is about the working directory being searched
    // rather than about the probe having been found by some other route.
    std::env::set_current_dir(&previous).unwrap();
    assert_eq!(
        which_in("gh-cwd-probe", &colon),
        None,
        "moving out of the scratch directory must make the empty entry miss again"
    );
}

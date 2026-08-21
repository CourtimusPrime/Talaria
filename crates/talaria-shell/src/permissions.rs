//! Owner-only permissions for the plaintext stores this browser keeps on
//! disk: [`crate::history`], [`crate::bookmarks`], [`crate::settings`] and
//! [`crate::downloads`].
//!
//! Each of those modules stores its contents unencrypted, and two of them say
//! in their own headers that this is safe because the surrounding file
//! permissions already restrict who can read them. That sentence is only true
//! if something sets those permissions, and `fs::write` and
//! `OpenOptions::create` do not — both land at `0666 & !umask`, which is
//! `0644` on an ordinary machine, and `create_dir_all` leaves the config
//! directory at `0755`. On a shared machine that makes the complete record of
//! every URL the human visited, query strings included, readable by every
//! local account.
//!
//! One module rather than a copy per store, deliberately, and against the
//! grain of the `config_dir()` duplication beside it: four copies of a
//! security control is four chances to forget the fifth.
//!
//! [`crate::vault`] keeps its own `restrict_to_owner` and does not call in
//! here. The difference is the log level — a plaintext *password* file that
//! could not be locked down is an `error!`, where a browsing history is a
//! `warn!` — and collapsing the two would quieten the louder one.
//!
//! Every function here degrades rather than aborting, per the crate's rule
//! for user-data problems: a permission that cannot be set is logged and the
//! write still happens, because a readable history is better than no history
//! and much better than a browser that will not start.

use std::fs;
use std::io;
use std::path::Path;

/// Owner read/write, nothing for anyone else.
#[cfg(unix)]
const FILE_MODE: u32 = 0o600;

/// Owner read/write/traverse, nothing for anyone else. A directory needs the
/// execute bit to be entered at all, so this is `0700` where a file is
/// `0600`.
#[cfg(unix)]
const DIR_MODE: u32 = 0o700;

/// Create `path` and every missing parent, then make `path` itself
/// owner-only. Idempotent, and it tightens a directory an older build already
/// left at `0755`.
///
/// Call this on a directory this browser owns — each store's
/// `config_dir()` — and never on whatever a write path happens to find in
/// `Path::parent`, which in a unit test is the shared system temp directory
/// and under the `config_dir()` fallback is the process's working directory.
/// The one this browser owns is the one worth tightening: the listing is
/// itself a disclosure, since `history.jsonl` and `downloads.json` name what
/// they are.
pub fn create_dir_owner_only(path: &Path) {
    if let Err(error) = fs::create_dir_all(path) {
        log::warn!("could not create {}: {error}", path.display());
        return;
    }
    restrict_dir(path);
}

/// Write `data` to `path`, owner-only, replacing whatever was there.
///
/// The [`fs::write`] this replaces creates world-readable and offers no way
/// to say otherwise, so the mode is set twice here: at `open` time, which is
/// the only way to close the window between a file existing and being
/// restricted, and again afterwards, because `mode` is ignored for a file
/// that already existed — which is exactly the case for a stale `.tmp`
/// sibling left by an interrupted save.
///
/// Callers that stage through a `.tmp` and [`fs::rename`] into place should
/// call this on the *staged* file. A rename replaces the destination inode
/// with the staged one, so the staged file's mode is the mode that lands;
/// restricting the destination beforehand would achieve nothing.
pub fn write_owner_only(path: &Path, data: &[u8]) -> io::Result<()> {
    use io::Write;

    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(FILE_MODE);
    }
    let mut file = options.open(path)?;
    file.write_all(data)?;
    file.flush()?;
    restrict_file(path);
    Ok(())
}

/// Open `path` for appending, owner-only, creating it if it is not there.
///
/// The append log [`crate::history`] keeps is the one store that is never
/// rewritten in the ordinary case, so its file is only ever created once and
/// [`write_owner_only`]'s mode would never reach an existing one. Pair this
/// with a [`restrict_file`] at load time to bring a log written by an older
/// build down to the same mode.
pub fn append_owner_only(path: &Path) -> io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.append(true).create(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(FILE_MODE);
    }
    options.open(path)
}

/// Make an existing file owner-only.
pub fn restrict_file(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(error) = fs::set_permissions(path, fs::Permissions::from_mode(FILE_MODE)) {
            log::warn!("could not make {} owner-only: {error}", path.display());
        }
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
}

/// Make an existing directory owner-only.
pub fn restrict_dir(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(error) = fs::set_permissions(path, fs::Permissions::from_mode(DIR_MODE)) {
            log::warn!("could not make {} owner-only: {error}", path.display());
        }
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
}

/// A path's permission bits, for the tests that assert a store landed at
/// `0600`. The `& 0o777` drops the file-type bits `st_mode` also carries.
#[cfg(all(unix, test))]
pub fn mode_of(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path).expect("the file under test exists").permissions().mode() & 0o777
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    /// A path in the temp directory that removes itself. `tempfile` is not a
    /// dependency of this crate and this fix may not add one, so the unique
    /// name is built the way every store module's tests build theirs.
    struct TempPath(std::path::PathBuf);

    impl TempPath {
        fn new(extension: &str) -> Self {
            let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
            let name =
                format!("talaria-permissions-{}-{unique}.{extension}", std::process::id());
            Self(std::env::temp_dir().join(name))
        }
    }

    impl Drop for TempPath {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[cfg(unix)]
    #[test]
    fn a_written_file_lands_owner_only() {
        let temp = TempPath::new("json");
        write_owner_only(&temp.0, b"{}").expect("write the file under test");
        assert_eq!(mode_of(&temp.0), 0o600);
    }

    /// The stale-`.tmp` case: `OpenOptions::mode` is ignored for a file that
    /// already exists, so without the second `set_permissions` a staging file
    /// left world-readable by an older build would stay that way and then be
    /// renamed into place.
    #[cfg(unix)]
    #[test]
    fn a_pre_existing_world_readable_file_is_brought_down() {
        use std::os::unix::fs::PermissionsExt;

        let temp = TempPath::new("json");
        fs::write(&temp.0, b"stale").expect("write the test fixture");
        fs::set_permissions(&temp.0, fs::Permissions::from_mode(0o644))
            .expect("loosen the test fixture");
        assert_eq!(mode_of(&temp.0), 0o644, "the fixture must start loose");

        write_owner_only(&temp.0, b"{}").expect("write the file under test");
        assert_eq!(mode_of(&temp.0), 0o600);
        assert_eq!(fs::read(&temp.0).expect("read it back"), b"{}");
    }

    #[cfg(unix)]
    #[test]
    fn an_appended_file_lands_owner_only() {
        let temp = TempPath::new("jsonl");
        {
            use std::io::Write;
            let mut file = append_owner_only(&temp.0).expect("open the file under test");
            file.write_all(b"one\n").expect("append to it");
        }
        assert_eq!(mode_of(&temp.0), 0o600);
    }

    #[cfg(unix)]
    #[test]
    fn a_created_directory_lands_owner_only() {
        let temp = TempPath::new("dir");
        create_dir_owner_only(&temp.0);
        assert_eq!(mode_of(&temp.0), 0o700);
    }
}

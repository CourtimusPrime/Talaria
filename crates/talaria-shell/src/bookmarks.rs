//! Bookmarks: the small, user-curated list of pages the human chose to keep,
//! stored as one JSON array in `bookmarks.json` beside the vault.
//!
//! This is the store module closest to [`crate::vault`]'s original shape —
//! the whole array is re-serialised on every mutation — with two deliberate
//! differences:
//!
//! - **It is plaintext, not encrypted.** A list a person deliberately curated
//!   is not a secret in the way a password is, and encrypting it would buy
//!   nothing the owner-only file permissions [`crate::permissions`] sets here
//!   do not already provide. That premise is load-bearing and was once
//!   false: the write below goes through [`crate::permissions`], which lands
//!   the file at `0600` and its directory at `0700`, rather than the `0644`
//!   and `0755` a bare `fs::write` and `create_dir_all` leave behind.
//! - **Every write goes through a temp file plus [`fs::rename`].** The vault
//!   writes its file in place; this module does not copy that line. A whole-
//!   array rewrite interrupted halfway leaves a file that parses as neither
//!   the old list nor the new one, and a rename costs nothing on a store this
//!   small. [`crate::history`] uses the same upgrade for its one whole-file
//!   write; here *every* write is a whole-file write, so every one of them is
//!   atomic.
//!
//! Unlike [`crate::history`], this store is not capped and not append-only: it
//! grows only when a human presses the star, and shrinks when they press it
//! again. There is deliberately no control-socket command and no MCP tool that
//! reaches it — what a person bookmarked is not an agent's to read or to add
//! to.

use std::fs;
use std::path::PathBuf;

use crate::permissions;

/// One bookmarked page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bookmark {
    /// The page's URL, and the identity of the bookmark: the same URL is
    /// never stored twice.
    pub url: String,
    /// The page's title at the moment it was bookmarked, or an empty string
    /// for a page that had none.
    pub title: String,
    /// Unix epoch milliseconds at the moment the bookmark was created.
    pub created_at_ms: u64,
}

impl Bookmark {
    /// This bookmark as the JSON object the stored array holds.
    ///
    /// Built through `serde_json::json!` rather than a `Serialize` derive
    /// because `serde` itself is not a dependency of this crate — only
    /// `serde_json` is — and this plan may not add one. `to_json` and
    /// `from_json` sit next to each other so a field added to one is visibly
    /// missing from the other.
    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "url": self.url,
            "title": self.title,
            "created_at_ms": self.created_at_ms,
        })
    }

    /// One stored object back into a bookmark, or `None` for an object this
    /// module cannot make sense of.
    ///
    /// Only `url` is load-bearing: an entry with no URL names no page and is
    /// counted as corrupt. A missing or malformed `title`/`created_at_ms`
    /// degrades to the empty string / epoch rather than discarding a row that
    /// still names a real page.
    fn from_json(value: &serde_json::Value) -> Option<Self> {
        Some(Self {
            url: value.get("url")?.as_str()?.to_owned(),
            title: value
                .get("title")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            created_at_ms: value
                .get("created_at_ms")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or_default(),
        })
    }
}

/// The human's bookmarks, in the order they were added.
pub struct Bookmarks {
    path: PathBuf,
    entries: Vec<Bookmark>,
}

/// Where every store in this crate keeps its file. Each store owns its own
/// copy of these lines rather than sharing a `paths` module: the stores
/// are deliberately independent, and one shared helper is one more thing that
/// has to be right for all of them at once.
fn config_dir() -> PathBuf {
    let path = dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("talaria");
    // Owner-only, and created here rather than in the write paths below.
    // This is the one directory known to be this browser's own, where a write
    // path is handed whatever `Path::parent` yields — the shared system temp
    // directory under a unit test. See [`crate::permissions`].
    permissions::create_dir_owner_only(&path);
    path
}

impl Bookmarks {
    /// Read the list once, at startup.
    ///
    /// Degrade, never abort: a file that cannot be read, or whose contents
    /// are not the array this module wrote, produces an empty list rather
    /// than an error the caller has to handle or a panic that takes startup
    /// with it.
    pub fn load() -> Self {
        Self::load_from(config_dir().join("bookmarks.json"))
    }

    fn load_from(path: PathBuf) -> Self {
        let stored: Option<Vec<serde_json::Value>> = fs::read(&path)
            .ok()
            .and_then(|data| serde_json::from_slice(&data).ok());
        let Some(stored) = stored else {
            // Absent and unreadable are the same answer — an empty list — but
            // only the second is worth saying out loud, and only when there
            // is something there to have failed on.
            if path.exists() {
                log::warn!("bookmarks: {} is unreadable; starting empty", path.display());
            }
            return Self { path, entries: Vec::new() };
        };
        let mut entries = Vec::with_capacity(stored.len());
        let mut skipped = 0usize;
        for value in &stored {
            match Bookmark::from_json(value) {
                Some(entry) => entries.push(entry),
                None => skipped += 1,
            }
        }
        if skipped > 0 {
            log::warn!("bookmarks: skipped {skipped} unreadable entry(s) in {}", path.display());
        }
        Self { path, entries }
    }

    /// Whether `url` is already bookmarked. A pure read: the star button
    /// calls this every frame.
    pub fn is_bookmarked(&self, url: &str) -> bool {
        self.entries.iter().any(|entry| entry.url == url)
    }

    /// Add a bookmark for `url`, unless one already exists.
    ///
    /// Not a toggle, and not an overwrite: a second call on the same URL is a
    /// no-op that does not touch the title and does not touch the file. The
    /// caller decides between this and [`Bookmarks::remove`] — the star
    /// button's add-or-remove decision lives in `apply_ui_actions`, not here.
    pub fn upsert(&mut self, url: String, title: String, created_at_ms: u64) {
        if self.is_bookmarked(&url) {
            return;
        }
        self.entries.push(Bookmark { url, title, created_at_ms });
        self.save();
    }

    /// Remove the bookmark for `url`, reporting whether one was there.
    /// Persists only when something was actually removed.
    pub fn remove(&mut self, url: &str) -> bool {
        let before = self.entries.len();
        self.entries.retain(|entry| entry.url != url);
        let removed = self.entries.len() != before;
        if removed {
            self.save();
        }
        removed
    }

    /// Every bookmark in insertion order, borrowed so the panel can list them
    /// without cloning the whole list. The panel reverses for display; the
    /// store never reorders itself, so two bookmarks created in the same
    /// millisecond keep the order the human made them in.
    pub fn entries(&self) -> &[Bookmark] {
        &self.entries
    }

    /// Write the whole list, atomically.
    ///
    /// Staged into a `.tmp` sibling and moved into place with [`fs::rename`],
    /// which is atomic on the same filesystem, so a process killed mid-save
    /// leaves either the old list or the new one — never a half-written file
    /// the next startup would read as corrupt and silently discard.
    ///
    /// Every failure here is logged and swallowed: a bookmark that cannot
    /// reach the disk still exists in this session, and losing it at the next
    /// restart is a better answer than taking the browser down over it.
    fn save(&mut self) {
        let array: Vec<serde_json::Value> =
            self.entries.iter().map(Bookmark::to_json).collect();
        let Ok(data) = serde_json::to_vec(&array) else {
            log::warn!("bookmarks could not be serialised; this change will not survive a restart");
            return;
        };
        // Staged beside the target, never in the temp directory: `fs::rename`
        // is only atomic within one filesystem, and a config directory on a
        // different mount than /tmp is an ordinary setup.
        let Some(name) = self.path.file_name().map(|name| name.to_string_lossy().into_owned())
        else {
            log::warn!("bookmarks path has no file name; not saving");
            return;
        };
        let staged = self.path.with_file_name(format!("{name}.tmp"));
        if let Some(parent) = self.path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        // Restricted on the staging file, not on the target: a rename swaps
        // the destination's inode for the staged one, so the mode that lands
        // is the staged file's. Restricting after the rename would leave a
        // window; restricting the target beforehand would achieve nothing.
        if let Err(error) = permissions::write_owner_only(&staged, &data) {
            log::warn!("could not stage bookmarks: {error}");
            return;
        }
        if let Err(error) = fs::rename(&staged, &self.path) {
            log::warn!("could not replace bookmarks with the new list: {error}");
            let _ = fs::remove_file(&staged);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    /// A bookmarks path in the temp directory that removes itself, and the
    /// `.tmp` sibling [`Bookmarks::save`] writes, when a test ends.
    ///
    /// `tempfile` is not a dependency of this crate and this plan may not add
    /// one, so the unique name is built here: the process id keeps two
    /// concurrent `cargo test` runs apart, and the counter keeps this
    /// process's own parallel test threads apart.
    struct TempPath(PathBuf);

    impl TempPath {
        fn new() -> Self {
            let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
            let name = format!("talaria-bookmarks-{}-{unique}.json", std::process::id());
            Self(std::env::temp_dir().join(name))
        }

        fn path(&self) -> PathBuf {
            self.0.clone()
        }

        /// The `.tmp` sibling a save stages through, which must never survive
        /// a completed save.
        fn temp_sibling(&self) -> PathBuf {
            let name = self.0.file_name().map(|name| name.to_string_lossy().into_owned());
            match name {
                Some(name) => self.0.with_file_name(format!("{name}.tmp")),
                None => self.0.clone(),
            }
        }

        fn write(&self, contents: &str) {
            fs::write(&self.0, contents).expect("write the test fixture");
        }

        fn read(&self) -> String {
            fs::read_to_string(&self.0).unwrap_or_default()
        }

        fn exists(&self) -> bool {
            self.0.exists()
        }
    }

    impl Drop for TempPath {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
            let _ = fs::remove_file(self.temp_sibling());
        }
    }

    #[test]
    fn a_missing_file_loads_as_an_empty_store() {
        let temp = TempPath::new();
        let bookmarks = Bookmarks::load_from(temp.path());
        assert!(bookmarks.entries().is_empty());
    }

    #[test]
    fn an_empty_file_loads_as_an_empty_store() {
        let temp = TempPath::new();
        temp.write("");
        let bookmarks = Bookmarks::load_from(temp.path());
        assert!(bookmarks.entries().is_empty());
    }

    #[test]
    fn a_corrupt_file_loads_as_an_empty_store_without_panicking() {
        let temp = TempPath::new();
        temp.write("{not json at all");
        let bookmarks = Bookmarks::load_from(temp.path());
        assert!(bookmarks.entries().is_empty());
    }

    /// A hand-edited file holding the right JSON of the wrong shape (an
    /// object, not an array) is the other half of the corruption case.
    #[test]
    fn a_file_of_the_wrong_shape_loads_as_an_empty_store() {
        let temp = TempPath::new();
        temp.write(r#"{"url":"https://example.com/"}"#);
        let bookmarks = Bookmarks::load_from(temp.path());
        assert!(bookmarks.entries().is_empty());
    }

    /// An array whose entries are individually broken costs only those
    /// entries — the same degrade the history store applies per line.
    #[test]
    fn an_entry_with_no_url_is_skipped_and_the_good_ones_survive() {
        let temp = TempPath::new();
        temp.write(
            r#"[{"title":"nameless","created_at_ms":1},
                {"url":"https://kept.example/","title":"Kept","created_at_ms":2}]"#,
        );
        let bookmarks = Bookmarks::load_from(temp.path());
        assert_eq!(bookmarks.entries().len(), 1);
        assert_eq!(bookmarks.entries()[0].url, "https://kept.example/");
    }

    #[test]
    fn an_upserted_bookmark_survives_a_reload() {
        let temp = TempPath::new();
        let mut bookmarks = Bookmarks::load_from(temp.path());
        bookmarks.upsert("https://example.com/".into(), "Example".into(), 42);

        let reloaded = Bookmarks::load_from(temp.path());
        assert_eq!(reloaded.entries().len(), 1);
        assert_eq!(reloaded.entries()[0].url, "https://example.com/");
        assert_eq!(reloaded.entries()[0].title, "Example");
        assert_eq!(reloaded.entries()[0].created_at_ms, 42);
    }

    /// The store's half of "the same URL is never stored twice". `upsert` is
    /// not the toggle — a second call adds nothing and changes nothing.
    #[test]
    fn upserting_the_same_url_twice_leaves_exactly_one_entry() {
        let temp = TempPath::new();
        let mut bookmarks = Bookmarks::load_from(temp.path());
        bookmarks.upsert("https://example.com/".into(), "First".into(), 1);
        bookmarks.upsert("https://example.com/".into(), "Second".into(), 2);

        assert_eq!(bookmarks.entries().len(), 1, "the same url was stored twice");
        assert_eq!(bookmarks.entries()[0].title, "First", "upsert overwrote an entry");
        assert_eq!(bookmarks.entries()[0].created_at_ms, 1);

        let reloaded = Bookmarks::load_from(temp.path());
        assert_eq!(reloaded.entries().len(), 1);
        assert_eq!(reloaded.entries()[0].title, "First");
    }

    #[test]
    fn entries_keep_their_insertion_order_across_a_reload() {
        let temp = TempPath::new();
        let mut bookmarks = Bookmarks::load_from(temp.path());
        for index in 0..4u64 {
            bookmarks.upsert(format!("https://example.com/{index}"), String::new(), index);
        }

        let reloaded = Bookmarks::load_from(temp.path());
        let urls: Vec<&str> = reloaded.entries().iter().map(|entry| entry.url.as_str()).collect();
        assert_eq!(
            urls,
            vec![
                "https://example.com/0",
                "https://example.com/1",
                "https://example.com/2",
                "https://example.com/3",
            ]
        );
    }

    #[test]
    fn is_bookmarked_follows_upsert_and_remove() {
        let temp = TempPath::new();
        let mut bookmarks = Bookmarks::load_from(temp.path());
        assert!(!bookmarks.is_bookmarked("https://example.com/"));

        bookmarks.upsert("https://example.com/".into(), "Example".into(), 1);
        assert!(bookmarks.is_bookmarked("https://example.com/"));
        assert!(!bookmarks.is_bookmarked("https://other.example/"));

        assert!(bookmarks.remove("https://example.com/"));
        assert!(!bookmarks.is_bookmarked("https://example.com/"));
    }

    #[test]
    fn removing_an_absent_url_reports_false_and_writes_nothing() {
        let temp = TempPath::new();
        let mut bookmarks = Bookmarks::load_from(temp.path());
        assert!(!bookmarks.remove("https://never-bookmarked.example/"));
        assert!(!temp.exists(), "a no-op remove wrote the file anyway");
    }

    #[test]
    fn removing_a_present_url_reports_true_and_does_not_survive_a_reload() {
        let temp = TempPath::new();
        let mut bookmarks = Bookmarks::load_from(temp.path());
        bookmarks.upsert("https://one.example/".into(), "One".into(), 1);
        bookmarks.upsert("https://two.example/".into(), "Two".into(), 2);

        assert!(bookmarks.remove("https://one.example/"));

        let reloaded = Bookmarks::load_from(temp.path());
        assert_eq!(reloaded.entries().len(), 1);
        assert_eq!(reloaded.entries()[0].url, "https://two.example/");
    }

    /// Stored in plaintext on the module header's promise that the file
    /// permissions carry the weight. This is that promise — and because the
    /// save renames a staged sibling into place, and a rename carries the
    /// *staged* file's mode, it is also what pins the mode being set before
    /// the rename rather than after it.
    #[cfg(unix)]
    #[test]
    fn a_saved_list_lands_owner_only_through_the_rename() {
        let temp = TempPath::new();
        let mut bookmarks = Bookmarks::load_from(temp.path());
        bookmarks.upsert("https://example.com/".into(), "Example".into(), 1);

        assert!(temp.exists(), "the save did not land");
        assert_eq!(permissions::mode_of(&temp.path()), 0o600);
    }

    /// T-03-02-01: the target file only ever changes through a rename, so a
    /// completed save leaves no staging file behind and the target always
    /// parses.
    #[test]
    fn a_save_stages_through_a_temp_file_and_leaves_none_behind() {
        let temp = TempPath::new();
        let mut bookmarks = Bookmarks::load_from(temp.path());
        bookmarks.upsert("https://example.com/".into(), "Example".into(), 1);

        assert!(temp.exists(), "the save did not land");
        assert!(!temp.temp_sibling().exists(), "the save left its staging file behind");
        let parsed: serde_json::Value =
            serde_json::from_str(&temp.read()).expect("the saved file is valid JSON");
        assert!(parsed.is_array(), "the saved file is not an array");
    }

    /// A stale `.tmp` from an interrupted earlier save must not be read as
    /// data, and must not stop the next save from landing.
    #[test]
    fn a_stale_staging_file_does_not_block_the_next_save() {
        let temp = TempPath::new();
        fs::write(temp.temp_sibling(), b"garbage from a killed process")
            .expect("write the stale staging file");

        let mut bookmarks = Bookmarks::load_from(temp.path());
        bookmarks.upsert("https://example.com/".into(), "Example".into(), 1);

        let reloaded = Bookmarks::load_from(temp.path());
        assert_eq!(reloaded.entries().len(), 1);
        assert_eq!(reloaded.entries()[0].url, "https://example.com/");
    }

    #[test]
    fn a_save_replaces_the_whole_array_rather_than_appending_to_it() {
        let temp = TempPath::new();
        let mut bookmarks = Bookmarks::load_from(temp.path());
        bookmarks.upsert("https://one.example/".into(), "One".into(), 1);
        bookmarks.upsert("https://two.example/".into(), "Two".into(), 2);
        bookmarks.remove("https://one.example/");

        let parsed: serde_json::Value =
            serde_json::from_str(&temp.read()).expect("the saved file is valid JSON");
        let array = parsed.as_array().expect("the saved file is an array");
        assert_eq!(array.len(), 1, "the removed entry is still on disk");
    }
}

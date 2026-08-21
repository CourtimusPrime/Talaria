//! Browsing history: an append-only log of the pages the human visited, one
//! JSON object per line in `history.jsonl` beside the vault.
//!
//! Two things separate this store from [`crate::vault`], which is otherwise
//! the template every store module in this crate follows:
//!
//! - **It is plaintext, deliberately.** A list of visited URLs is not a
//!   secret in the way a password is, and encrypting it would buy nothing the
//!   owner-only file permissions [`crate::permissions`] sets here do not
//!   already provide, while costing the ability to read the file with `tail`.
//!   That premise is load-bearing and was once false: every write in this
//!   module now goes through [`crate::permissions`], and the load path
//!   repairs a log left world-readable by an older build, because a URL list
//!   carries session tokens, reset links and search terms in its query
//!   strings and is not a thing to leave at `0644`.
//! - **It is an append log, not a whole-array rewrite.** The vault holds
//!   dozens of entries and is written when a human saves a credential;
//!   history is written on every completed navigation and grows forever, so
//!   re-serialising the whole vector per write would get slower every month.
//!   [`History::append`] writes exactly one line; the only whole-file rewrite
//!   is the retention-cap prune, which runs at most once per startup.
//!
//! History records the *human's* browsing only, and the filter that makes
//! that true asks who **caused** a load rather than who owns the tab it
//! happened in. Owning the tab is necessary but not sufficient: an agent can
//! act on one of the human's own tabs, so a load an agent started there is
//! skipped too. The filter lives in `Shared::process_pending_history_writes`
//! (see `belongs_in_history` beside it), which is the one caller.

use std::fs;
use std::io::Write;
use std::path::PathBuf;

use crate::permissions;

/// One visited page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryEntry {
    /// The page's URL as the engine reported it once the load completed.
    pub url: String,
    /// The page's title, or an empty string for a page that has none.
    pub title: String,
    /// Unix epoch milliseconds at the moment the visit was recorded.
    pub visited_at_ms: u64,
}

impl HistoryEntry {
    /// This entry as the single JSON line the log stores it on.
    ///
    /// Built through `serde_json::json!` rather than a `Serialize` derive
    /// because `serde` itself is not a dependency of this crate — only
    /// `serde_json` is — and this plan may not add one. The two functions
    /// below are the whole of the mapping, and they sit next to each other so
    /// a field added to one is visibly missing from the other.
    fn to_json_line(&self) -> Option<String> {
        let value = serde_json::json!({
            "url": self.url,
            "title": self.title,
            "visited_at_ms": self.visited_at_ms,
        });
        serde_json::to_string(&value).ok()
    }

    /// One stored line back into an entry, or `None` for a line this module
    /// cannot make sense of.
    ///
    /// Only `url` is load-bearing: a row with no URL is not a visit and is
    /// counted as corrupt. A missing or malformed `title`/`visited_at_ms`
    /// degrades to the empty string / epoch rather than discarding a row that
    /// still names a real page.
    fn from_json_line(line: &str) -> Option<Self> {
        let value: serde_json::Value = serde_json::from_str(line).ok()?;
        Some(Self {
            url: value.get("url")?.as_str()?.to_owned(),
            title: value
                .get("title")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            visited_at_ms: value
                .get("visited_at_ms")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or_default(),
        })
    }
}

/// The human's browsing history, in append order.
pub struct History {
    path: PathBuf,
    entries: Vec<HistoryEntry>,
}

/// How many entries survive a load when nothing overrides it.
const DEFAULT_MAX_HISTORY_ENTRIES: usize = 5_000;

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

/// The retention cap, tunable through `TALARIA_HISTORY_MAX_ENTRIES`.
fn history_max_entries() -> usize {
    parse_max_entries(std::env::var("TALARIA_HISTORY_MAX_ENTRIES").ok())
}

/// The parsing half of [`history_max_entries`], split out so the fallback is
/// testable without mutating this process's environment.
fn parse_max_entries(raw: Option<String>) -> usize {
    // Zero is refused rather than honoured. `"0".parse::<usize>()` succeeds,
    // so it used to reach [`History::prune_to`] as a real cap and empty the
    // log on every startup — which is the exact opposite of what a person who
    // set it reading "0" as "no limit" was asking for, and it is not
    // recoverable. Clearing history is the History panel's Clear button, a
    // thing a human does once on purpose, not a thing an environment variable
    // does silently at every launch. Same shape as `vault::keychain_timeout`,
    // which filters the same class of bad override the same way.
    raw.and_then(|value| value.parse::<usize>().ok())
        .filter(|entries| *entries > 0)
        .unwrap_or(DEFAULT_MAX_HISTORY_ENTRIES)
}

impl History {
    /// Read the log once, at startup, and prune it to the retention cap.
    ///
    /// Degrade, never abort: a file that cannot be read, or whose every line
    /// is nonsense, produces an empty store rather than an error the caller
    /// has to handle or a panic that takes startup with it.
    pub fn load() -> Self {
        Self::load_from(config_dir().join("history.jsonl"), history_max_entries())
    }

    fn load_from(path: PathBuf, max_entries: usize) -> Self {
        // The one store whose file is never recreated in the ordinary case,
        // so the create-time mode in `permissions::append_owner_only` can
        // never reach a log an older build already left at 0644. Repairing it
        // costs one `chmod` per startup, and only when the file is there.
        let contents = match fs::read_to_string(&path) {
            Ok(contents) => {
                permissions::restrict_file(&path);
                contents
            },
            Err(_) => String::new(),
        };
        let mut entries = Vec::new();
        let mut skipped = 0usize;
        for line in contents.lines().filter(|line| !line.trim().is_empty()) {
            match HistoryEntry::from_json_line(line) {
                Some(entry) => entries.push(entry),
                // Counted rather than logged one by one: an interrupted append
                // truncates the trailing line, and a hand-edited file can ruin
                // many at once — neither is worth a warning per line.
                None => skipped += 1,
            }
        }
        if skipped > 0 {
            log::warn!("history: skipped {skipped} unreadable line(s) in {}", path.display());
        }
        let mut history = Self { path, entries };
        history.prune_to(max_entries);
        history
    }

    /// Record one visit, in memory and on disk.
    ///
    /// A revisit is a second row, never an update of the first — history is a
    /// chronological log, not a set of distinct pages.
    ///
    /// The write is a single appended line, deliberately without the
    /// temp-file-plus-rename [`History::prune_to`] uses: an interrupted append
    /// can corrupt at most its own trailing line, which the load path already
    /// tolerates, whereas a rename per navigation would rewrite the whole file
    /// every time the user opened a page. A failed write is logged and the
    /// in-memory row is kept, so the panel still shows this session's visit
    /// even when it will not survive a restart.
    pub fn append(&mut self, url: String, title: String, visited_at_ms: u64) {
        self.entries.push(HistoryEntry { url, title, visited_at_ms });
        let Some(entry) = self.entries.last() else { return };
        let Some(mut record) = entry.to_json_line() else {
            log::warn!("history entry could not be serialised; it will not survive a restart");
            return;
        };
        record.push('\n');
        if let Some(parent) = self.path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let opened = permissions::append_owner_only(&self.path);
        let mut file = match opened {
            Ok(file) => file,
            Err(error) => {
                log::warn!("could not open history for append: {error}");
                return;
            },
        };
        if let Err(error) = file.write_all(record.as_bytes()) {
            log::warn!("could not write history entry: {error}");
            return;
        }
        if let Err(error) = file.flush() {
            log::warn!("could not flush history: {error}");
        }
    }

    /// Forget everything, in memory and on disk. Idempotent: clearing a store
    /// that has no file is what a second click does, not an error.
    pub fn clear(&mut self) {
        self.entries.clear();
        match fs::remove_file(&self.path) {
            Ok(()) => {},
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {},
            Err(error) => log::warn!("could not remove history file: {error}"),
        }
    }

    /// Every recorded visit in append order, borrowed so the panel can list
    /// them without cloning the whole log.
    pub fn entries(&self) -> &[HistoryEntry] {
        &self.entries
    }

    /// Drop everything past the retention cap, keeping the most recent
    /// `max_entries`, and shrink the file to match.
    ///
    /// This is the module's only whole-file rewrite, and it runs at most once
    /// per startup. It goes through a temp file plus [`fs::rename`], which is
    /// atomic on the same filesystem, so a process killed mid-prune leaves
    /// either the old log or the new one — never a half-written file that the
    /// next startup would read as mostly corrupt.
    fn prune_to(&mut self, max_entries: usize) {
        let before = self.entries.len();
        if before <= max_entries {
            return;
        }
        self.entries = self.entries.split_off(before - max_entries);
        log::warn!("history: pruned {before} entries down to {max_entries}");

        let mut data = String::new();
        for entry in &self.entries {
            let Some(line) = entry.to_json_line() else {
                log::warn!("history: an entry could not be serialised; leaving the file as it is");
                return;
            };
            data.push_str(&line);
            data.push('\n');
        }
        let Some(name) = self.path.file_name().map(|name| name.to_string_lossy().into_owned())
        else {
            return;
        };
        let temp = self.path.with_file_name(format!("{name}.tmp"));
        if let Some(parent) = self.path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        // Restricted on the staging file, not on the target: a rename swaps
        // the destination's inode for the staged one, so the mode that lands
        // is the staged file's.
        if let Err(error) = permissions::write_owner_only(&temp, data.as_bytes()) {
            log::warn!("could not stage the pruned history: {error}");
            return;
        }
        if let Err(error) = fs::rename(&temp, &self.path) {
            log::warn!("could not replace history with its pruned copy: {error}");
            let _ = fs::remove_file(&temp);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    /// A history path in the temp directory that removes itself, and the
    /// `.tmp` sibling the prune path writes, when a test ends.
    ///
    /// `tempfile` is not a dependency of this crate and this plan may not add
    /// one, so the unique name is built here: the process id keeps two
    /// concurrent `cargo test` runs apart, and the counter keeps this
    /// process's own parallel test threads apart.
    struct TempPath(PathBuf);

    impl TempPath {
        fn new() -> Self {
            let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
            let name = format!("talaria-history-{}-{unique}.jsonl", std::process::id());
            Self(std::env::temp_dir().join(name))
        }

        fn path(&self) -> PathBuf {
            self.0.clone()
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
            if let Some(name) = self.0.file_name() {
                let temp = self.0.with_file_name(format!("{}.tmp", name.to_string_lossy()));
                let _ = fs::remove_file(temp);
            }
        }
    }

    fn line(url: &str, title: &str, visited_at_ms: u64) -> String {
        format!(r#"{{"url":"{url}","title":"{title}","visited_at_ms":{visited_at_ms}}}"#)
    }

    #[test]
    fn a_missing_file_loads_as_an_empty_store() {
        let temp = TempPath::new();
        let history = History::load_from(temp.path(), 5_000);
        assert!(history.entries().is_empty());
    }

    #[test]
    fn an_empty_file_loads_as_an_empty_store() {
        let temp = TempPath::new();
        temp.write("");
        let history = History::load_from(temp.path(), 5_000);
        assert!(history.entries().is_empty());
    }

    #[test]
    fn an_appended_entry_survives_a_reload() {
        let temp = TempPath::new();
        let mut history = History::load_from(temp.path(), 5_000);
        history.append("https://example.com/".into(), "Example".into(), 42);

        let reloaded = History::load_from(temp.path(), 5_000);
        assert_eq!(reloaded.entries().len(), 1);
        assert_eq!(reloaded.entries()[0].url, "https://example.com/");
        assert_eq!(reloaded.entries()[0].title, "Example");
        assert_eq!(reloaded.entries()[0].visited_at_ms, 42);
    }

    #[test]
    fn the_same_url_twice_is_two_entries_not_one_updated_one() {
        let temp = TempPath::new();
        let mut history = History::load_from(temp.path(), 5_000);
        history.append("https://example.com/".into(), "Example".into(), 1);
        history.append("https://example.com/".into(), "Example".into(), 2);

        assert_eq!(history.entries().len(), 2, "history merged a revisit");
        let reloaded = History::load_from(temp.path(), 5_000);
        assert_eq!(reloaded.entries().len(), 2, "the revisit did not reach disk");
        assert_eq!(reloaded.entries()[0].visited_at_ms, 1);
        assert_eq!(reloaded.entries()[1].visited_at_ms, 2);
    }

    /// The log is a complete record of where the human went, query strings
    /// and all, and it is stored in plaintext on the module header's promise
    /// that the file permissions carry the weight. This is that promise.
    #[cfg(unix)]
    #[test]
    fn an_appended_log_lands_owner_only() {
        let temp = TempPath::new();
        let mut history = History::load_from(temp.path(), 5_000);
        history.append("https://example.com/".into(), "Example".into(), 1);

        assert!(temp.exists(), "the append did not land");
        assert_eq!(permissions::mode_of(&temp.path()), 0o600);
    }

    /// A log an older build left world-readable is never recreated — the
    /// append path only ever opens it — so the load path is the only place
    /// that can bring it down.
    #[cfg(unix)]
    #[test]
    fn a_world_readable_log_is_brought_down_at_load() {
        use std::os::unix::fs::PermissionsExt;

        let temp = TempPath::new();
        temp.write(&format!("{}\n", line("https://example.com/", "Example", 1)));
        fs::set_permissions(temp.path(), fs::Permissions::from_mode(0o644))
            .expect("loosen the test fixture");
        assert_eq!(permissions::mode_of(&temp.path()), 0o644, "the fixture must start loose");

        let history = History::load_from(temp.path(), 5_000);
        assert_eq!(history.entries().len(), 1, "the repair must not cost the contents");
        assert_eq!(permissions::mode_of(&temp.path()), 0o600);
    }

    /// The prune is the module's one whole-file rewrite, and it lands through
    /// a staged sibling. A rename carries the *staged* file's mode, so the
    /// mode has to be right before the rename, not after it.
    #[cfg(unix)]
    #[test]
    fn a_pruned_log_lands_owner_only_through_the_rename() {
        let temp = TempPath::new();
        let mut fixture = String::new();
        for index in 0..10u64 {
            fixture.push_str(&line(&format!("https://example.com/{index}"), "", index));
            fixture.push('\n');
        }
        temp.write(&fixture);

        let history = History::load_from(temp.path(), 3);
        assert_eq!(history.entries().len(), 3, "the prune did not run");
        assert_eq!(permissions::mode_of(&temp.path()), 0o600);
    }

    #[test]
    fn entries_keep_their_append_order_across_a_reload() {
        let temp = TempPath::new();
        let mut history = History::load_from(temp.path(), 5_000);
        for index in 0..5u64 {
            history.append(format!("https://example.com/{index}"), String::new(), index);
        }

        let reloaded = History::load_from(temp.path(), 5_000);
        let urls: Vec<&str> = reloaded.entries().iter().map(|e| e.url.as_str()).collect();
        assert_eq!(
            urls,
            vec![
                "https://example.com/0",
                "https://example.com/1",
                "https://example.com/2",
                "https://example.com/3",
                "https://example.com/4",
            ]
        );
    }

    #[test]
    fn a_corrupt_line_is_skipped_and_the_good_ones_survive() {
        let temp = TempPath::new();
        temp.write(&format!(
            "{}\n{{not json at all\n{}\n",
            line("https://one.example/", "One", 1),
            line("https://two.example/", "Two", 2),
        ));

        let history = History::load_from(temp.path(), 5_000);
        assert_eq!(history.entries().len(), 2);
        assert_eq!(history.entries()[0].url, "https://one.example/");
        assert_eq!(history.entries()[1].url, "https://two.example/");
    }

    #[test]
    fn a_truncated_trailing_line_costs_only_that_line() {
        let temp = TempPath::new();
        temp.write(&format!(
            "{}\n{{\"url\":\"https://two.exam",
            line("https://one.example/", "One", 1),
        ));

        let history = History::load_from(temp.path(), 5_000);
        assert_eq!(history.entries().len(), 1);
        assert_eq!(history.entries()[0].url, "https://one.example/");
    }

    #[test]
    fn an_entirely_unreadable_file_loads_as_an_empty_store() {
        let temp = TempPath::new();
        temp.write("nonsense\nmore nonsense\n");
        let history = History::load_from(temp.path(), 5_000);
        assert!(history.entries().is_empty());
    }

    #[test]
    fn a_store_over_the_cap_keeps_only_the_most_recent_entries() {
        let temp = TempPath::new();
        let mut contents = String::new();
        for index in 0..10u64 {
            contents.push_str(&line(&format!("https://example.com/{index}"), "", index));
            contents.push('\n');
        }
        temp.write(&contents);

        let history = History::load_from(temp.path(), 3);
        assert_eq!(history.entries().len(), 3);
        assert_eq!(history.entries()[0].url, "https://example.com/7");
        assert_eq!(history.entries()[2].url, "https://example.com/9");
    }

    #[test]
    fn the_cap_prune_shrinks_the_file_too() {
        let temp = TempPath::new();
        let mut contents = String::new();
        for index in 0..10u64 {
            contents.push_str(&line(&format!("https://example.com/{index}"), "", index));
            contents.push('\n');
        }
        temp.write(&contents);

        let _pruned = History::load_from(temp.path(), 3);
        assert_eq!(
            temp.read().lines().filter(|l| !l.trim().is_empty()).count(),
            3,
            "the prune kept the file at its old size"
        );

        let reloaded = History::load_from(temp.path(), 5_000);
        assert_eq!(reloaded.entries().len(), 3, "the prune did not survive a reload");
    }

    #[test]
    fn a_store_under_the_cap_is_left_alone() {
        let temp = TempPath::new();
        temp.write(&format!("{}\n", line("https://one.example/", "One", 1)));
        let before = temp.read();

        let history = History::load_from(temp.path(), 5_000);
        assert_eq!(history.entries().len(), 1);
        assert_eq!(temp.read(), before, "an under-cap load rewrote the file anyway");
    }

    #[test]
    fn clear_empties_the_store_and_removes_the_file() {
        let temp = TempPath::new();
        let mut history = History::load_from(temp.path(), 5_000);
        history.append("https://example.com/".into(), "Example".into(), 1);
        assert!(temp.exists());

        history.clear();
        assert!(history.entries().is_empty());
        assert!(!temp.exists(), "clear left the history file on disk");
        assert!(History::load_from(temp.path(), 5_000).entries().is_empty());
    }

    #[test]
    fn clearing_an_already_empty_store_is_not_an_error() {
        let temp = TempPath::new();
        let mut history = History::load_from(temp.path(), 5_000);
        history.clear();
        history.clear();
        assert!(history.entries().is_empty());
    }

    #[test]
    fn appending_after_a_clear_starts_the_file_again() {
        let temp = TempPath::new();
        let mut history = History::load_from(temp.path(), 5_000);
        history.append("https://one.example/".into(), "One".into(), 1);
        history.clear();
        history.append("https://two.example/".into(), "Two".into(), 2);

        let reloaded = History::load_from(temp.path(), 5_000);
        assert_eq!(reloaded.entries().len(), 1);
        assert_eq!(reloaded.entries()[0].url, "https://two.example/");
    }

    #[test]
    fn an_unset_cap_falls_back_to_the_default() {
        assert_eq!(parse_max_entries(None), DEFAULT_MAX_HISTORY_ENTRIES);
    }

    #[test]
    fn an_unparseable_cap_falls_back_to_the_default_not_to_zero() {
        assert_eq!(parse_max_entries(Some("lots".into())), DEFAULT_MAX_HISTORY_ENTRIES);
        assert_eq!(parse_max_entries(Some("-1".into())), DEFAULT_MAX_HISTORY_ENTRIES);
        assert_eq!(parse_max_entries(Some(String::new())), DEFAULT_MAX_HISTORY_ENTRIES);
        // The one input that actually reaches zero, and the reason this test
        // is named the way it is: `"0"` parses, so the fallback above does not
        // fire for it and it needs its own filter.
        assert_eq!(parse_max_entries(Some("0".into())), DEFAULT_MAX_HISTORY_ENTRIES);
    }

    /// The consequence the guard exists to prevent, pinned end to end rather
    /// than at the parse alone: a cap of zero used to prune every row and
    /// rewrite `history.jsonl` empty at every startup.
    #[test]
    fn a_zero_cap_does_not_empty_the_log() {
        let temp = TempPath::new();
        let mut fixture = String::new();
        for index in 0..3u64 {
            fixture.push_str(&line(&format!("https://example.com/{index}"), "", index));
            fixture.push('\n');
        }
        temp.write(&fixture);

        let history = History::load_from(temp.path(), parse_max_entries(Some("0".into())));
        assert_eq!(history.entries().len(), 3, "a zero cap deleted the whole log");
        assert!(!temp.read().is_empty(), "a zero cap emptied the file");
    }

    #[test]
    fn a_valid_cap_override_is_honoured() {
        assert_eq!(parse_max_entries(Some("25".into())), 25);
    }
}

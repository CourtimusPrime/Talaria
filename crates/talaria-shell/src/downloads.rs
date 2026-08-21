//! Downloads: the list of files that actually landed on the human's disk,
//! stored as one JSON array in `downloads.json` beside the vault.
//!
//! Shaped after [`crate::bookmarks`] — a whole-array store, re-serialised and
//! atomically renamed into place on every mutation — with three differences
//! worth stating rather than leaving to be inferred:
//!
//! - **It records every completed download, whoever asked for it.** This is
//!   the deliberate opposite of [`crate::history`]'s Me-only filter. A visit
//!   an agent made is the agent's business; a *file* an agent wrote is on the
//!   human's filesystem, and a person who cannot see what appeared in their
//!   downloads directory cannot make a decision about it. Nothing here
//!   filters, and nothing here ever should.
//! - **It records who asked, and filtering is still not what that is for.**
//!   [`DownloadEntry::requested_by_agent`] exists because the row is not only
//!   read — the Downloads panel hangs a one-click handoff to the OS's default
//!   application off it, and an agent picked both the bytes and the extension
//!   that decides which application that is. A person choosing whether to
//!   press that button is choosing with this fact or without it. Recording it
//!   is the opposite of dropping the row.
//! - **Its `path` field is the path that was actually written**, as returned
//!   by `create_unique`, never the filename the request asked for. Those two
//!   differ whenever a name collided (`report.pdf` → `report (1).pdf`), and
//!   the stored path is what the Downloads panel's Open button hands to the
//!   OS. A re-derived path would be an opener pointed at a file this browser
//!   did not write.
//! - **It is plaintext and uncapped.** Plaintext for the same reason
//!   bookmarks are: a list of files a person already has is not a secret in
//!   the way a password is. Uncapped per `03-UI-SPEC.md`'s Open Question 6 —
//!   nothing in BROWSE-04 asks for retention, and real-world download volume
//!   is not going to make an unbounded array matter inside v1's lifetime.
//!   Flagged deliberately, not assumed silently; a cap is additive later.
//!
//! There is deliberately no control-socket command and no MCP tool that
//! reaches this store, and none that reaches the opener the panel drives. The
//! list is written *from* an agent's action and read only by the human.

use std::fs;
use std::path::PathBuf;

use crate::permissions;

/// One completed download.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DownloadEntry {
    /// The file's location on disk, exactly as written — `create_unique`'s
    /// returned path, which may carry a ` (1)` suffix the request never asked
    /// for. The identity of the entry, and the only thing the Open button is
    /// ever allowed to launch.
    pub path: String,
    /// The name the download was requested under. Display text only: it is
    /// *not* the basename of [`DownloadEntry::path`] when a collision was
    /// resolved, and nothing may reconstruct a path from it.
    pub filename: String,
    /// Where the bytes came from.
    pub url: String,
    /// How many bytes were written, counted from the bytes actually read and
    /// written rather than from any `Content-Length` header.
    pub bytes: u64,
    /// Unix epoch milliseconds at the moment the download finished.
    pub completed_at_ms: u64,
    /// Whether a `download` control-socket command asked for this file
    /// rather than the human.
    ///
    /// Not a filter — every completed download is a row either way. It is
    /// here for the Downloads panel's `Open` button, which hands the file to
    /// the OS's default application: an agent chose the bytes and chose the
    /// extension that selects that application, and the human pressing the
    /// button is entitled to know that before they do.
    ///
    /// A bool rather than the requesting session's label, deliberately. The
    /// label is a string the agent picked in its own `hello`, so putting it
    /// in the row would hand the same actor a second place to write copy the
    /// human reads. "An agent asked for this" is the whole of what the
    /// decision needs and is not something an agent can word.
    pub requested_by_agent: bool,
}

impl DownloadEntry {
    /// This entry as the JSON object the stored array holds.
    ///
    /// Built through `serde_json::json!` rather than a `Serialize` derive
    /// because `serde` itself is not a dependency of this crate — only
    /// `serde_json` is — and this phase may not add one. `to_json` and
    /// `from_json` sit next to each other so a field added to one is visibly
    /// missing from the other.
    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "path": self.path,
            "filename": self.filename,
            "url": self.url,
            "bytes": self.bytes,
            "completed_at_ms": self.completed_at_ms,
            "requested_by_agent": self.requested_by_agent,
        })
    }

    /// One stored object back into an entry, or `None` for an object this
    /// module cannot make sense of.
    ///
    /// Only `path` is load-bearing: an entry with no path names no file, so
    /// there is nothing for Open to launch and nothing for Remove to key on.
    /// A missing or malformed `filename`/`url`/`bytes`/`completed_at_ms`
    /// degrades to the empty string / zero rather than discarding a row that
    /// still names a real file on the human's disk.
    ///
    /// `requested_by_agent` degrades to **`true`**, which is the one default
    /// here that is not the falsy one. A `downloads.json` written before that
    /// field existed still loads unchanged — but every row in such a file
    /// came through `Command::Download`, the control socket's own command,
    /// because no human-initiated download path exists to have written any of
    /// them. So `true` is the factually correct answer for a legacy row, and
    /// it is also the cautious one: where the provenance of a file is not
    /// recorded, the answer that makes the human look twice is the safe one
    /// to give.
    fn from_json(value: &serde_json::Value) -> Option<Self> {
        Some(Self {
            path: value.get("path")?.as_str()?.to_owned(),
            filename: value
                .get("filename")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            url: value
                .get("url")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            bytes: value
                .get("bytes")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or_default(),
            completed_at_ms: value
                .get("completed_at_ms")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or_default(),
            requested_by_agent: value
                .get("requested_by_agent")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(true),
        })
    }
}

/// Every completed download, in the order they finished.
pub struct Downloads {
    /// This store's own file (`downloads.json`) — not to be confused with
    /// [`DownloadEntry::path`], which is a downloaded file's location.
    path: PathBuf,
    entries: Vec<DownloadEntry>,
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

impl Downloads {
    /// Read the list once, at startup.
    ///
    /// Degrade, never abort: a file that cannot be read, or whose contents
    /// are not the array this module wrote, produces an empty list rather
    /// than an error the caller has to handle or a panic that takes startup
    /// with it.
    pub fn load() -> Self {
        Self::load_from(config_dir().join("downloads.json"))
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
                log::warn!("downloads: {} is unreadable; starting empty", path.display());
            }
            return Self { path, entries: Vec::new() };
        };
        let mut entries = Vec::with_capacity(stored.len());
        let mut skipped = 0usize;
        for value in &stored {
            match DownloadEntry::from_json(value) {
                Some(entry) => entries.push(entry),
                None => skipped += 1,
            }
        }
        if skipped > 0 {
            log::warn!("downloads: skipped {skipped} unreadable entry(s) in {}", path.display());
        }
        Self { path, entries }
    }

    /// Every completed download in the order it finished, borrowed so the
    /// panel can list them without cloning. The panel reverses for display;
    /// the store never reorders itself, so two downloads that finished in the
    /// same millisecond keep the order they actually finished in.
    pub fn entries(&self) -> &[DownloadEntry] {
        &self.entries
    }

    /// Record one completed download.
    ///
    /// Always adds a row. There is no "already present" check and no merge,
    /// because two completed downloads are two rows by construction — even
    /// two of the same requested `filename`, which `create_unique` has
    /// already given two different `path`s by the time this is called.
    pub fn append(
        &mut self,
        path: String,
        filename: String,
        url: String,
        bytes: u64,
        completed_at_ms: u64,
        requested_by_agent: bool,
    ) {
        self.entries.push(DownloadEntry {
            path,
            filename,
            url,
            bytes,
            completed_at_ms,
            requested_by_agent,
        });
        self.save();
    }

    /// Remove the entry at `path`, reporting whether one was there.
    ///
    /// Removes a *row*, never a file. Nothing in this function touches the
    /// filesystem at `path`, and nothing in this module imports anything that
    /// could — which is what makes the panel's `Remove from list` hover text
    /// true rather than merely intended.
    pub fn remove(&mut self, path: &str) -> bool {
        let before = self.entries.len();
        self.entries.retain(|entry| entry.path != path);
        let removed = self.entries.len() != before;
        if removed {
            self.save();
        }
        removed
    }

    /// Write the whole list, atomically.
    ///
    /// Staged into a `.tmp` sibling and moved into place with [`fs::rename`],
    /// which is atomic on the same filesystem, so a process killed mid-save
    /// leaves either the old list or the new one — never a half-written file
    /// the next startup would read as corrupt and silently discard. The same
    /// shape [`crate::bookmarks`] and [`crate::settings`] use, and for the
    /// same reason: every write here is a whole-file write.
    ///
    /// Every failure is logged and swallowed: a download that cannot be
    /// recorded still landed on disk, and losing its list row at the next
    /// restart is a better answer than taking the browser down over it.
    fn save(&mut self) {
        let array: Vec<serde_json::Value> =
            self.entries.iter().map(DownloadEntry::to_json).collect();
        let Ok(data) = serde_json::to_vec(&array) else {
            log::warn!("downloads could not be serialised; this change will not survive a restart");
            return;
        };
        // Staged beside the target, never in the temp directory: `fs::rename`
        // is only atomic within one filesystem, and a config directory on a
        // different mount than /tmp is an ordinary setup.
        let Some(name) = self.path.file_name().map(|name| name.to_string_lossy().into_owned())
        else {
            log::warn!("downloads path has no file name; not saving");
            return;
        };
        let staged = self.path.with_file_name(format!("{name}.tmp"));
        if let Some(parent) = self.path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        // Restricted on the staging file, not on the target: a rename swaps
        // the destination's inode for the staged one, so the mode that lands
        // is the staged file's. The list of what the human downloaded is as
        // much a record of their browsing as `history.jsonl` is.
        if let Err(error) = permissions::write_owner_only(&staged, &data) {
            log::warn!("could not stage downloads: {error}");
            return;
        }
        if let Err(error) = fs::rename(&staged, &self.path) {
            // `bookmarks.rs` unlinks its staging file here; this module
            // deliberately does not, and the difference is the point. Nothing
            // in this file — not `remove`, not `save`, not a helper either of
            // them might grow later — calls anything that deletes a file, so
            // the lever that could delete a *downloaded* file does not exist
            // here to be reached for by mistake. The cost is a stale `.tmp`
            // after a failed rename, which is harmless: it is never read as
            // data (`load` only ever opens `downloads.json`) and the next
            // save overwrites it, both pinned by
            // `a_stale_staging_file_does_not_block_the_next_save`.
            log::warn!("could not replace downloads with the new list: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    /// A scratch directory holding one test's `downloads.json`, the `.tmp`
    /// sibling a save stages through, and any stand-in downloaded files —
    /// removed whole when the test ends.
    ///
    /// `tempfile` is not a dependency of this crate and this phase may not add
    /// one, so the unique name is built here: the process id keeps two
    /// concurrent `cargo test` runs apart, and the counter keeps this
    /// process's own parallel test threads apart. [`crate::bookmarks`]'s
    /// equivalent fixture is a bare path rather than a directory, because it
    /// can unlink the two files it knows about; this one holds a directory so
    /// that nothing in this module, tests included, has to unlink a single
    /// file by path — see [`Downloads::save`] for why that matters here and
    /// not there.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
            let name = format!("talaria-downloads-{}-{unique}", std::process::id());
            let dir = std::env::temp_dir().join(name);
            fs::create_dir_all(&dir).expect("create the test directory");
            Self(dir)
        }

        /// The store's own file.
        fn store(&self) -> PathBuf {
            self.0.join("downloads.json")
        }

        /// The staging file a save writes before renaming, which must never
        /// survive a completed save.
        fn staging(&self) -> PathBuf {
            self.0.join("downloads.json.tmp")
        }

        fn write(&self, contents: &str) {
            fs::write(self.store(), contents).expect("write the test fixture");
        }

        fn read(&self) -> String {
            fs::read_to_string(self.store()).unwrap_or_default()
        }

        fn exists(&self) -> bool {
            self.store().exists()
        }

        /// A real file on disk standing in for a downloaded one, so a test can
        /// assert that removing its list row leaves it alone.
        fn downloaded_file(&self, name: &str) -> PathBuf {
            let path = self.0.join(name);
            fs::write(&path, b"downloaded bytes").expect("write the downloaded fixture");
            path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_missing_file_loads_as_an_empty_store() {
        let temp = TempDir::new();
        let downloads = Downloads::load_from(temp.store());
        assert!(downloads.entries().is_empty());
    }

    #[test]
    fn an_empty_file_loads_as_an_empty_store() {
        let temp = TempDir::new();
        temp.write("");
        let downloads = Downloads::load_from(temp.store());
        assert!(downloads.entries().is_empty());
    }

    #[test]
    fn a_corrupt_file_loads_as_an_empty_store_without_panicking() {
        let temp = TempDir::new();
        temp.write("{not json at all");
        let downloads = Downloads::load_from(temp.store());
        assert!(downloads.entries().is_empty());
    }

    /// A hand-edited file holding the right JSON of the wrong shape (an
    /// object, not an array) is the other half of the corruption case.
    #[test]
    fn a_file_of_the_wrong_shape_loads_as_an_empty_store() {
        let temp = TempDir::new();
        temp.write(r#"{"path":"/tmp/report.pdf"}"#);
        let downloads = Downloads::load_from(temp.store());
        assert!(downloads.entries().is_empty());
    }

    /// An array whose entries are individually broken costs only those
    /// entries. `path` is the only load-bearing field: an entry with no path
    /// names no file, so there is nothing for Open to launch and nothing for
    /// Remove to key on.
    #[test]
    fn an_entry_with_no_path_is_skipped_and_the_good_ones_survive() {
        let temp = TempDir::new();
        temp.write(
            r#"[{"filename":"pathless.pdf","url":"https://example.com/","bytes":1,"completed_at_ms":1},
                {"path":"/tmp/kept.pdf","filename":"kept.pdf","url":"https://example.com/kept.pdf","bytes":2,"completed_at_ms":2}]"#,
        );
        let downloads = Downloads::load_from(temp.store());
        assert_eq!(downloads.entries().len(), 1);
        assert_eq!(downloads.entries()[0].path, "/tmp/kept.pdf");
    }

    /// The list of what landed on the human's disk, and where, is as much a
    /// record of their browsing as `history.jsonl` is. The save renames a
    /// staged sibling into place, and a rename carries the *staged* file's
    /// mode, so this pins the mode being set before the rename.
    #[cfg(unix)]
    #[test]
    fn a_saved_list_lands_owner_only_through_the_rename() {
        let temp = TempDir::new();
        let mut downloads = Downloads::load_from(temp.store());
        downloads.append(
            "/tmp/report.pdf".into(),
            "report.pdf".into(),
            "https://example.com/report.pdf".into(),
            1024,
            42,
            true,
        );

        assert!(temp.store().exists(), "the save did not land");
        assert_eq!(permissions::mode_of(&temp.store()), 0o600);
    }

    #[test]
    fn an_appended_download_survives_a_reload() {
        let temp = TempDir::new();
        let mut downloads = Downloads::load_from(temp.store());
        downloads.append(
            "/tmp/report.pdf".into(),
            "report.pdf".into(),
            "https://example.com/report.pdf".into(),
            1024,
            42,
            true,
        );

        let reloaded = Downloads::load_from(temp.store());
        assert_eq!(reloaded.entries().len(), 1);
        assert_eq!(reloaded.entries()[0].path, "/tmp/report.pdf");
    }

    /// The adjacency edge this plan exists to pin: `create_unique` has already
    /// given two same-named downloads two different paths by the time they
    /// reach this store, and the store must treat them as two rows rather than
    /// deduplicating on the name they share.
    #[test]
    fn two_downloads_of_the_same_filename_are_two_distinct_entries() {
        let temp = TempDir::new();
        let mut downloads = Downloads::load_from(temp.store());
        downloads.append(
            "/tmp/report.pdf".into(),
            "report.pdf".into(),
            "https://example.com/report.pdf".into(),
            10,
            1,
            true,
        );
        downloads.append(
            "/tmp/report (1).pdf".into(),
            "report.pdf".into(),
            "https://example.com/report.pdf".into(),
            20,
            2,
            true,
        );

        let reloaded = Downloads::load_from(temp.store());
        assert_eq!(reloaded.entries().len(), 2, "the second download overwrote the first");
        assert_eq!(reloaded.entries()[0].path, "/tmp/report.pdf");
        assert_eq!(reloaded.entries()[1].path, "/tmp/report (1).pdf");
        assert_eq!(reloaded.entries()[0].bytes, 10, "the first entry's byte count was clobbered");
        assert_eq!(reloaded.entries()[1].bytes, 20);
    }

    #[test]
    fn a_round_trip_preserves_every_field_exactly() {
        let temp = TempDir::new();
        let mut downloads = Downloads::load_from(temp.store());
        downloads.append(
            "/home/someone/Downloads/quarterly (2).pdf".into(),
            "quarterly.pdf".into(),
            "https://example.com/q.pdf?token=abc".into(),
            18_446_744_073_709_551_615,
            1_700_000_000_123,
            true,
        );

        let reloaded = Downloads::load_from(temp.store());
        assert_eq!(
            reloaded.entries(),
            &[DownloadEntry {
                path: "/home/someone/Downloads/quarterly (2).pdf".into(),
                filename: "quarterly.pdf".into(),
                url: "https://example.com/q.pdf?token=abc".into(),
                bytes: 18_446_744_073_709_551_615,
                completed_at_ms: 1_700_000_000_123,
                requested_by_agent: true,
            }],
        );
    }

    #[test]
    fn entries_keep_their_append_order_across_a_reload() {
        let temp = TempDir::new();
        let mut downloads = Downloads::load_from(temp.store());
        // Every one of these claims the same completion millisecond, so an
        // unstable sort on the timestamp would be free to reorder them.
        for index in 0..4u64 {
            downloads.append(
                format!("/tmp/file-{index}.bin"),
                format!("file-{index}.bin"),
                "https://example.com/".into(),
                index,
                7,
                true,
            );
        }

        let reloaded = Downloads::load_from(temp.store());
        let paths: Vec<&str> = reloaded.entries().iter().map(|entry| entry.path.as_str()).collect();
        assert_eq!(
            paths,
            vec!["/tmp/file-0.bin", "/tmp/file-1.bin", "/tmp/file-2.bin", "/tmp/file-3.bin"],
        );
    }

    #[test]
    fn removing_a_present_path_reports_true_and_does_not_survive_a_reload() {
        let temp = TempDir::new();
        let mut downloads = Downloads::load_from(temp.store());
        downloads.append(
            "/tmp/one.bin".into(),
            "one.bin".into(),
            "https://one/".into(),
            1,
            1,
            true,
        );
        downloads.append(
            "/tmp/two.bin".into(),
            "two.bin".into(),
            "https://two/".into(),
            2,
            2,
            true,
        );

        assert!(downloads.remove("/tmp/one.bin"));

        let reloaded = Downloads::load_from(temp.store());
        assert_eq!(reloaded.entries().len(), 1);
        assert_eq!(reloaded.entries()[0].path, "/tmp/two.bin");
        assert_eq!(reloaded.entries()[0].bytes, 2, "the surviving entry was altered");
    }

    #[test]
    fn removing_an_absent_path_reports_false_and_writes_nothing() {
        let temp = TempDir::new();
        let mut downloads = Downloads::load_from(temp.store());
        assert!(!downloads.remove("/tmp/never-downloaded.bin"));
        assert!(!temp.exists(), "a no-op remove wrote the file anyway");
    }

    /// The boundary between "remove from list" and "delete the file". The
    /// panel's hover text promises the former; this is what makes the promise
    /// checkable.
    #[test]
    fn removing_an_entry_leaves_the_downloaded_file_on_disk() {
        let temp = TempDir::new();
        let downloaded = temp.downloaded_file("downloaded.bin");
        let recorded = downloaded.display().to_string();
        let mut downloads = Downloads::load_from(temp.store());
        downloads.append(
            recorded.clone(),
            "downloaded.bin".into(),
            "https://example.com/downloaded.bin".into(),
            16,
            1,
            true,
        );

        assert!(downloads.remove(&recorded));
        assert!(downloaded.exists(), "removing a list row deleted the file on disk");
        assert!(
            Downloads::load_from(temp.store()).entries().is_empty(),
            "the list row survived the remove",
        );
    }

    /// T-03-04-03's other half: the target file only ever changes through a
    /// rename, so a completed save leaves no staging file behind and the
    /// target always parses.
    #[test]
    fn a_save_stages_through_a_temp_file_and_leaves_none_behind() {
        let temp = TempDir::new();
        let mut downloads = Downloads::load_from(temp.store());
        downloads.append(
            "/tmp/one.bin".into(),
            "one.bin".into(),
            "https://one/".into(),
            1,
            1,
            true,
        );

        assert!(temp.exists(), "the save did not land");
        assert!(!temp.staging().exists(), "the save left its staging file behind");
        let parsed: serde_json::Value =
            serde_json::from_str(&temp.read()).expect("the saved file is valid JSON");
        assert!(parsed.is_array(), "the saved file is not an array");
    }

    /// CR-04: the provenance the panel shows has to survive the file, and a
    /// `false` specifically — a store that simply never wrote the key would
    /// pass a `true`-only round trip, because a missing key reads back as
    /// `true` by design.
    #[test]
    fn the_agent_requested_flag_round_trips_in_both_directions() {
        let temp = TempDir::new();
        let mut downloads = Downloads::load_from(temp.store());
        downloads.append(
            "/tmp/agent.bin".into(),
            "agent.bin".into(),
            "https://example.com/agent.bin".into(),
            1,
            1,
            true,
        );
        downloads.append(
            "/tmp/human.bin".into(),
            "human.bin".into(),
            "https://example.com/human.bin".into(),
            2,
            2,
            false,
        );

        let reloaded = Downloads::load_from(temp.store());
        let flags: Vec<bool> =
            reloaded.entries().iter().map(|entry| entry.requested_by_agent).collect();
        assert_eq!(flags, vec![true, false], "the provenance did not survive the file");
    }

    /// A `downloads.json` written before the field existed must still load,
    /// row for row. Its rows read back as agent-requested, which is what they
    /// actually were: `Command::Download` is the control socket's own command
    /// and there is no human-initiated download path that could have written
    /// any of them.
    #[test]
    fn a_row_written_before_the_field_existed_still_loads_and_reads_as_agent_requested() {
        let temp = TempDir::new();
        temp.write(
            r#"[{"path":"/tmp/legacy.pdf","filename":"legacy.pdf",
                 "url":"https://example.com/legacy.pdf","bytes":9,"completed_at_ms":9}]"#,
        );

        let downloads = Downloads::load_from(temp.store());
        assert_eq!(downloads.entries().len(), 1, "a pre-change file was invalidated");
        let entry = &downloads.entries()[0];
        assert_eq!(entry.path, "/tmp/legacy.pdf");
        assert_eq!(entry.bytes, 9);
        assert!(entry.requested_by_agent, "an unattributed row was presented as the human's own");
    }

    /// A stale `.tmp` from a killed process must never be read as data, and
    /// must not stop the next save from landing.
    #[test]
    fn a_stale_staging_file_does_not_block_the_next_save() {
        let temp = TempDir::new();
        fs::write(temp.staging(), b"garbage from a killed process")
            .expect("write the stale staging file");

        let mut downloads = Downloads::load_from(temp.store());
        downloads.append(
            "/tmp/one.bin".into(),
            "one.bin".into(),
            "https://one/".into(),
            1,
            1,
            true,
        );

        let reloaded = Downloads::load_from(temp.store());
        assert_eq!(reloaded.entries().len(), 1);
        assert_eq!(reloaded.entries()[0].path, "/tmp/one.bin");
    }
}

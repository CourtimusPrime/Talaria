//! Agent authorization: the registered OAuth clients this browser knows about
//! and the digests of the tokens they currently hold, stored as one JSON
//! document in `agents.json` beside the other stores.
//!
//! **What it holds.** Two arrays. A client is a registration — an id this
//! browser minted, the name the client asked to be called, its redirect URIs,
//! when it registered and when (if ever) a human approved it. A token record
//! is the SHA-256 of a token this browser issued, plus everything needed to
//! decide whether presenting it should work: which client it belongs to,
//! which rotation family, when it expires, and what it was issued for.
//!
//! **Why it is plaintext.** Only digests are stored. A stolen `agents.json`
//! yields hashes, not credentials — the same argument that lets a password
//! file store hashes — so there is nothing here for encryption to protect.
//! That premise is load-bearing, and it is what lets this store skip both
//! encryption and the OS keychain entirely. It leans on the same thing
//! [`crate::bookmarks`] leans on: the write below goes through
//! [`crate::permissions`], so the file lands at `0600` and its directory at
//! `0700`.
//!
//! **Why it is not in the vault**, which is the obvious place to put anything
//! secret-adjacent and is the wrong one, twice over:
//!
//! - [`crate::vault`] takes its key from the OS keychain, and `Vault::load()`
//!   has a documented startup hang on machines with no session D-Bus. Putting
//!   agent authorization behind that key would make "no session bus" mean "no
//!   agent can connect" — a browser that starts but can never be driven.
//! - The vault is reachable from the agent tool surface: `cookies_read` reads
//!   it. Token material must not live behind a door an agent already holds a
//!   key to.
//!
//! **`client_id` is the first identity in this project a client did not choose
//! for itself.** `ClientMessage::Hello { client }` and the `Session.client`
//! string the Agents view labels sessions with are whatever the peer typed;
//! anything can claim to be anything. `client_id` is minted here, from the
//! operating system's random source, and it is what the Access panel keys on.
//!
//! **`client_name` is still attacker-chosen.** It is stored verbatim — this
//! module does not sanitise it, because a store that quietly rewrote what it
//! was given would make the panel's job harder, not easier. It is handled as
//! an untrusted claim where it is *rendered*: the Access panel's client row,
//! and the consent prompt that asks a human to approve a registration.
//!
//! There is deliberately no control-socket command and no MCP tool that
//! reaches this store. An agent that could read it could enumerate every other
//! agent's token family; an agent that could write it could authorize itself.
//!
//! **About the `dead_code` expectations that used to be below.** They are all
//! gone, and the way they went is the point. 04-05 wired the *reading* half of
//! this module — [`Agents::load`] and [`Agents::lookup`], through
//! [`crate::oauth`]; 04-06 wired registration and the persistence path behind
//! it; 04-07 wired issuance ([`Agents::authorize`], [`Agents::rotate_refresh`]
//! and the helpers only they use); 04-08 wired revocation
//! ([`Agents::revoke_client`] from the Access panel, [`Agents::tokens`] from
//! the revocation endpoint). Each plan's own wiring made the matching
//! expectation *unfulfilled*, which is an error under the same `-D warnings`
//! gate, so every attribute announced itself rather than being hunted for.
//!
//! One item never found a caller: a public single-token `mint`. Three plans in
//! a row re-pointed its expectation at the next one, and 04-08 — the plan that
//! was meant to wire it — deleted it instead. See [`Agents::stage_token`].
//!
//! 04-04 covered that with a single `#[expect(dead_code)]` on the `mod agents;`
//! line, and said in its own summary why that was unsatisfying: a lint
//! expectation is fulfilled by *any* diagnostic in its scope, so one blanket
//! attribute stays quietly fulfilled while a plan wires only half the module.
//! 04-05 deleted it and replaced it with one attribute per still-unreachable
//! item, each naming the plan that will make it reachable. That is what made
//! 04-06's own wiring self-checking: an `expect` whose item has become live is
//! *unfulfilled*, which is an error under the same gate, so each of the twelve
//! attributes it had to delete announced itself rather than being hunted for.
//!
//! The `cfg_attr(not(test), ..)` wrapper is what makes that work: these items
//! are all exercised by this module's own tests, so in a test build they are
//! not dead and an unconditional expectation would be *unfulfilled* — itself
//! an error under the same gate. The expectation therefore applies only to the
//! build where the claim is true.

use std::fs;
use std::path::PathBuf;

use base64::Engine as _;
use rand::RngCore as _;
use sha2::{Digest as _, Sha256};

use crate::permissions;

/// One registered OAuth client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentClient {
    /// The identifier this browser minted at registration, and the identity
    /// of the client: unlike `client_name`, the client did not choose it.
    pub client_id: String,
    /// The display name the client asked to be called. Attacker-chosen,
    /// stored verbatim, and treated as an untrusted claim wherever it is
    /// rendered — see the module header.
    pub client_name: String,
    /// The redirect URIs the client registered.
    pub redirect_uris: Vec<String>,
    /// Unix epoch milliseconds at the moment the registration landed.
    pub registered_at_ms: u64,
    /// Unix epoch milliseconds at the moment a human approved this client, or
    /// `None` for one that registered and was never approved.
    ///
    /// The `None` case is the whole of this field's point. A registration
    /// nobody approved is **inert**: it holds no tokens, it appears in no list
    /// of connected agents, and there is no operation it can be the subject
    /// of. That is half of the answer to a registry an attacker floods — the
    /// other half is the cap on registrations — and it is why flooding buys an
    /// attacker a bounded amount of disk and nothing else.
    pub authorized_at_ms: Option<u64>,
}

impl AgentClient {
    /// This client as the JSON object the stored document holds.
    ///
    /// Built through `serde_json::json!` rather than a `Serialize` derive
    /// because `serde` itself is not a dependency of this crate — only
    /// `serde_json` is. `to_json` and `from_json` sit next to each other so a
    /// field added to one is visibly missing from the other.
    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "client_id": self.client_id,
            "client_name": self.client_name,
            "redirect_uris": self.redirect_uris,
            "registered_at_ms": self.registered_at_ms,
            "authorized_at_ms": self.authorized_at_ms,
        })
    }

    /// One stored object back into a client, or `None` for an object this
    /// module cannot make sense of.
    ///
    /// Only `client_id` is load-bearing: a registration with no id names no
    /// client, can be the subject of no revocation, and is counted as corrupt.
    /// A missing display name is not corruption — it loads as the empty
    /// string, because a client that registered without a name is a client the
    /// human can still see and still revoke.
    fn from_json(value: &serde_json::Value) -> Option<Self> {
        Some(Self {
            client_id: value.get("client_id")?.as_str()?.to_owned(),
            client_name: value
                .get("client_name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            redirect_uris: value
                .get("redirect_uris")
                .and_then(serde_json::Value::as_array)
                .map(|uris| {
                    uris.iter()
                        .filter_map(serde_json::Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
            registered_at_ms: value
                .get("registered_at_ms")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or_default(),
            authorized_at_ms: value.get("authorized_at_ms").and_then(serde_json::Value::as_u64),
        })
    }
}

/// Which half of an issued pair a token record describes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    /// Presented on every request. Short-lived.
    Access,
    /// Presented only to exchange for a new pair. Long-lived, single-use.
    Refresh,
}

impl TokenKind {
    /// The string this kind is stored as.
    fn as_str(self) -> &'static str {
        match self {
            TokenKind::Access => "access",
            TokenKind::Refresh => "refresh",
        }
    }

    /// One stored string back into a kind, or `None` for a value this module
    /// does not know. An unknown kind makes the record corrupt rather than
    /// defaulting to one of the two: guessing here would either promote a
    /// refresh token to an access token or the reverse.
    fn from_str(value: &str) -> Option<Self> {
        match value {
            "access" => Some(TokenKind::Access),
            "refresh" => Some(TokenKind::Refresh),
            _ => None,
        }
    }
}

/// One token this browser issued, identified by its digest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenRecord {
    /// Lowercase hex SHA-256 of the raw token, and the primary key of this
    /// record. The raw token existed exactly once, in the response that
    /// carried it to the client; nothing in this process retains it and
    /// nothing on disk can be turned back into it.
    pub digest: String,
    /// Access or refresh.
    pub kind: TokenKind,
    /// The client this token was issued to.
    pub client_id: String,
    /// Groups an access token and the refresh token issued alongside it, plus
    /// every later rotation of that pair. Revocation and reuse detection both
    /// operate on the family rather than on the individual token: revoking a
    /// client drops every family it owns, and presenting an already-consumed
    /// refresh token drops the family that token belonged to.
    pub family_id: String,
    /// Unix epoch milliseconds after which this token no longer resolves.
    pub expires_at_ms: u64,
    /// The resource identifier this token was issued for, checked on every
    /// use. It is what makes a token issued for something else unusable here,
    /// rather than merely unintended.
    pub audience: String,
    /// The scope string the specification requires a token to carry. Nothing
    /// in this browser branches on it: per-agent permission scoping is out of
    /// scope by design, so there is one scope, and this field exists so the
    /// protocol responses are well-formed rather than so a decision can be
    /// made from it.
    pub scope: String,
    /// Unix epoch milliseconds at the moment a refresh token was exchanged
    /// for a new pair, or `None` for a token that has never been used.
    ///
    /// A consumed record is deliberately kept rather than deleted: it is the
    /// entire reuse-detection window. A refresh token presented a second time
    /// is found here with a consumption time already set, which is what
    /// separates a stolen replay from a first use. An access token is never
    /// consumed — it is presented on every request — so this stays `None` for
    /// [`TokenKind::Access`] and a consumed record never resolves as live.
    pub consumed_at_ms: Option<u64>,
}

impl TokenRecord {
    /// This record as the JSON object the stored document holds. Adjacent to
    /// [`TokenRecord::from_json`] for the reason [`AgentClient::to_json`]
    /// gives.
    ///
    /// Note what is *not* here: there is no token field, and there is no
    /// field a raw token could be reconstructed from.
    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "digest": self.digest,
            "kind": self.kind.as_str(),
            "client_id": self.client_id,
            "family_id": self.family_id,
            "expires_at_ms": self.expires_at_ms,
            "audience": self.audience,
            "scope": self.scope,
            "consumed_at_ms": self.consumed_at_ms,
        })
    }

    /// One stored object back into a token record, or `None` for an object
    /// this module cannot make sense of.
    ///
    /// The digest and the kind are load-bearing, and so is the client id: a
    /// record with no digest matches nothing and can only be noise, a record
    /// with no recognised kind cannot be filtered correctly, and a record
    /// naming no client cannot be revoked with one. A record missing its
    /// expiry loads as epoch — already expired — because the safe reading of
    /// "no stated expiry" is "expired", not "forever".
    fn from_json(value: &serde_json::Value) -> Option<Self> {
        Some(Self {
            digest: value.get("digest")?.as_str()?.to_owned(),
            kind: TokenKind::from_str(value.get("kind")?.as_str()?)?,
            client_id: value.get("client_id")?.as_str()?.to_owned(),
            family_id: value
                .get("family_id")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            expires_at_ms: value
                .get("expires_at_ms")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or_default(),
            audience: value
                .get("audience")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            scope: value
                .get("scope")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            consumed_at_ms: value.get("consumed_at_ms").and_then(serde_json::Value::as_u64),
        })
    }
}

/// What happened when a refresh token was presented.
///
/// Three cases, kept apart on purpose. Collapsing reuse into "unknown" would
/// throw away the only signal that a token was stolen; collapsing it into
/// "rotated" would hand the thief a working pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefreshOutcome {
    /// The presented token was live and has now been consumed. The new pair
    /// travels back to the client in this value and nowhere else.
    Rotated {
        /// The new access token, raw, for this response only.
        access_token: String,
        /// The new refresh token, raw, for this response only.
        refresh_token: String,
    },
    /// The presented token had already been exchanged. Somebody has a copy of
    /// a token the legitimate client already spent, and there is no way to
    /// tell from here which of the two is the legitimate one — so the whole
    /// family is gone, and both have to start again through a human.
    ReuseDetected,
    /// No such refresh token, or it has expired. One answer for both, for the
    /// reason [`Agents::lookup`] gives.
    Unknown,
}

/// The most registrations this store will hold.
///
/// The file cannot grow past this, and an **approved** client is never
/// displaced by a flood — a registration that would push an approved client
/// out is refused instead. That is the property worth keeping and it is kept.
///
/// What this cap does **not** do on its own is protect the *slot*. `/register`
/// is unauthenticated by design and reachable by any local account, so
/// thirty-two POSTs used to fill the registry permanently: registrations past
/// the cap were refused rather than evicted, nothing expired an unapproved
/// one, and the Access panel lists only approved clients, so the human saw
/// "Nothing authorized yet" and had no control to press. Recovery meant
/// quitting the browser and hand-editing `agents.json` (CR-04).
///
/// [`Agents::register`] now displaces the **oldest unapproved** registration
/// when the cap is reached. Of the three candidate fixes that is the one that
/// restores service *immediately*: a TTL alone leaves a legitimate agent
/// locked out until the flood ages away, and a separate bound for unapproved
/// entries has the same displacement behaviour with one more number to reason
/// about. An unapproved registration is inert — it holds no tokens
/// ([`AgentClient::authorized_at_ms`]) — so displacing one costs nothing but
/// a `client_id` nobody ever approved, and the client whose registration went
/// simply registers again.
pub const MAX_REGISTERED_CLIENTS: usize = 32;

/// How long a registration nobody approved is kept, in milliseconds.
///
/// One hour. A registration is one step in a flow that completes in seconds —
/// the client registers, opens `/authorize`, and a human answers inside
/// [`crate::oauth`]'s two-minute consent lifetime. One still unapproved an
/// hour later is abandoned, and keeping it costs a slot a legitimate agent
/// needs.
///
/// This is the tidy-up, not the defence: the defence is the displacement in
/// [`Agents::register`], which does not wait an hour. What the TTL buys is
/// that the count the Access panel shows means something — "3 waiting for
/// approval" should be three programs that are actually waiting, not three
/// from last Tuesday.
pub const UNAPPROVED_REGISTRATION_TTL_MS: u64 = 60 * 60 * 1_000;

/// How long an access token lives by default: one hour.
///
/// A conventional default, not a requirement — the specification says SHOULD
/// be short-lived and says nothing about the number. It is a parameter
/// everywhere below precisely so this constant can change without anything
/// depending on the value it happens to hold.
pub const DEFAULT_ACCESS_TTL_MS: u64 = 60 * 60 * 1_000;

/// How long a refresh token lives by default: thirty days.
///
/// Conventional in the same way and for the same reason. Refresh tokens are
/// rotated on every use, so this bounds an *idle* client rather than an active
/// one: an agent that has not run for a month asks its human again.
pub const DEFAULT_REFRESH_TTL_MS: u64 = 30 * 24 * 60 * 60 * 1_000;

/// The number of raw bytes behind a token. 256 bits of operating-system
/// randomness, which is what makes guessing one uninteresting to attempt.
const TOKEN_BYTES: usize = 32;

/// The number of raw bytes behind a client or family identifier. These are
/// names rather than credentials — knowing one grants nothing — so they are
/// shorter, and long enough not to collide.
const ID_BYTES: usize = 16;

/// `count` bytes from the operating system's random source.
///
/// The same source [`crate::vault`] draws its keys and nonces from, and never
/// a time-seeded or user-space generator: a token an attacker can derive from
/// the moment it was issued is not a token. Nothing in this module reads a
/// clock at all — every time value is a parameter — so there is no clock here
/// to fall back to even by accident.
fn random_bytes(count: usize) -> Vec<u8> {
    let mut bytes = vec![0u8; count];
    let mut rng = rand::rngs::OsRng;
    rng.fill_bytes(&mut bytes);
    bytes
}

/// A fresh opaque token, base64url with no padding so it is clean in an
/// `Authorization` header and in a query string without further escaping.
///
/// Opaque rather than self-contained, per the phase's research: revocation has
/// to bite on the next request, and a self-contained token cannot be revoked
/// without a lookup — at which point the token format has been paid for and
/// the lookup still happens.
fn random_token() -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(random_bytes(TOKEN_BYTES))
}

/// A fresh identifier for a client or a token family.
///
/// Random rather than sequential: a counter would let one client work out how
/// many others exist and guess their identifiers, and this is the one
/// identifier a client does not choose for itself.
fn random_id() -> String {
    hex::encode(random_bytes(ID_BYTES))
}

/// The lowercase hex SHA-256 of a raw token.
///
/// The only place the token-to-digest mapping is written down. Minting and
/// lookup both come through here, so the two cannot drift into disagreeing
/// about what a stored record means — which would present as tokens that were
/// issued and then never worked.
pub(crate) fn digest_of(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

/// Whether two digests are equal, in time that does not depend on where they
/// first differ.
///
/// The obvious version — `left == right`, or a loop that returns on the first
/// mismatched byte — leaks the length of the matching prefix through how long
/// it takes to answer, and a caller who can measure that can recover a valid
/// digest one byte at a time by presenting guesses. So every byte is compared
/// and the results are accumulated, and the answer is only read at the end.
/// This is the one place in this file where the readable version is the
/// insecure one.
///
/// Lengths are compared up front and early-returned on, deliberately: a digest
/// of the wrong length is not a near-miss to be measured, it is a value that
/// was never produced by [`digest_of`], and the length of a SHA-256 digest is
/// not a secret.
pub(crate) fn digests_match(left: &str, right: &str) -> bool {
    let left = left.as_bytes();
    let right = right.as_bytes();
    if left.len() != right.len() {
        return false;
    }
    let mut difference = 0u8;
    for (mine, theirs) in left.iter().zip(right.iter()) {
        difference |= mine ^ theirs;
    }
    difference == 0
}

/// The registered clients and the digests of their live tokens.
pub struct Agents {
    path: PathBuf,
    clients: Vec<AgentClient>,
    tokens: Vec<TokenRecord>,
}

/// Where every store in this crate keeps its file. Each store owns its own
/// copy of these lines rather than sharing a `paths` module: the stores are
/// deliberately independent, and one shared helper is one more thing that has
/// to be right for all of them at once.
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

impl Agents {
    /// Read the store once, at startup.
    ///
    /// Degrade, never abort — and note *which way* it degrades, because there
    /// are two directions and one of them is a vulnerability. A file that
    /// cannot be read, or whose contents are not the document this module
    /// wrote, produces an **empty client set**: every agent has to
    /// re-authorize. It never produces an empty *check*. The distinction is
    /// the difference between a browser that has forgotten who may drive it
    /// and a browser that will let anybody.
    ///
    /// The code below is shaped so the wrong answer is not writable rather
    /// than merely not written: every decision this store makes is a search
    /// of a list, an empty list finds nothing, and there is no
    /// "if the store is empty, allow" branch anywhere for a later edit to
    /// reach for.
    pub fn load() -> Self {
        Self::load_from(config_dir().join("agents.json"))
    }

    pub(crate) fn load_from(path: PathBuf) -> Self {
        let stored: Option<serde_json::Value> =
            fs::read(&path).ok().and_then(|data| serde_json::from_slice(&data).ok());
        let Some(stored) = stored.filter(serde_json::Value::is_object) else {
            // Absent and unreadable are the same answer — an empty client set
            // — but only the second is worth saying out loud, and only when
            // there is something there to have failed on.
            if path.exists() {
                log::warn!(
                    "agents: {} is unreadable; starting with no authorized clients",
                    path.display()
                );
            }
            return Self { path, clients: Vec::new(), tokens: Vec::new() };
        };

        let mut skipped = 0usize;
        let mut clients = Vec::new();
        if let Some(array) = stored.get("clients").and_then(serde_json::Value::as_array) {
            clients.reserve(array.len());
            for value in array {
                match AgentClient::from_json(value) {
                    Some(client) => clients.push(client),
                    None => skipped += 1,
                }
            }
        }
        let mut tokens = Vec::new();
        if let Some(array) = stored.get("tokens").and_then(serde_json::Value::as_array) {
            tokens.reserve(array.len());
            for value in array {
                match TokenRecord::from_json(value) {
                    Some(token) => tokens.push(token),
                    None => skipped += 1,
                }
            }
        }
        if skipped > 0 {
            log::warn!("agents: skipped {skipped} unreadable record(s) in {}", path.display());
        }
        Self { path, clients, tokens }
    }

    /// Every registered client, in the order they registered, borrowed so the
    /// Access panel can list them without cloning.
    ///
    /// The store never reorders itself. Display order is the panel's business
    /// — it renders newest first, the same display-side reversal the history
    /// panel uses — and deliberately not this module's: sorting here by an
    /// authorization time would give two clients approved in the same
    /// millisecond an order that depended on the sort's stability, where
    /// insertion order gives them one that does not depend on the clock at
    /// all.
    pub fn clients(&self) -> &[AgentClient] {
        &self.clients
    }

    /// Every token record. Borrowed, and of interest mainly to the tests and
    /// to whatever has to reason about a family as a whole — which is what the
    /// revocation endpoint does: it is handed one token and has to find the
    /// family that token belongs to before it can drop it.
    pub fn tokens(&self) -> &[TokenRecord] {
        &self.tokens
    }

    /// How many clients are registered.
    #[cfg_attr(not(test), expect(dead_code, reason = "test-only accessor; no production caller"))]
    pub fn len(&self) -> usize {
        self.clients.len()
    }

    /// Whether no client is registered. A valid, ordinary state — the state
    /// every fresh profile starts in and the state a corrupt file degrades to
    /// — and nothing below special-cases it.
    #[cfg_attr(not(test), expect(dead_code, reason = "test-only accessor; no production caller"))]
    pub fn is_empty(&self) -> bool {
        self.clients.is_empty()
    }

    /// Stage one token for a client and keep only its digest, returning the
    /// raw value to the caller — once, here, and never again. Nothing retains
    /// it: it exists in this return value and in the caller that carries it
    /// onward, and after that the only trace of it anywhere is a hash.
    ///
    /// **Private, and single-token minting has no public wrapper on purpose.**
    /// Every path that issues a credential issues a **pair** — an access token
    /// and the refresh token that rotates it — and stages both before one
    /// write. A public single-token `mint` was written in 04-04 and never
    /// called; 04-05, 04-06 and 04-07 each found it still dead and re-pointed
    /// its `expect(dead_code)` at the next plan. 04-08 is the plan that was
    /// meant to wire it, and does not, so it is gone rather than deferred a
    /// fourth time. What replaced it is [`Agents::mint_for_test`], which is
    /// compiled only under `cfg(test)` and cannot be reached for by production
    /// code at all.
    fn stage_token(
        &mut self,
        kind: TokenKind,
        client_id: &str,
        family_id: &str,
        expires_at_ms: u64,
        audience: &str,
        scope: &str,
    ) -> String {
        let token = random_token();
        self.tokens.push(TokenRecord {
            digest: digest_of(&token),
            kind,
            client_id: client_id.to_owned(),
            family_id: family_id.to_owned(),
            expires_at_ms,
            audience: audience.to_owned(),
            scope: scope.to_owned(),
            consumed_at_ms: None,
        });
        token
    }

    /// One staged token, persisted — a **test fixture**, not an API.
    ///
    /// Compiled only under `cfg(test)`, so no production path can call it and
    /// no future edit can start. It exists because the unit tests in this
    /// module and in [`crate::oauth`] need to place a credential of a chosen
    /// kind, family, expiry and audience into the store, and the two
    /// production paths that issue credentials ([`Agents::authorize`] and
    /// [`Agents::rotate_refresh`]) deliberately choose all four themselves.
    #[cfg(test)]
    pub fn mint_for_test(
        &mut self,
        kind: TokenKind,
        client_id: &str,
        family_id: &str,
        expires_at_ms: u64,
        audience: &str,
        scope: &str,
    ) -> String {
        let token = self.stage_token(kind, client_id, family_id, expires_at_ms, audience, scope);
        self.save();
        token
    }

    /// The record behind a presented access token, if it is live right now.
    ///
    /// A live search of the current list, every time. That is the whole
    /// mechanism behind revocation landing on the next request: a record
    /// dropped a microsecond ago is not here to be found. **Nothing may cache
    /// the digest-to-identity mapping past a single request.** A caller that
    /// memoises this answer has broken revocation, and no amount of care
    /// elsewhere fixes it — which is why the warning lives here, next to the
    /// thing being cached, rather than in the caller.
    ///
    /// Unknown, expired, consumed and wrong-kind all produce the same `None`.
    /// A caller that could tell them apart would be an oracle: "expired" tells
    /// an attacker their stolen value was real, which is exactly the bit they
    /// did not have.
    pub fn lookup(&self, digest: &str, now_ms: u64) -> Option<&TokenRecord> {
        self.tokens.iter().find(|record| {
            digests_match(&record.digest, digest)
                && record.kind == TokenKind::Access
                && record.consumed_at_ms.is_none()
                && record.expires_at_ms > now_ms
        })
    }

    /// Record a new client, minting the one identifier it does not get to
    /// choose, and hand back the registration.
    ///
    /// `None` means [`MAX_REGISTERED_CLIENTS`] approved clients are already
    /// registered. Reported to the caller rather than logged and swallowed: a
    /// registration that silently did not happen would present to the human as
    /// an agent that inexplicably cannot connect.
    ///
    /// Two things happen before the cap is consulted, and both exist because
    /// this endpoint is unauthenticated (CR-04):
    ///
    /// 1. Registrations nobody approved and nobody came back for are forgotten
    ///    ([`UNAPPROVED_REGISTRATION_TTL_MS`]).
    /// 2. If the registry is still full, the **oldest unapproved** entry is
    ///    displaced. An approved client is never displaced — a flood cannot
    ///    take a human's decision away — which is why a registry of
    ///    [`MAX_REGISTERED_CLIENTS`] approved clients still refuses.
    ///
    /// The client is **not** authorized by this call. It holds no tokens and
    /// appears in no list of connected agents until a human approves it.
    pub fn register(
        &mut self,
        client_name: String,
        redirect_uris: Vec<String>,
        registered_at_ms: u64,
    ) -> Option<AgentClient> {
        // Abandoned registrations first: a flood from an hour ago is not
        // something a legitimate agent should have to displace one at a time.
        self.clients.retain(|client| {
            client.authorized_at_ms.is_some()
                || registered_at_ms.saturating_sub(client.registered_at_ms)
                    < UNAPPROVED_REGISTRATION_TTL_MS
        });
        if self.clients.len() >= MAX_REGISTERED_CLIENTS {
            // `min_by_key` keeps the first entry on a tie, and the list is in
            // insertion order, so "oldest" is deterministic even when a flood
            // registers many clients within one millisecond.
            let displaceable = self
                .clients
                .iter()
                .enumerate()
                .filter(|(_, client)| client.authorized_at_ms.is_none())
                .min_by_key(|(_, client)| client.registered_at_ms)
                .map(|(index, _)| index);
            match displaceable {
                Some(index) => {
                    // The client id, never the claimed name: the name is
                    // attacker-chosen and this line ends up in a log a human
                    // reads.
                    let displaced = self.clients.remove(index);
                    log::info!(
                        "agents: the registry is full; forgetting the oldest unapproved \
                         registration {}",
                        displaced.client_id
                    );
                },
                None => {
                    log::warn!(
                        "agents: refusing a registration; {MAX_REGISTERED_CLIENTS} \
                         approved clients are already registered"
                    );
                    return None;
                },
            }
        }
        let client = AgentClient {
            client_id: random_id(),
            client_name,
            redirect_uris,
            registered_at_ms,
            authorized_at_ms: None,
        };
        self.clients.push(client.clone());
        self.save();
        Some(client)
    }

    /// Mark a registered client approved and issue its first pair, both halves
    /// sharing a fresh family. Returns the raw access and refresh tokens, in
    /// that order, once.
    ///
    /// `None` means no client with that id is registered — a client cannot be
    /// approved into existence.
    ///
    /// The two lifetimes are parameters rather than literals so a caller can
    /// pin a clock in a test and so the defaults can move; see
    /// [`DEFAULT_ACCESS_TTL_MS`] and [`DEFAULT_REFRESH_TTL_MS`] for what they
    /// are and why they are conventions rather than requirements.
    pub fn authorize(
        &mut self,
        client_id: &str,
        now_ms: u64,
        audience: &str,
        scope: &str,
        access_ttl_ms: u64,
        refresh_ttl_ms: u64,
    ) -> Option<(String, String)> {
        let client = self.clients.iter_mut().find(|client| client.client_id == client_id)?;
        client.authorized_at_ms = Some(now_ms);
        let client_id = client.client_id.clone();
        let family_id = random_id();
        let access = self.stage_token(
            TokenKind::Access,
            &client_id,
            &family_id,
            now_ms.saturating_add(access_ttl_ms),
            audience,
            scope,
        );
        let refresh = self.stage_token(
            TokenKind::Refresh,
            &client_id,
            &family_id,
            now_ms.saturating_add(refresh_ttl_ms),
            audience,
            scope,
        );
        self.save();
        Some((access, refresh))
    }

    /// Exchange a refresh token for a new pair in the same family.
    ///
    /// Rotation on every use is required of public clients, and reuse
    /// detection is its other half: a refresh token presented after it has
    /// already been exchanged means two parties hold it, and nothing here can
    /// tell which one is the legitimate client — so the family is revoked and
    /// both start again through a human. That is the standard mitigation, and
    /// it is cheap when the store is a local file.
    ///
    /// The distinction that keeps this from firing on innocent traffic: a
    /// client *retrying with its current token* has presented a token this
    /// store has never consumed, and that rotates normally. Only a token
    /// already marked consumed is reuse. Consumption and issuance happen in
    /// one mutation below, so two interleaved rotations of the same digest
    /// cannot both come back rotated — the second is unambiguously the
    /// second.
    ///
    /// The family's outstanding access token is deliberately left alone on a
    /// successful rotation: it is short-lived, the client may have a request
    /// in flight with it, and killing it would make every rotation a race.
    pub fn rotate_refresh(
        &mut self,
        digest: &str,
        now_ms: u64,
        access_ttl_ms: u64,
        refresh_ttl_ms: u64,
    ) -> RefreshOutcome {
        let Some(index) = self.tokens.iter().position(|record| {
            digests_match(&record.digest, digest) && record.kind == TokenKind::Refresh
        }) else {
            return RefreshOutcome::Unknown;
        };
        if self.tokens[index].consumed_at_ms.is_some() {
            let family_id = self.tokens[index].family_id.clone();
            log::warn!(
                "agents: a consumed refresh token was presented again; \
                 revoking token family {family_id}"
            );
            self.revoke_family(&family_id);
            return RefreshOutcome::ReuseDetected;
        }
        if self.tokens[index].expires_at_ms <= now_ms {
            return RefreshOutcome::Unknown;
        }
        // Consumed first, then issued, in one mutation and before anything can
        // observe the store again.
        self.tokens[index].consumed_at_ms = Some(now_ms);
        let client_id = self.tokens[index].client_id.clone();
        let family_id = self.tokens[index].family_id.clone();
        let audience = self.tokens[index].audience.clone();
        let scope = self.tokens[index].scope.clone();
        let access_token = self.stage_token(
            TokenKind::Access,
            &client_id,
            &family_id,
            now_ms.saturating_add(access_ttl_ms),
            &audience,
            &scope,
        );
        let refresh_token = self.stage_token(
            TokenKind::Refresh,
            &client_id,
            &family_id,
            now_ms.saturating_add(refresh_ttl_ms),
            &audience,
            &scope,
        );
        self.save();
        RefreshOutcome::Rotated { access_token, refresh_token }
    }

    /// Remove a client's registration and every token it holds, reporting
    /// whether anything was there. Persists only when something actually
    /// changed, so revoking an agent that is already gone writes nothing.
    ///
    /// The registration goes too, not only the tokens. The Access panel's row
    /// *is* the client, and a row that vanished while its registration lingered
    /// would come back at the next authorization without a human having
    /// approved anything — which is the opposite of what pressing revoke means.
    pub fn revoke_client(&mut self, client_id: &str) -> bool {
        let before = self.clients.len() + self.tokens.len();
        self.clients.retain(|client| client.client_id != client_id);
        self.tokens.retain(|record| record.client_id != client_id);
        let changed = self.clients.len() + self.tokens.len() != before;
        if changed {
            self.save();
        }
        changed
    }

    /// Drop every registration a human has not approved, and report how many
    /// went.
    ///
    /// The Access panel's recovery control (CR-04). It exists so that a
    /// registry filled by an unauthenticated flood is something a human can
    /// clear from inside the product rather than by quitting the browser and
    /// editing `agents.json` — which was the only route, because unapproved
    /// registrations render no row and therefore no Revoke button.
    ///
    /// Takes nothing a human approved, so it cannot cost anybody access. What
    /// it can cost is a registration in flight: a client that registered a
    /// moment ago and is waiting at the consent panel has its approval refused
    /// and has to register again. That is the safe direction — the failure is
    /// a refused grant, never a granted one — and it is what the panel's own
    /// wording says the control does.
    pub fn forget_unapproved(&mut self) -> usize {
        let before = self.clients.len();
        self.clients.retain(|client| client.authorized_at_ms.is_some());
        let forgotten = before - self.clients.len();
        if forgotten > 0 {
            self.save();
        }
        forgotten
    }

    /// Drop every token in one rotation family, leaving the registration
    /// alone, and report whether anything was there.
    ///
    /// Used by reuse detection above, and by the single-token revocation
    /// endpoint: revoking one token of a pair has to take its sibling and
    /// every rotation of it, or the client simply refreshes back into having
    /// one.
    pub fn revoke_family(&mut self, family_id: &str) -> bool {
        let before = self.tokens.len();
        self.tokens.retain(|record| record.family_id != family_id);
        let changed = self.tokens.len() != before;
        if changed {
            self.save();
        }
        changed
    }

    /// Write the whole document, atomically.
    ///
    /// Staged into a `.tmp` sibling and moved into place with [`fs::rename`],
    /// which is atomic on the same filesystem, so a process killed mid-save
    /// leaves either the old set or the new one — never a half-written file
    /// the next startup would read as corrupt and drop entirely.
    ///
    /// Every failure here is logged and swallowed: a token that cannot reach
    /// the disk still works in this session, and losing it at the next restart
    /// costs an agent one re-authorization, where taking the browser down over
    /// it costs the human their window.
    fn save(&mut self) {
        let document = serde_json::json!({
            "clients": self.clients.iter().map(AgentClient::to_json).collect::<Vec<_>>(),
            "tokens": self.tokens.iter().map(TokenRecord::to_json).collect::<Vec<_>>(),
        });
        let Ok(data) = serde_json::to_vec(&document) else {
            log::warn!("agents could not be serialised; this change will not survive a restart");
            return;
        };
        // Staged beside the target, never in the temp directory: `fs::rename`
        // is only atomic within one filesystem, and a config directory on a
        // different mount than /tmp is an ordinary setup.
        let Some(name) = self.path.file_name().map(|name| name.to_string_lossy().into_owned())
        else {
            log::warn!("agents path has no file name; not saving");
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
            log::warn!("could not stage agents: {error}");
            return;
        }
        if let Err(error) = fs::rename(&staged, &self.path) {
            log::warn!("could not replace agents with the new set: {error}");
            let _ = fs::remove_file(&staged);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    /// An agents path in the temp directory that removes itself, and the
    /// `.tmp` sibling [`Agents::save`] writes, when a test ends.
    ///
    /// `tempfile` is not a dependency of this crate and this plan may not add
    /// one, so the unique name is built here: the process id keeps two
    /// concurrent `cargo test` runs apart, and the counter keeps this
    /// process's own parallel test threads apart.
    struct TempPath(PathBuf);

    impl TempPath {
        fn new() -> Self {
            let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
            let name = format!("talaria-agents-{}-{unique}.json", std::process::id());
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

        /// Write a fixture, through `File::create` rather than the one-line
        /// whole-file helper in [`std::fs`]. That helper appears nowhere in
        /// this module, not even in a test fixture, so that "the target only
        /// ever changes through a rename" stays a claim a grep can check.
        fn write(&self, contents: &str) {
            let mut file = fs::File::create(&self.0).expect("create the test fixture");
            file.write_all(contents.as_bytes()).expect("write the test fixture");
        }

        fn read_bytes(&self) -> Vec<u8> {
            fs::read(&self.0).unwrap_or_default()
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

    /// A directory in the temp directory that removes itself, for the one
    /// test that has to assert the mode of the directory a store's file sits
    /// in without touching the human's real config directory.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
            let name = format!("talaria-agents-dir-{}-{unique}", std::process::id());
            Self(std::env::temp_dir().join(name))
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn a_client(client_id: &str) -> AgentClient {
        AgentClient {
            client_id: client_id.to_owned(),
            client_name: "Example Agent".to_owned(),
            redirect_uris: vec!["http://127.0.0.1:1234/callback".to_owned()],
            registered_at_ms: 1_000,
            authorized_at_ms: Some(2_000),
        }
    }

    fn a_token(digest: &str) -> TokenRecord {
        TokenRecord {
            digest: digest.to_owned(),
            kind: TokenKind::Access,
            client_id: "client-1".to_owned(),
            family_id: "family-1".to_owned(),
            expires_at_ms: 9_000,
            audience: "http://127.0.0.1:8779/mcp".to_owned(),
            scope: "talaria".to_owned(),
            consumed_at_ms: None,
        }
    }

    #[test]
    fn a_missing_file_loads_as_an_empty_store() {
        let temp = TempPath::new();
        let agents = Agents::load_from(temp.path());
        assert!(agents.is_empty());
        assert_eq!(agents.len(), 0);
        assert!(agents.tokens().is_empty());
    }

    /// T-15, the sharp one. A file that is not JSON at all must land on
    /// *deny*: no clients, and — the half that matters — no tokens either, so
    /// every later search of these lists finds nothing.
    #[test]
    fn an_unparseable_file_loads_as_an_empty_client_set_rather_than_an_empty_check() {
        let temp = TempPath::new();
        temp.write("{not json at all");
        let agents = Agents::load_from(temp.path());
        assert!(agents.clients().is_empty(), "a corrupt file produced a client");
        assert!(agents.tokens().is_empty(), "a corrupt file produced a token");
    }

    /// The other half of corruption: valid JSON of the wrong shape, which is
    /// what a hand-edited file usually is.
    #[test]
    fn a_document_of_the_wrong_shape_loads_as_an_empty_store() {
        let temp = TempPath::new();
        temp.write(r#"["not", "an", "object"]"#);
        let agents = Agents::load_from(temp.path());
        assert!(agents.clients().is_empty());
        assert!(agents.tokens().is_empty());
    }

    /// A truncated file — the shape a process killed mid-write would leave if
    /// the write were not staged and renamed.
    #[test]
    fn a_truncated_file_loads_as_an_empty_store() {
        let temp = TempPath::new();
        temp.write(r#"{"clients":[{"client_id":"c1","client_na"#);
        let agents = Agents::load_from(temp.path());
        assert!(agents.clients().is_empty());
        assert!(agents.tokens().is_empty());
    }

    /// An object with neither key is not corruption — it is an empty store
    /// written by an older build, and it loads as one.
    #[test]
    fn a_document_with_neither_array_loads_as_an_empty_store() {
        let temp = TempPath::new();
        temp.write("{}");
        let agents = Agents::load_from(temp.path());
        assert!(agents.is_empty());
        assert!(agents.tokens().is_empty());
    }

    /// Individually broken records cost only those records, the same degrade
    /// the other stores apply per entry.
    #[test]
    fn a_client_with_no_id_is_skipped_and_the_good_ones_survive() {
        let temp = TempPath::new();
        temp.write(
            r#"{"clients":[{"client_name":"nameless one"},
                           {"client_id":"kept","client_name":"Kept"}]}"#,
        );
        let agents = Agents::load_from(temp.path());
        assert_eq!(agents.clients().len(), 1);
        assert_eq!(agents.clients()[0].client_id, "kept");
    }

    /// The digest is the token record's load-bearing field: a record with no
    /// digest matches nothing and can only be noise.
    #[test]
    fn a_token_with_no_digest_is_skipped() {
        let temp = TempPath::new();
        temp.write(
            r#"{"tokens":[{"kind":"access","client_id":"c1"},
                          {"digest":"abc","kind":"access","client_id":"c1"}]}"#,
        );
        let agents = Agents::load_from(temp.path());
        assert_eq!(agents.tokens().len(), 1);
        assert_eq!(agents.tokens()[0].digest, "abc");
    }

    /// An unrecognised kind is corruption rather than a default: guessing
    /// would either promote a refresh token to an access token or the reverse.
    #[test]
    fn a_token_with_an_unknown_kind_is_skipped() {
        let temp = TempPath::new();
        temp.write(r#"{"tokens":[{"digest":"abc","kind":"bearer","client_id":"c1"}]}"#);
        let agents = Agents::load_from(temp.path());
        assert!(agents.tokens().is_empty());
    }

    /// A missing display name is not corruption — the client is still one a
    /// human can see and revoke.
    #[test]
    fn a_client_with_no_name_loads_with_an_empty_name() {
        let temp = TempPath::new();
        temp.write(r#"{"clients":[{"client_id":"c1"}]}"#);
        let agents = Agents::load_from(temp.path());
        assert_eq!(agents.clients().len(), 1);
        assert_eq!(agents.clients()[0].client_name, "");
        assert_eq!(agents.clients()[0].authorized_at_ms, None);
    }

    /// A record with no stated expiry loads as epoch — already expired —
    /// because the safe reading of "no expiry" is "expired", not "forever".
    #[test]
    fn a_token_with_no_expiry_loads_as_already_expired() {
        let temp = TempPath::new();
        temp.write(r#"{"tokens":[{"digest":"abc","kind":"access","client_id":"c1"}]}"#);
        let agents = Agents::load_from(temp.path());
        assert_eq!(agents.tokens()[0].expires_at_ms, 0);
    }

    #[test]
    fn a_round_trip_preserves_every_field_of_both_record_types() {
        let temp = TempPath::new();
        let mut agents = Agents::load_from(temp.path());
        let client = AgentClient {
            client_id: "client-42".to_owned(),
            client_name: "Some Agent".to_owned(),
            redirect_uris: vec![
                "http://127.0.0.1:1234/callback".to_owned(),
                "http://127.0.0.1:5678/other".to_owned(),
            ],
            registered_at_ms: 111,
            authorized_at_ms: Some(222),
        };
        let token = TokenRecord {
            digest: "0f".repeat(32),
            kind: TokenKind::Refresh,
            client_id: "client-42".to_owned(),
            family_id: "family-9".to_owned(),
            expires_at_ms: 333,
            audience: "http://127.0.0.1:8779/mcp".to_owned(),
            scope: "talaria".to_owned(),
            consumed_at_ms: Some(444),
        };
        agents.clients.push(client.clone());
        agents.tokens.push(token.clone());
        agents.save();

        let reloaded = Agents::load_from(temp.path());
        assert_eq!(reloaded.clients(), &[client]);
        assert_eq!(reloaded.tokens(), &[token]);
    }

    /// An unapproved registration round-trips as unapproved. `None` here is
    /// what makes such a client inert, so it must not come back as a time.
    #[test]
    fn an_unapproved_registration_round_trips_as_unapproved() {
        let temp = TempPath::new();
        let mut agents = Agents::load_from(temp.path());
        let mut client = a_client("c1");
        client.authorized_at_ms = None;
        agents.clients.push(client);
        agents.save();

        let reloaded = Agents::load_from(temp.path());
        assert_eq!(reloaded.clients()[0].authorized_at_ms, None);
    }

    #[test]
    fn an_empty_store_saves_and_reloads_as_empty() {
        let temp = TempPath::new();
        let mut agents = Agents::load_from(temp.path());
        agents.save();

        assert!(temp.exists(), "the save did not land");
        let reloaded = Agents::load_from(temp.path());
        assert!(reloaded.is_empty());
        assert!(reloaded.tokens().is_empty());
    }

    /// T-8. Stored in plaintext on the module header's promise that the file
    /// permissions carry the weight — this is that promise. Because the save
    /// renames a staged sibling into place, and a rename carries the *staged*
    /// file's mode, it also pins the mode being set before the rename rather
    /// than after it.
    #[cfg(unix)]
    #[test]
    fn a_saved_store_lands_owner_only_through_the_rename() {
        let temp = TempPath::new();
        let mut agents = Agents::load_from(temp.path());
        agents.clients.push(a_client("c1"));
        agents.save();

        assert!(temp.exists(), "the save did not land");
        assert_eq!(permissions::mode_of(&temp.path()), 0o600);
    }

    /// The other half of T-8: `0600` inside a `0700` directory. Asserted
    /// against a temp directory created through the same helper `config_dir()`
    /// uses, rather than against the human's real config directory, which a
    /// unit test has no business tightening.
    #[cfg(unix)]
    #[test]
    fn a_saved_store_sits_in_an_owner_only_directory() {
        let dir = TempDir::new();
        permissions::create_dir_owner_only(&dir.0);
        let mut agents = Agents::load_from(dir.0.join("agents.json"));
        agents.clients.push(a_client("c1"));
        agents.save();

        assert_eq!(permissions::mode_of(&dir.0), 0o700);
        assert_eq!(permissions::mode_of(&dir.0.join("agents.json")), 0o600);
    }

    #[test]
    fn a_save_stages_through_a_temp_file_and_leaves_none_behind() {
        let temp = TempPath::new();
        let mut agents = Agents::load_from(temp.path());
        agents.clients.push(a_client("c1"));
        agents.save();

        assert!(temp.exists(), "the save did not land");
        assert!(!temp.temp_sibling().exists(), "the save left its staging file behind");
        let parsed: serde_json::Value =
            serde_json::from_slice(&temp.read_bytes()).expect("the saved file is valid JSON");
        assert!(parsed.is_object(), "the saved document is not an object");
    }

    /// A stale `.tmp` from an interrupted earlier save must not be read as
    /// data, and must not stop the next save from landing.
    #[test]
    fn a_stale_staging_file_does_not_block_the_next_save() {
        let temp = TempPath::new();
        {
            let mut file =
                fs::File::create(temp.temp_sibling()).expect("create the stale staging file");
            file.write_all(b"garbage from a killed process").expect("write it");
        }

        let mut agents = Agents::load_from(temp.path());
        agents.clients.push(a_client("c1"));
        agents.save();

        let reloaded = Agents::load_from(temp.path());
        assert_eq!(reloaded.clients().len(), 1);
        assert_eq!(reloaded.clients()[0].client_id, "c1");
    }

    /// A save that cannot reach the disk is logged and swallowed: the store
    /// keeps working for this session, which is the crate's rule for
    /// user-data problems.
    #[test]
    fn a_save_into_an_unwritable_path_leaves_memory_intact() {
        let temp = TempPath::new();
        // A file, used as if it were a directory: the parent cannot be
        // created and the staging write cannot open.
        temp.write("not a directory");
        let mut agents = Agents::load_from(temp.path().join("nested").join("agents.json"));
        agents.clients.push(a_client("c1"));
        agents.tokens.push(a_token("abc"));
        agents.save();

        assert_eq!(agents.clients().len(), 1, "a failed save emptied the store");
        assert_eq!(agents.tokens().len(), 1, "a failed save dropped a token");
    }

    /// A save replaces the whole document rather than appending to it.
    #[test]
    fn a_save_replaces_the_whole_document() {
        let temp = TempPath::new();
        let mut agents = Agents::load_from(temp.path());
        agents.clients.push(a_client("c1"));
        agents.clients.push(a_client("c2"));
        agents.save();
        agents.clients.retain(|client| client.client_id == "c2");
        agents.save();

        let reloaded = Agents::load_from(temp.path());
        assert_eq!(reloaded.clients().len(), 1, "the removed client is still on disk");
        assert_eq!(reloaded.clients()[0].client_id, "c2");
    }

    // ---- the store's security operations -----------------------------------

    const AUDIENCE: &str = "http://127.0.0.1:8779/mcp";
    const SCOPE: &str = "talaria";
    /// A pinned clock. Nothing in the module reads one, so every test picks
    /// its own moments and the results do not depend on when the suite runs.
    const NOW: u64 = 1_700_000_000_000;

    /// Register a client and approve it, returning its id and its first pair.
    fn authorized(agents: &mut Agents, name: &str, now_ms: u64) -> (String, String, String) {
        let client = agents
            .register(name.to_owned(), vec!["http://127.0.0.1:1234/cb".to_owned()], now_ms)
            .expect("the registration was accepted");
        let (access, refresh) = agents
            .authorize(
                &client.client_id,
                now_ms,
                AUDIENCE,
                SCOPE,
                DEFAULT_ACCESS_TTL_MS,
                DEFAULT_REFRESH_TTL_MS,
            )
            .expect("a registered client can be authorized");
        (client.client_id, access, refresh)
    }

    /// Whether `needle` appears anywhere in `haystack`.
    fn bytes_contain(haystack: &[u8], needle: &[u8]) -> bool {
        !needle.is_empty()
            && haystack.len() >= needle.len()
            && haystack.windows(needle.len()).any(|window| window == needle)
    }

    #[test]
    fn a_minted_token_is_stored_only_as_its_digest() {
        let temp = TempPath::new();
        let mut agents = Agents::load_from(temp.path());
        let token =
            agents.mint_for_test(TokenKind::Access, "c1", "f1", NOW + 1_000, AUDIENCE, SCOPE);

        assert_eq!(agents.tokens().len(), 1);
        assert_eq!(agents.tokens()[0].digest, digest_of(&token));
        assert_ne!(agents.tokens()[0].digest, token, "the raw token was stored");
    }

    /// T-8's sharpest form: whatever else the file holds, it does not hold the
    /// thing that would let somebody drive the browser.
    #[test]
    fn a_raw_token_never_appears_in_the_saved_file() {
        let temp = TempPath::new();
        let mut agents = Agents::load_from(temp.path());
        let (_, access, refresh) = authorized(&mut agents, "Some Agent", NOW);

        let written = temp.read_bytes();
        assert!(!written.is_empty(), "the save did not land");
        assert!(!bytes_contain(&written, access.as_bytes()), "the access token is on disk");
        assert!(!bytes_contain(&written, refresh.as_bytes()), "the refresh token is on disk");
        assert!(bytes_contain(&written, digest_of(&access).as_bytes()), "no digest was stored");
    }

    /// Not a proof of randomness — that lives in the operating system — but it
    /// does catch the failure mode that matters, a mint that returns the same
    /// value twice because it is derived from something that has not changed.
    #[test]
    fn two_mints_never_collide() {
        let temp = TempPath::new();
        let mut agents = Agents::load_from(temp.path());
        let first = agents.mint_for_test(TokenKind::Access, "c1", "f1", NOW + 1, AUDIENCE, SCOPE);
        let second = agents.mint_for_test(TokenKind::Access, "c1", "f1", NOW + 1, AUDIENCE, SCOPE);

        assert_ne!(first, second);
        assert_ne!(agents.tokens()[0].digest, agents.tokens()[1].digest);
    }

    #[test]
    fn a_live_access_token_resolves_to_its_record() {
        let temp = TempPath::new();
        let mut agents = Agents::load_from(temp.path());
        let (client_id, access, _) = authorized(&mut agents, "Some Agent", NOW);

        let record = agents.lookup(&digest_of(&access), NOW + 1).expect("a live token resolves");
        assert_eq!(record.client_id, client_id);
        assert_eq!(record.audience, AUDIENCE);
        assert_eq!(record.scope, SCOPE);
        assert_eq!(record.kind, TokenKind::Access);
    }

    /// Unknown and expired are one answer. A caller that could tell them apart
    /// would confirm for an attacker that a stolen value was once real.
    #[test]
    fn an_expired_token_and_an_unknown_one_are_the_same_answer() {
        let temp = TempPath::new();
        let mut agents = Agents::load_from(temp.path());
        let (_, access, _) = authorized(&mut agents, "Some Agent", NOW);

        let expired = agents.lookup(&digest_of(&access), NOW + DEFAULT_ACCESS_TTL_MS);
        let unknown = agents.lookup(&digest_of("never issued"), NOW + 1);
        assert!(expired.is_none(), "an expired token resolved");
        assert!(unknown.is_none(), "an unknown token resolved");
    }

    /// A refresh token is not an access token, and presenting one where the
    /// other belongs is the same `None` as presenting nothing.
    #[test]
    fn a_refresh_token_does_not_resolve_as_an_access_token() {
        let temp = TempPath::new();
        let mut agents = Agents::load_from(temp.path());
        let (_, _, refresh) = authorized(&mut agents, "Some Agent", NOW);

        assert!(agents.lookup(&digest_of(&refresh), NOW + 1).is_none());
    }

    /// SC 3, at the level this store is responsible for: the lookup is a live
    /// read, so a record dropped a moment ago is not found by the next call.
    /// Nothing is memoised across calls, and there is nothing here that could
    /// be.
    #[test]
    fn a_token_revoked_a_moment_ago_is_not_found_by_the_next_lookup() {
        let temp = TempPath::new();
        let mut agents = Agents::load_from(temp.path());
        let (client_id, access, _) = authorized(&mut agents, "Some Agent", NOW);
        assert!(agents.lookup(&digest_of(&access), NOW + 1).is_some());

        assert!(agents.revoke_client(&client_id));
        assert!(
            agents.lookup(&digest_of(&access), NOW + 1).is_none(),
            "a revoked token still resolved"
        );
    }

    #[test]
    fn the_constant_time_comparison_agrees_with_ordinary_equality() {
        let digest = digest_of("a token");
        assert!(digests_match(&digest, &digest));
        assert!(!digests_match(&digest, &digest_of("another token")));
        assert!(!digests_match(&digest, ""), "a length mismatch compared equal");
        assert!(!digests_match(&digest, &digest[..digest.len() - 1]));
        assert!(digests_match("", ""));
    }

    #[test]
    fn registering_mints_a_client_id_the_caller_did_not_choose() {
        let temp = TempPath::new();
        let mut agents = Agents::load_from(temp.path());
        let client = agents
            .register("Some Agent".to_owned(), vec![], NOW)
            .expect("the registration was accepted");

        assert!(!client.client_id.is_empty());
        assert_ne!(client.client_id, client.client_name);
        assert_eq!(client.registered_at_ms, NOW);
        assert_eq!(client.authorized_at_ms, None, "registration approved the client by itself");
        assert_eq!(agents.clients().len(), 1);
    }

    /// T-11's second half: a registration nobody approved is inert — it holds
    /// no tokens, so the flood buys an attacker nothing even below the cap.
    #[test]
    fn an_unapproved_registration_holds_no_tokens() {
        let temp = TempPath::new();
        let mut agents = Agents::load_from(temp.path());
        agents.register("Some Agent".to_owned(), vec![], NOW).expect("accepted");

        assert!(agents.tokens().is_empty());
    }

    /// Fill the registry with unapproved registrations, the way an
    /// unauthenticated flood does.
    fn flood(agents: &mut Agents, at_ms: u64) {
        while agents.clients().len() < MAX_REGISTERED_CLIENTS {
            let index = agents.clients().len();
            agents
                .register(format!("Flood {index}"), vec![], at_ms)
                .expect("a registration below the cap was refused");
        }
    }

    /// CR-04: an unauthenticated flood cannot lock the registry permanently.
    ///
    /// Thirty-two POSTs used to fill it for good — refused rather than
    /// evicted, no TTL, and no row in the Access panel to revoke — so the only
    /// recovery was to quit the browser and hand-edit `agents.json`. The
    /// oldest **unapproved** entry is now displaced instead, so the very next
    /// legitimate registration succeeds rather than the one an hour from now.
    #[test]
    fn a_flood_of_unapproved_registrations_cannot_lock_out_the_next_one() {
        let temp = TempPath::new();
        let mut agents = Agents::load_from(temp.path());
        flood(&mut agents, NOW);
        let oldest = agents.clients()[0].client_id.clone();

        let legitimate = agents
            .register("A Real Agent".to_owned(), vec![], NOW)
            .expect("a legitimate registration was locked out by a flood");

        assert_eq!(agents.clients().len(), MAX_REGISTERED_CLIENTS, "the cap moved");
        assert!(
            !agents.clients().iter().any(|client| client.client_id == oldest),
            "the oldest unapproved registration was not the one displaced"
        );
        assert!(agents.clients().iter().any(|client| client.client_id == legitimate.client_id));
    }

    /// T-11's surviving half, and the part that must not regress: a flood
    /// cannot take a decision a human already made.
    #[test]
    fn a_flood_never_displaces_an_approved_client_and_a_full_registry_still_refuses() {
        let temp = TempPath::new();
        let mut agents = Agents::load_from(temp.path());
        let approved = agents
            .register("The First".to_owned(), vec![], NOW)
            .expect("the first registration was accepted");
        agents
            .authorize(&approved.client_id, NOW, AUDIENCE, SCOPE, DEFAULT_ACCESS_TTL_MS, DEFAULT_REFRESH_TTL_MS)
            .expect("a human approved it");
        flood(&mut agents, NOW);

        // Ten more, each displacing an unapproved entry and none of them the
        // approved client.
        for index in 0..10 {
            agents
                .register(format!("Later {index}"), vec![], NOW)
                .expect("a registration was refused while unapproved entries remained");
            assert!(
                agents.clients().iter().any(|client| client.client_id == approved.client_id),
                "a flood displaced the client a human had approved"
            );
        }

        // And once every slot holds an approved client there is nothing left
        // to displace, so the refusal is back and reaches the caller.
        for client in agents.clients.iter_mut() {
            client.authorized_at_ms = Some(NOW);
        }
        assert!(
            agents.register("One Too Many".to_owned(), vec![], NOW).is_none(),
            "a registration past a registry of approved clients was accepted"
        );
        assert_eq!(agents.clients().len(), MAX_REGISTERED_CLIENTS);
    }

    /// The tidy-up half: an unapproved registration nobody came back for is
    /// forgotten, so the count the Access panel shows means what it says.
    #[test]
    fn an_unapproved_registration_nobody_came_back_for_expires() {
        let temp = TempPath::new();
        let mut agents = Agents::load_from(temp.path());
        let abandoned = agents.register("Abandoned".to_owned(), vec![], NOW).expect("accepted");
        let approved = agents.register("Approved".to_owned(), vec![], NOW).expect("accepted");
        agents
            .authorize(&approved.client_id, NOW, AUDIENCE, SCOPE, DEFAULT_ACCESS_TTL_MS, DEFAULT_REFRESH_TTL_MS)
            .expect("a human approved it");

        let later = NOW + UNAPPROVED_REGISTRATION_TTL_MS + 1;
        agents.register("Fresh".to_owned(), vec![], later).expect("accepted");

        assert!(
            !agents.clients().iter().any(|client| client.client_id == abandoned.client_id),
            "an unapproved registration outlived its TTL"
        );
        assert!(
            agents.clients().iter().any(|client| client.client_id == approved.client_id),
            "an approved client was expired by an unapproved registration's TTL"
        );
    }

    /// The recovery control the Access panel raises: takes every registration
    /// nobody approved, and nothing else.
    #[test]
    fn forgetting_unapproved_registrations_leaves_approved_clients_and_their_tokens() {
        let temp = TempPath::new();
        let mut agents = Agents::load_from(temp.path());
        let approved = agents.register("Approved".to_owned(), vec![], NOW).expect("accepted");
        agents
            .authorize(&approved.client_id, NOW, AUDIENCE, SCOPE, DEFAULT_ACCESS_TTL_MS, DEFAULT_REFRESH_TTL_MS)
            .expect("a human approved it");
        flood(&mut agents, NOW);

        let forgotten = agents.forget_unapproved();

        assert_eq!(forgotten, MAX_REGISTERED_CLIENTS - 1);
        assert_eq!(agents.clients().len(), 1);
        assert_eq!(agents.clients()[0].client_id, approved.client_id);
        assert_eq!(agents.tokens().len(), 2, "an approved client lost its token pair");
        // It reaches the disk, so quitting the browser does not bring them back.
        let reloaded = Agents::load_from(temp.path());
        assert_eq!(reloaded.clients().len(), 1);
        // And a second press is a no-op that reports so.
        assert_eq!(agents.forget_unapproved(), 0);
    }

    #[test]
    fn authorizing_an_unknown_client_yields_nothing() {
        let temp = TempPath::new();
        let mut agents = Agents::load_from(temp.path());
        let outcome = agents.authorize(
            "not-a-registered-client",
            NOW,
            AUDIENCE,
            SCOPE,
            DEFAULT_ACCESS_TTL_MS,
            DEFAULT_REFRESH_TTL_MS,
        );

        assert!(outcome.is_none(), "an unregistered client was approved into existence");
        assert!(agents.tokens().is_empty());
    }

    #[test]
    fn authorizing_marks_the_client_and_issues_one_family() {
        let temp = TempPath::new();
        let mut agents = Agents::load_from(temp.path());
        let (client_id, access, refresh) = authorized(&mut agents, "Some Agent", NOW);

        assert_eq!(agents.clients()[0].authorized_at_ms, Some(NOW));
        assert_eq!(agents.tokens().len(), 2);
        let access_record = agents.lookup(&digest_of(&access), NOW + 1).expect("live access token");
        let family_id = access_record.family_id.clone();
        assert!(!family_id.is_empty());
        for record in agents.tokens() {
            assert_eq!(record.family_id, family_id, "the pair landed in two families");
            assert_eq!(record.client_id, client_id);
        }
        assert_ne!(access, refresh);
    }

    #[test]
    fn an_authorization_survives_a_reload() {
        let temp = TempPath::new();
        let access = {
            let mut agents = Agents::load_from(temp.path());
            let (_, access, _) = authorized(&mut agents, "Some Agent", NOW);
            access
        };

        let reloaded = Agents::load_from(temp.path());
        assert!(reloaded.lookup(&digest_of(&access), NOW + 1).is_some());
    }

    #[test]
    fn rotating_an_unknown_refresh_digest_is_unknown() {
        let temp = TempPath::new();
        let mut agents = Agents::load_from(temp.path());
        authorized(&mut agents, "Some Agent", NOW);

        let outcome = agents.rotate_refresh(
            &digest_of("never issued"),
            NOW + 1,
            DEFAULT_ACCESS_TTL_MS,
            DEFAULT_REFRESH_TTL_MS,
        );
        assert_eq!(outcome, RefreshOutcome::Unknown);
    }

    #[test]
    fn rotating_an_expired_refresh_token_is_unknown() {
        let temp = TempPath::new();
        let mut agents = Agents::load_from(temp.path());
        let (_, _, refresh) = authorized(&mut agents, "Some Agent", NOW);

        let outcome = agents.rotate_refresh(
            &digest_of(&refresh),
            NOW + DEFAULT_REFRESH_TTL_MS,
            DEFAULT_ACCESS_TTL_MS,
            DEFAULT_REFRESH_TTL_MS,
        );
        assert_eq!(outcome, RefreshOutcome::Unknown);
    }

    /// An access token is not a refresh token either, and presenting one to
    /// the rotation path is not reuse — it is nothing.
    #[test]
    fn rotating_an_access_token_digest_is_unknown() {
        let temp = TempPath::new();
        let mut agents = Agents::load_from(temp.path());
        let (_, access, _) = authorized(&mut agents, "Some Agent", NOW);

        let outcome = agents.rotate_refresh(
            &digest_of(&access),
            NOW + 1,
            DEFAULT_ACCESS_TTL_MS,
            DEFAULT_REFRESH_TTL_MS,
        );
        assert_eq!(outcome, RefreshOutcome::Unknown);
    }

    #[test]
    fn a_rotation_issues_a_new_pair_in_the_same_family() {
        let temp = TempPath::new();
        let mut agents = Agents::load_from(temp.path());
        let (client_id, access, refresh) = authorized(&mut agents, "Some Agent", NOW);
        let family_id = agents.tokens()[0].family_id.clone();

        let outcome = agents.rotate_refresh(
            &digest_of(&refresh),
            NOW + 1,
            DEFAULT_ACCESS_TTL_MS,
            DEFAULT_REFRESH_TTL_MS,
        );
        let RefreshOutcome::Rotated { access_token, refresh_token } = outcome else {
            panic!("a live refresh token did not rotate");
        };

        assert_ne!(access_token, access);
        assert_ne!(refresh_token, refresh);
        let new_record =
            agents.lookup(&digest_of(&access_token), NOW + 2).expect("the new access token is live");
        assert_eq!(new_record.family_id, family_id, "the rotation left the family");
        assert_eq!(new_record.client_id, client_id);
        // The outstanding access token is deliberately untouched: it is short
        // lived and the client may have a request in flight with it.
        assert!(agents.lookup(&digest_of(&access), NOW + 2).is_some());
    }

    /// **Concurrency probe, refresh rotation.** Two rotations of the same
    /// digest: the first rotates, the second is unambiguously reuse, the family
    /// is gone afterwards, and the access token issued into that family stops
    /// resolving.
    #[test]
    fn edge_probe_concurrency_rotating_one_digest_twice_is_rotated_then_reuse_detected() {
        let temp = TempPath::new();
        let mut agents = Agents::load_from(temp.path());
        let (_, _, refresh) = authorized(&mut agents, "Some Agent", NOW);

        let first = agents.rotate_refresh(
            &digest_of(&refresh),
            NOW + 1,
            DEFAULT_ACCESS_TTL_MS,
            DEFAULT_REFRESH_TTL_MS,
        );
        let RefreshOutcome::Rotated { access_token, .. } = first else {
            panic!("the first rotation did not rotate");
        };
        assert!(agents.lookup(&digest_of(&access_token), NOW + 2).is_some());

        let second = agents.rotate_refresh(
            &digest_of(&refresh),
            NOW + 2,
            DEFAULT_ACCESS_TTL_MS,
            DEFAULT_REFRESH_TTL_MS,
        );
        assert_eq!(second, RefreshOutcome::ReuseDetected);
        assert!(agents.tokens().is_empty(), "the family survived a reuse");
        assert!(
            agents.lookup(&digest_of(&access_token), NOW + 3).is_none(),
            "the family's access token still resolved after reuse was detected"
        );
        // And the revocation is on disk, not only in memory.
        let reloaded = Agents::load_from(temp.path());
        assert!(reloaded.tokens().is_empty());
        assert_eq!(reloaded.clients().len(), 1, "reuse detection removed the registration too");
    }

    /// **Concurrency probe, the false-positive half.** A client that rotates,
    /// then rotates again with the token it was just given, is doing exactly
    /// what it is supposed to do and must not be caught by reuse detection.
    #[test]
    fn edge_probe_concurrency_a_client_rotating_its_current_token_is_not_reuse() {
        let temp = TempPath::new();
        let mut agents = Agents::load_from(temp.path());
        let (_, _, refresh) = authorized(&mut agents, "Some Agent", NOW);

        let first = agents.rotate_refresh(
            &digest_of(&refresh),
            NOW + 1,
            DEFAULT_ACCESS_TTL_MS,
            DEFAULT_REFRESH_TTL_MS,
        );
        let RefreshOutcome::Rotated { refresh_token, .. } = first else {
            panic!("the first rotation did not rotate");
        };

        let second = agents.rotate_refresh(
            &digest_of(&refresh_token),
            NOW + 2,
            DEFAULT_ACCESS_TTL_MS,
            DEFAULT_REFRESH_TTL_MS,
        );
        let RefreshOutcome::Rotated { access_token, .. } = second else {
            panic!("rotating the current token was treated as reuse");
        };
        assert!(agents.lookup(&digest_of(&access_token), NOW + 3).is_some());
    }

    #[test]
    fn revoking_a_client_removes_its_registration_and_its_tokens() {
        let temp = TempPath::new();
        let mut agents = Agents::load_from(temp.path());
        let (client_id, _, _) = authorized(&mut agents, "Going Away", NOW);
        let (kept_id, kept_access, _) = authorized(&mut agents, "Staying", NOW);

        assert!(agents.revoke_client(&client_id));
        assert_eq!(agents.clients().len(), 1);
        assert_eq!(agents.clients()[0].client_id, kept_id);
        assert!(agents.lookup(&digest_of(&kept_access), NOW + 1).is_some());
        for record in agents.tokens() {
            assert_ne!(record.client_id, client_id, "a revoked client kept a token");
        }

        let reloaded = Agents::load_from(temp.path());
        assert_eq!(reloaded.clients().len(), 1);
    }

    /// **Adjacency probe, already revoked.** The second call reports that
    /// nothing changed, and — the half worth pinning — writes nothing at all.
    #[test]
    fn edge_probe_adjacency_revoking_twice_reports_changed_then_unchanged_and_writes_nothing() {
        let temp = TempPath::new();
        let mut agents = Agents::load_from(temp.path());
        let (client_id, _, _) = authorized(&mut agents, "Some Agent", NOW);

        assert!(agents.revoke_client(&client_id), "the first revoke reported no change");
        assert!(temp.exists(), "the first revoke did not persist");

        // Remove the file so a second write would be visible as its return.
        fs::remove_file(temp.path()).expect("remove the file between revokes");
        assert!(!agents.revoke_client(&client_id), "revoking a gone client reported a change");
        assert!(!temp.exists(), "a no-op revoke wrote the file anyway");
    }

    /// **Adjacency probe, same display name.** Two clients asking to be called
    /// the same thing are two clients. They are told apart by the id the store
    /// minted, which is the one identifier neither of them chose.
    #[test]
    fn edge_probe_adjacency_two_clients_may_share_a_display_name() {
        let temp = TempPath::new();
        let mut agents = Agents::load_from(temp.path());
        let first = agents.register("Claude".to_owned(), vec![], NOW).expect("the first");
        let second = agents.register("Claude".to_owned(), vec![], NOW).expect("the second");

        assert_eq!(agents.clients().len(), 2, "the second registration replaced the first");
        assert_ne!(first.client_id, second.client_id, "two clients share an id");
        assert_eq!(first.client_name, second.client_name);

        assert!(agents.revoke_client(&first.client_id));
        assert_eq!(agents.clients().len(), 1, "revoking one name-alike took both");
        assert_eq!(agents.clients()[0].client_id, second.client_id);
    }

    /// **Ordering probe, the same millisecond.** Two clients approved at the
    /// identical timestamp both appear, and two consecutive reads agree on
    /// their order — because the order is the order they registered in and
    /// does not come from the clock at all.
    #[test]
    fn edge_probe_ordering_two_clients_authorized_in_the_same_millisecond_both_appear() {
        let temp = TempPath::new();
        let mut agents = Agents::load_from(temp.path());
        let (first_id, _, _) = authorized(&mut agents, "First", NOW);
        let (second_id, _, _) = authorized(&mut agents, "Second", NOW);

        assert_eq!(agents.clients().len(), 2, "one authorization displaced the other");
        assert_eq!(agents.clients()[0].authorized_at_ms, agents.clients()[1].authorized_at_ms);

        let read_once: Vec<&str> =
            agents.clients().iter().map(|client| client.client_id.as_str()).collect();
        let read_twice: Vec<&str> =
            agents.clients().iter().map(|client| client.client_id.as_str()).collect();
        assert_eq!(read_once, read_twice, "two reads disagreed about the order");
        assert_eq!(read_once, vec![first_id.as_str(), second_id.as_str()]);

        let reloaded = Agents::load_from(temp.path());
        let after_reload: Vec<&str> =
            reloaded.clients().iter().map(|client| client.client_id.as_str()).collect();
        assert_eq!(after_reload, read_once, "a reload reordered two same-millisecond clients");
    }

    /// **Empty-store probe.** Every operation against a store with no clients,
    /// with no special case anywhere and — the part that is the security
    /// property — no operation succeeding by default.
    #[test]
    fn edge_probe_empty_store_answers_every_operation_without_a_special_case() {
        let temp = TempPath::new();
        let mut agents = Agents::load_from(temp.path());

        assert!(agents.is_empty());
        assert_eq!(agents.len(), 0);
        assert!(agents.clients().is_empty());
        assert!(agents.tokens().is_empty());
        assert!(agents.lookup(&digest_of("anything at all"), NOW).is_none());
        assert!(agents.lookup("", NOW).is_none(), "an empty digest resolved");
        assert!(!agents.revoke_client("nobody"));
        assert!(!agents.revoke_family("no-such-family"));
        assert_eq!(
            agents.rotate_refresh(
                &digest_of("anything at all"),
                NOW,
                DEFAULT_ACCESS_TTL_MS,
                DEFAULT_REFRESH_TTL_MS,
            ),
            RefreshOutcome::Unknown
        );
        assert!(agents
            .authorize("nobody", NOW, AUDIENCE, SCOPE, DEFAULT_ACCESS_TTL_MS, DEFAULT_REFRESH_TTL_MS)
            .is_none());
        assert!(!temp.exists(), "an empty store wrote a file without being asked to");
    }

    /// The same probe against a store that degraded from a corrupt file rather
    /// than from an absent one. This is T-15 stated as a behaviour rather than
    /// as a count: the damaged file admits nobody.
    #[test]
    fn a_store_degraded_from_a_corrupt_file_admits_nobody() {
        let temp = TempPath::new();
        temp.write("{not json at all");
        let agents = Agents::load_from(temp.path());

        assert!(agents.lookup(&digest_of("a token that used to work"), NOW).is_none());
        assert!(agents.lookup("", NOW).is_none());
        assert!(agents.clients().is_empty());
    }

    #[test]
    fn revoking_a_family_leaves_another_familys_tokens_alone() {
        let temp = TempPath::new();
        let mut agents = Agents::load_from(temp.path());
        let (client_id, first_access, _) = authorized(&mut agents, "Some Agent", NOW);
        let (second_access, _) = agents
            .authorize(
                &client_id,
                NOW + 1,
                AUDIENCE,
                SCOPE,
                DEFAULT_ACCESS_TTL_MS,
                DEFAULT_REFRESH_TTL_MS,
            )
            .expect("a second authorization of the same client");
        let first_family = agents
            .lookup(&digest_of(&first_access), NOW + 2)
            .expect("the first family's access token")
            .family_id
            .clone();

        assert!(agents.revoke_family(&first_family));
        assert!(agents.lookup(&digest_of(&first_access), NOW + 2).is_none());
        assert_eq!(agents.tokens().len(), 2, "the second family was taken too");
        assert_eq!(agents.clients().len(), 1, "a family revocation removed the registration");
        assert!(!second_access.is_empty());
    }

    #[test]
    fn revoking_an_absent_family_reports_no_change_and_writes_nothing() {
        let temp = TempPath::new();
        let mut agents = Agents::load_from(temp.path());

        assert!(!agents.revoke_family("no-such-family"));
        assert!(!temp.exists(), "a no-op family revoke wrote the file anyway");
    }
}

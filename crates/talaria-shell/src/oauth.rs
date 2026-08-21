//! OAuth 2.1 for the remote MCP transport — the **resource server** half.
//!
//! **The roles first, because the mechanics only make sense after them.**
//! Talaria is an OAuth 2.1 *resource server*: it holds something an agent
//! wants (the human's logged-in browsing session) and it demands a bearer
//! token before handing any of it over. It also co-hosts its own
//! *authorization server*: the thing that mints those tokens runs in this same
//! process, on the same loopback port. The MCP authorization specification
//! explicitly permits one process to be both, and for a local, single-user
//! browser it is the only arrangement that makes sense.
//!
//! **Both halves live here now.** The resource server — verification,
//! audience binding, and the RFC 9728 protected-resource metadata document
//! that tells a client where to go for a token — was written first, in 04-05.
//! The authorization server grew beside it: `/register` and `/authorize` with
//! the consent gate in 04-06, and `/token` in 04-07. Only `/revoke` is still
//! to come, in 04-08. Each plan extended [`TalariaAuth`] rather than replacing
//! it, which is why the two roles share one provider and one store.
//!
//! ## The alternative that was rejected
//!
//! Delegating to an external identity provider is what the SDK gives away
//! free: `rust_mcp_sdk::auth::RemoteAuthProvider` exists, points at a hosted
//! issuer, and would have made this file a few lines of configuration. It is
//! rejected for two reasons, and it is worth writing them down because it is
//! the path of least engineering resistance and somebody will propose it
//! again.
//!
//! - **It contradicts the product.** Talaria is positioned on no cloud
//!   dependency and plain local files. Requiring a SaaS identity provider —
//!   or a locally-run one, which for the usual choices means running a service
//!   substantially larger than Talaria itself — before an agent may connect
//!   would make "open the browser and connect your agent" into "first, stand
//!   up an identity provider".
//! - **There is nobody to authenticate as.** This is single-user software.
//!   There is no organisation to model, no directory, no groups and no roles.
//!   The human at the keyboard is the resource owner, and they are
//!   authenticated by the fact that they have the window. What an external
//!   identity provider is *for* is a question this product does not ask.
//!
//! ## What is enforced here
//!
//! - **Verification is a live read of [`crate::agents`], every request.** No
//!   digest-to-identity map exists in this file and none may be added: a
//!   cached answer would keep a revoked token working until whatever cached it
//!   expired, which is exactly the failure Success Criterion 3 forbids. See
//!   [`Agents::lookup`]'s own doc comment — this module is the caller it
//!   addresses.
//! - **Audience binding (RFC 8707).** A token records the resource identifier
//!   it was issued for, and verification compares that against
//!   [`canonical_resource`] byte for byte. Without it, a token minted by some
//!   other authorization server for some other service would be accepted here
//!   and Talaria would be a confused deputy. Talaria also never *forwards* a
//!   token it received: the only outbound HTTP client in this process is the
//!   `download` tool's, and it never sees an `Authorization` header.
//! - **One refusal for every failure.** Unknown, expired, consumed,
//!   wrong-kind and wrong-audience all produce [`REFUSAL`] and nothing else. A
//!   caller who could tell them apart would have an oracle for which tokens
//!   exist, and "expired" in particular tells a thief that the value they hold
//!   was real.
//! - **No token material is ever logged.** [`digest_prefix`] is the only way a
//!   log line in this module identifies a token; a caller is named by its
//!   `client_id`. See that function's doc comment for the rule.
//! - **Nothing on the request path panics.** A malformed `Authorization`
//!   header is attacker input. Every path here returns a status code.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use rand::RngCore as _;
use rust_mcp_sdk::auth::{
    AuthInfo, AuthProvider, AuthenticationError, Audience, OauthEndpoint,
    OAUTH_PROTECTED_RESOURCE_BASE, WELL_KNOWN_OAUTH_AUTHORIZATION_SERVER,
};
use rust_mcp_sdk::mcp_http::http::{
    header, HeaderMap, HeaderName, HeaderValue, Method, Request, Response, StatusCode,
};
use rust_mcp_sdk::mcp_http::{GenericBody, GenericBodyExt, McpAppState, McpHttpError};
use url::Url;

use crate::agents::{
    digest_of, digests_match, Agents, RefreshOutcome, DEFAULT_ACCESS_TTL_MS,
    DEFAULT_REFRESH_TTL_MS,
};

/// The one scope this browser issues, requires and understands.
///
/// One scope, deliberately. `SECURITY.md` states that per-agent permission
/// scoping is out of scope by design: Talaria is infrastructure, not a policy
/// layer, and the access-control decision is binary — an agent that connects
/// gets the tool surface. The field exists because the protocol's responses
/// are not well-formed without it, not because a decision is made from it. Do
/// not grow this into a permission matrix without changing `SECURITY.md`
/// first; the document is the design, and this constant follows it.
pub const TALARIA_SCOPE: &str = "talaria:drive";

/// The single message every refused verification carries.
///
/// It is one constant precisely so unknown, expired, consumed, wrong-kind and
/// wrong-audience are **indistinguishable** to the caller: the SDK renders
/// this string into the `error_description` of a `401` body and into the
/// `WWW-Authenticate` challenge, so one constant means one byte-identical
/// refusal. `&'static str` is not a limitation to work around here — it is
/// what makes a runtime-formatted reason (the thing that would leak) awkward
/// to write by accident.
const REFUSAL: &str = "the presented credential is not accepted";

/// How many hex characters of a digest a log line may carry.
///
/// Eight: enough to correlate two lines about the same token while reading a
/// log, far too few to be presented as one.
const DIGEST_PREFIX_LEN: usize = 8;

/// The store, shared by the winit main thread and the `talaria-http` thread.
///
/// **One store, not two.** The Access panel revokes and the verifier reads,
/// and they run on different threads; two independently-loaded [`Agents`]
/// values would give them different answers, so a revocation a human just
/// performed would be invisible to the listener until a restart. That is a
/// security bug with a friendly face, and the type is what prevents it.
///
/// A `std::sync::Mutex` rather than an async one on purpose: every critical
/// section here is a single synchronous operation over a `Vec` of a handful of
/// records, with **no `await` inside it**. An async mutex would invite one.
pub type SharedAgents = Arc<Mutex<Agents>>;

/// The canonical resource identifier for a listener bound at `bound`.
///
/// RFC 8707 audience validation needs exactly one string that names this
/// server, and it is compared **byte for byte** — so this function is the only
/// place it is constructed. A second construction site would be a way for a
/// token to validate against one spelling of this server and not another
/// (`127.0.0.1` against `localhost`, a trailing slash against none), which
/// presents as tokens that were issued and then mysteriously never worked.
///
/// It is the MCP endpoint's own URL, which is what the specification names as
/// the canonical URI of an MCP server, and it is derived from the address the
/// listener *actually bound* — `Shared::remote`'s single source of truth —
/// rather than from the configured port, so the metadata document, the token
/// records and the URL a human was given cannot disagree.
pub fn canonical_resource(bound: &str) -> String {
    format!("http://{bound}/mcp")
}

/// The absolute URL of the protected-resource metadata document.
///
/// Absolute, because it goes into the `WWW-Authenticate` challenge on a `401`
/// and a client that only knows the endpoint URL has to be able to follow it
/// without guessing a base.
pub fn metadata_url(bound: &str) -> String {
    format!("http://{bound}{OAUTH_PROTECTED_RESOURCE_BASE}")
}

/// A short, non-reversible way for a log line to name a token.
///
/// **The rule this exists to make easy: a raw token, and the value of an
/// `Authorization` header, must never be interpolated into a log message.** A
/// log is a durable artifact that anything able to read the log can read, and
/// a token in one is a credential lying in a file. Every log call in this
/// module identifies a caller by its `client_id` and a token by this prefix of
/// its digest — never by the token, and never by the whole digest either,
/// since a full digest is enough to search a stolen `agents.json` with.
///
/// The rule lives here, next to the tool that satisfies it, so that a reviewer
/// who finds this function has also found the rule.
fn digest_prefix(digest: &str) -> &str {
    match digest.char_indices().nth(DIGEST_PREFIX_LEN) {
        Some((index, _)) => &digest[..index],
        None => digest,
    }
}

/// Now, in Unix epoch milliseconds.
///
/// A clock that cannot read itself yields `0`, which reads every stored expiry
/// as being in the future — so this saturates to [`u64::MAX`] instead, making
/// every token expired and every request refused. The degrade direction is the
/// whole point: a browser whose clock is broken refuses agents rather than
/// admitting them. [`crate::agents`] deliberately contains no clock at all, so
/// this is the one in the authorization path.
fn now_ms() -> u64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(since) => u64::try_from(since.as_millis()).unwrap_or(u64::MAX),
        Err(_) => u64::MAX,
    }
}

/// A stored expiry as the [`SystemTime`] the SDK's middleware re-checks.
///
/// `None` when the value cannot be represented, which the middleware treats as
/// an invalid token — the safe direction, and unreachable in practice for any
/// expiry this browser minted.
fn expiry_at(expires_at_ms: u64) -> Option<SystemTime> {
    UNIX_EPOCH.checked_add(Duration::from_millis(expires_at_ms))
}

// ---------------------------------------------------------------------------
// The authorization server: paths, timings, the consent gate, and the
// authorization-code store's write side.
// ---------------------------------------------------------------------------

/// The authorization endpoint's path.
const AUTHORIZATION_PATH: &str = "/authorize";

/// The path the holding page refreshes itself to while it waits.
///
/// A **second path on the same endpoint kind**, not a second endpoint kind:
/// the SDK builds its auth router by folding over the *keys* of the map
/// [`AuthProvider::auth_endpoints`] returns, so a key is a route, and the verb
/// table `validate_allowed_methods` carries for the authorization endpoint
/// (`GET`, `HEAD`, `OPTIONS`) is exactly the one a status poll wants. There is
/// no `OauthEndpoint` variant for a poll, and inventing one would mean forking
/// the SDK's enum for a path only this browser serves.
const AUTHORIZATION_STATUS_PATH: &str = "/authorize/status";

/// The token endpoint's path.
const TOKEN_PATH: &str = "/token";

/// The one PKCE challenge method this browser advertises, accepts at the
/// authorization endpoint, records on a code, and re-checks at redemption.
///
/// One constant behind all four, for the reason [`canonical_resource`] gives
/// about strings that are compared rather than merely printed: an
/// authorization server whose metadata and whose verification disagreed about
/// the spelling would advertise a method it then refused. `plain` has no
/// constant here and no match arm anywhere — `04-RESEARCH.md`'s Pitfall 5
/// names an arm for it as the warning sign, because such an arm is one edit
/// away from being an arm that accepts it.
const CHALLENGE_METHOD: &str = "S256";

/// The grant type that exchanges an authorization code for a token pair.
const AUTHORIZATION_CODE_GRANT: &str = "authorization_code";

/// The grant type that rotates a refresh token into a new pair.
const REFRESH_TOKEN_GRANT: &str = "refresh_token";

/// The most bytes a token request body may carry.
///
/// A token request is five short form parameters. Eight kilobytes is generous
/// for the largest legitimate one and small enough that this endpoint cannot
/// be made to parse a megabyte of form data on an unauthenticated path.
/// `rust-mcp-axum`'s own 4 MiB body cap (`04-02-SPIKE.md`) is the right order
/// of magnitude for an MCP payload and the wrong one for this.
const MAX_TOKEN_REQUEST_BYTES: usize = 8 * 1024;

/// The single description every failed grant carries.
///
/// One constant for the same reason [`REFUSAL`] is one: an unknown code, an
/// expired one, a wrong verifier, a mismatched client, a mismatched redirect
/// URI, an unknown refresh token and a revoked family must all be *one*
/// answer, or the endpoint is an oracle for which credentials exist and which
/// families have been caught. See [`grant_refused`].
const GRANT_REFUSAL: &str = "the presented grant is not accepted";

/// The dynamic client registration endpoint's path (RFC 7591).
const REGISTRATION_PATH: &str = "/register";

/// The token revocation endpoint's path (RFC 7009).
///
/// The one endpoint on this server whose *answer* is fixed before the request
/// is read. See [`TalariaAuth::handle_revocation`].
const REVOCATION_PATH: &str = "/revoke";

/// How long a parked consent request waits for a human, in milliseconds.
///
/// Two minutes. An invented anti-harassment cap rather than anything the
/// specification asks for: it is how long somebody who has just switched
/// applications and is reading seven grant bullets plausibly needs.
const CONSENT_LIFETIME_MS: u64 = 120_000;

/// How long `/authorize` refuses after a human pressed Deny, in milliseconds.
///
/// Ten seconds. A denial is evidence the human *is* at the keyboard, so the
/// shorter of the two cooldowns is enough to stop a spam loop without making a
/// legitimate second attempt feel broken.
const DENY_COOLDOWN_MS: u64 = 10_000;

/// How long `/authorize` refuses after a request expired unanswered.
///
/// Thirty seconds, deliberately longer than [`DENY_COOLDOWN_MS`]. An expiry is
/// evidence nobody is at the keyboard, so re-raising immediately can only hold
/// the displayed page off screen behind a panel nobody is going to answer.
const EXPIRY_COOLDOWN_MS: u64 = 30_000;

/// The shortest any consent duration may be made, in milliseconds.
///
/// A **non-zero** floor, which is the property that matters: it is what stops
/// an override turning a cooldown off or collapsing the parked-request
/// lifetime to nothing.
const CONSENT_FLOOR_MS: u64 = 200;

/// How long a minted authorization code may be redeemed for, in milliseconds.
///
/// Sixty seconds — the conventional value, and deliberately **not**
/// overridable. A code's lifetime is not something a test has to wait out: the
/// suite redeems immediately, so shortening it would buy nothing and would add
/// a fourth knob to a mechanism whose whole defence is how few it has.
const AUTHORIZATION_CODE_LIFETIME_MS: u64 = 60_000;

/// The variable that gates every test-only behaviour in this browser.
///
/// A named constant rather than a literal because this module's own tests name
/// it too, and because "the gate is read at exactly one point" is a property a
/// reviewer should be able to confirm by finding one occurrence.
const TEST_HOOKS_VAR: &str = "TALARIA_TEST_HOOKS";

/// The three consent durations, resolved for one flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ConsentTimings {
    /// How long a parked request waits for a human.
    lifetime_ms: u64,
    /// How long `/authorize` refuses after a denial.
    deny_cooldown_ms: u64,
    /// How long `/authorize` refuses after an expiry.
    expiry_cooldown_ms: u64,
}

impl ConsentTimings {
    /// The values compiled into this build, before any override is consulted.
    const COMPILED: Self = Self {
        lifetime_ms: CONSENT_LIFETIME_MS,
        deny_cooldown_ms: DENY_COOLDOWN_MS,
        expiry_cooldown_ms: EXPIRY_COOLDOWN_MS,
    };
}

/// One override, clamped so it can only ever make a window **smaller**.
///
/// The compiled constant is the ceiling and [`CONSENT_FLOOR_MS`] is the floor,
/// so an over-large value yields the constant, a zero yields the floor, and a
/// value that does not parse — a negative number, a word, an empty string —
/// yields the constant untouched. That last direction is the same one
/// `TALARIA_MAX_DOWNLOAD_BYTES` established in 02-04: a value nobody can read
/// degrades to the safe default, never to zero and never to unbounded.
///
/// Separate from [`consent_timings`] so it can be exercised directly: a test
/// that had to set an environment variable to check a clamp would be testing
/// the process's environment rather than the clamp.
fn shorten(compiled_ms: u64, raw: Option<String>) -> u64 {
    match raw.and_then(|value| value.trim().parse::<u64>().ok()) {
        // `max` then `min` rather than `clamp`, which panics when the floor
        // exceeds the ceiling. It cannot here, and a panic on a network-facing
        // path is not something to leave one constant edit away.
        Some(value) => value.max(CONSENT_FLOOR_MS).min(compiled_ms),
        None => compiled_ms,
    }
}

/// The consent durations for this flow — the compiled constants, optionally
/// shortened.
///
/// **Why this knob exists.** The production values above are correct for a
/// human and ruinous for a test: an end-to-end suite that must observe an
/// expiry and then the cooldown that expiry arms would spend two and a half
/// minutes on one assertion, in a suite that runs sequentially on a single
/// self-hosted runner. An assertion that slow is an assertion somebody
/// quietly deletes later, so the expiry path — the one an attacker uses to
/// hold the chrome hostage — would end up the least-tested thing here.
///
/// **Why it is not a bypass, stated as three properties rather than as a
/// promise.**
///
/// 1. **Durations only.** There is no override that *answers* a request. The
///    approve branch reads no environment variable at all, and a future edit
///    that added one there would be exactly the bypass this paragraph exists
///    to prevent — say so in review rather than merging it.
/// 2. **Shortening only.** Every value goes through [`shorten`], which clamps
///    to the compiled constant as a ceiling and to a non-zero floor. An
///    override cannot lengthen a security timeout, cannot zero one, and cannot
///    switch a cooldown off.
/// 3. **Gated, and read once.** The gate is the same one
///    `Command::ChromeRects` uses: read here, at the single point in this file
///    that consults the environment at all, and inert in any build the
///    variable is not set for. When it is not set this function reads nothing
///    else and returns the constants untouched.
///
/// Shortening how long a human has to answer is not answering. The grant still
/// requires that human's click on a native control the network cannot reach —
/// which is what a unit test pins by setting every override to its floor and
/// confirming an unanswered request still resolves as *denied*.
fn consent_timings() -> ConsentTimings {
    if std::env::var(TEST_HOOKS_VAR).as_deref() != Ok("1") {
        return ConsentTimings::COMPILED;
    }
    ConsentTimings {
        lifetime_ms: shorten(
            CONSENT_LIFETIME_MS,
            std::env::var("TALARIA_CONSENT_LIFETIME_MS").ok(),
        ),
        deny_cooldown_ms: shorten(
            DENY_COOLDOWN_MS,
            std::env::var("TALARIA_CONSENT_DENY_COOLDOWN_MS").ok(),
        ),
        expiry_cooldown_ms: shorten(
            EXPIRY_COOLDOWN_MS,
            std::env::var("TALARIA_CONSENT_EXPIRY_COOLDOWN_MS").ok(),
        ),
    }
}

/// What a human pressed on the consent panel.
///
/// Two variants and no third: there is no "later", no "always allow", and no
/// value an environment variable or a control-socket command can produce.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConsentDecision {
    Approve,
    Deny,
}

/// One authorization request parked on the main thread, waiting for a human.
///
/// Constructed on the `talaria-http` thread once every parameter the caller
/// supplied has been validated, carried across by [`AppEvent::ConsentRequested`],
/// and read by the consent panel each frame the way every other panel reads
/// its store.
///
/// **What it does not carry is the point.** No authorization code, no token
/// and no PKCE verifier: the code is minted on the HTTP side *after* the
/// decision arrives, so no secret has a route into the chrome at all.
pub struct ConsentRequest {
    /// This browser's own identifier for the parked request. Random rather
    /// than sequential, because the holding page's status poll is keyed on it
    /// and a counter would let one caller poll another's request.
    pub id: u64,
    /// The identifier this browser minted at registration — the one the client
    /// did not choose for itself.
    pub client_id: String,
    /// The display name the client asked to be called. **Attacker-chosen**,
    /// and rendered as a fenced, labelled claim rather than as a fact.
    pub client_name: String,
    /// The redirect URI, already checked against the registration.
    pub redirect_uri: String,
    /// Unix epoch milliseconds at the moment the request was raised, which is
    /// what the panel's arm delay is measured from. The timing state lives
    /// here rather than on the chrome so a repaint cannot restart the clock.
    pub raised_at_ms: u64,
    /// Where the decision goes. Private: the only way to answer a request is
    /// to consume it through [`ConsentRequest::resolve`], so a decision cannot
    /// be sent twice and a request cannot be left half-answered.
    reply: tokio::sync::oneshot::Sender<ConsentDecision>,
}

/// Hand-written because a `oneshot::Sender` is not `Debug`, and
/// [`AppEvent`] is — the same trick `AgentRequest`'s own `Debug` uses in
/// `crate::app`. The sender is omitted rather than described: there is nothing
/// useful to say about a channel end in a log line.
impl std::fmt::Debug for ConsentRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConsentRequest")
            .field("id", &self.id)
            .field("client_id", &self.client_id)
            .field("raised_at_ms", &self.raised_at_ms)
            .finish_non_exhaustive()
    }
}

impl ConsentRequest {
    /// Answer this request, consuming it.
    ///
    /// A dropped send is deliberately ignored: the HTTP side gives up on the
    /// parked-request lifetime and drops its receiver, and a human who presses
    /// Approve one millisecond after that has pressed a button on a request
    /// that no longer exists. The panel closes either way.
    pub fn resolve(self, decision: ConsentDecision) {
        let _ = self.reply.send(decision);
    }

    /// Whether the HTTP side has stopped waiting — it timed out and dropped
    /// its receiver.
    ///
    /// This is the cancellation handle 02-04 established and 04-03 reuses for
    /// shutdown, in its third use: the panel checks it on the repaint it
    /// already schedules and closes itself, which is the third of its three
    /// exits (Approve, Deny, and the HTTP side giving up).
    pub fn is_abandoned(&self) -> bool {
        self.reply.is_closed()
    }
}

/// Where a consent request has got to, as the holding page reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConsentStatus {
    Pending,
    Approved,
    Denied,
    Expired,
}

impl ConsentStatus {
    /// The one word the status response carries. `OauthEndpoint` has no
    /// `Debug` and neither does this rely on one: the wire name is written
    /// down here rather than derived, so it cannot change under a rename.
    fn as_str(self) -> &'static str {
        match self {
            ConsentStatus::Pending => "pending",
            ConsentStatus::Approved => "approved",
            ConsentStatus::Denied => "denied",
            ConsentStatus::Expired => "expired",
        }
    }
}

/// How a parked request ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConsentOutcome {
    /// A human pressed Approve on the native control.
    Approved,
    /// A human pressed Deny, or the chrome dropped the request without
    /// answering it.
    Denied,
    /// Nobody answered within the parked-request lifetime.
    Expired,
}

/// A request that has finished, kept just long enough for the holding page to
/// find out where to go.
#[derive(Debug, Clone)]
struct Resolved {
    id: u64,
    status: ConsentStatus,
    /// The absolute URL the caller's browser is sent to, already carrying the
    /// code or the error, the original `state`, and the issuer.
    redirect: String,
    /// When this record is forgotten. After that the holding page reports
    /// `expired`, which is the honest answer for a request nothing remembers.
    forget_at_ms: u64,
}

/// One authorization code, bound at minting to everything redemption must
/// re-check.
#[derive(Debug, Clone)]
struct CodeRecord {
    /// The lowercase hex SHA-256 of the code, through the same
    /// [`digest_of`] the token store uses. The raw code existed once, in the
    /// `Location` header that carried it to the client.
    digest: String,
    /// The client the code was issued to.
    client_id: String,
    /// The PKCE challenge the code was issued against.
    code_challenge: String,
    /// The challenge method that challenge was presented under.
    ///
    /// Recorded rather than assumed. The authorization endpoint refuses
    /// anything but [`CHALLENGE_METHOD`], so in a correct build this is always
    /// `S256` — which is exactly why it is stored: redemption re-checks it, so
    /// a future edit that loosened the first gate would still grant nothing.
    code_challenge_method: String,
    /// The redirect URI the code was issued for.
    redirect_uri: String,
    /// Unix epoch milliseconds after which the code is refused.
    expires_at_ms: u64,
}

/// The authorization codes this browser has minted and not yet seen redeemed.
///
/// In memory only, and deliberately: a code lives for a minute, is redeemed
/// once, and has no business surviving a restart — a code that outlived the
/// process that issued it would be a credential lying in a file for no reason.
#[derive(Debug, Default)]
struct AuthorizationCodes {
    entries: Vec<CodeRecord>,
}

impl AuthorizationCodes {
    /// Issue a code, keeping only its digest, and return the raw value once.
    ///
    /// The bytes come from the operating system's random source — the same one
    /// `crate::agents` draws tokens from — and never from a clock: a code an
    /// attacker can derive from the moment it was issued is not a code.
    fn mint(
        &mut self,
        client_id: &str,
        code_challenge: &str,
        code_challenge_method: &str,
        redirect_uri: &str,
        expires_at_ms: u64,
    ) -> String {
        let mut bytes = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut bytes);
        let code = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
        self.entries.push(CodeRecord {
            digest: digest_of(&code),
            client_id: client_id.to_owned(),
            code_challenge: code_challenge.to_owned(),
            code_challenge_method: code_challenge_method.to_owned(),
            redirect_uri: redirect_uri.to_owned(),
            expires_at_ms,
        });
        code
    }

    /// Remove and return the record behind a presented code, in **one**
    /// operation.
    ///
    /// One operation is the whole point, and it is why this is not a `find`
    /// beside a `remove`: a read-then-remove pair lets two concurrent
    /// exchanges of the same code both find it, and single-use is then a
    /// property of how fast the two callers happened to be. Redemption itself
    /// — the PKCE comparison, the client and redirect-URI re-checks, and the
    /// constant-time digest compare — is [`TalariaAuth::redeem_code`]'s; this
    /// store exists so that path has one place to take a code from.
    ///
    /// An **expired** code is removed and *not* returned, which is the same
    /// answer an unknown one gets. Removing it either way is deliberate: a
    /// code that survived a refused redemption would be retryable by whoever
    /// intercepted it.
    fn take(&mut self, code: &str, now_ms: u64) -> Option<CodeRecord> {
        let digest = digest_of(code);
        let index = self.entries.iter().position(|record| record.digest == digest)?;
        let record = self.entries.swap_remove(index);
        (record.expires_at_ms > now_ms).then_some(record)
    }

    /// Forget every code whose minute is up.
    fn prune(&mut self, now_ms: u64) {
        self.entries.retain(|record| record.expires_at_ms > now_ms);
    }
}

/// Everything the authorization endpoint needs to remember between requests.
///
/// One mutex over both halves, because every decision this module makes reads
/// the gate and writes the codes in the same breath, and two locks taken in
/// two orders is a deadlock waiting for a busy afternoon.
#[derive(Debug, Default)]
struct ConsentState {
    /// The id of the one request currently on screen, if any.
    ///
    /// **An `Option`, not a counter.** "At most one consent request is on
    /// screen at a time" is then a property of the type rather than of a
    /// comparison somebody has to keep correct.
    parked: Option<u64>,
    /// Unix epoch milliseconds before which `/authorize` refuses outright.
    ///
    /// Armed by **every** terminal resolution the human did not consent to —
    /// a denial and an expiry alike. Scoping it to denial would leave the
    /// expiry path open: a peer could raise a request, wait out a human who
    /// simply ignores it, and re-raise the instant it expired, holding the
    /// displayed page off screen indefinitely because a panel replaces the
    /// page. Approval arms nothing: a client that just received a code has no
    /// reason to ask again, and the one-on-screen cap still applies if it does.
    cooldown_until_ms: u64,
    /// Requests that have finished, kept until [`Resolved::forget_at_ms`].
    resolved: Vec<Resolved>,
    /// The codes minted so far.
    codes: AuthorizationCodes,
}

/// The consent state, shared by the `talaria-http` thread's request handlers
/// and the background task that waits for each decision.
type SharedConsent = Arc<Mutex<ConsentState>>;

/// How the authorization endpoint asks a human.
///
/// A callback rather than an [`winit::event_loop::EventLoopProxy`] held here
/// directly, for two reasons that both outlast this plan. The window loop is
/// `crate::http`'s business — this module owns *what* is asked and *what the
/// answer means*, not how a message reaches the main thread — and a callback
/// is something a test can supply, which is what lets the whole authorization
/// path (parking, both harassment caps, approval, denial and expiry) be
/// exercised without a window.
///
/// `Err(())` means the chrome is gone: the browser is shutting down, and the
/// request is refused rather than parked forever.
pub type ConsentRaiser = Arc<dyn Fn(ConsentRequest) -> Result<(), ()> + Send + Sync>;

impl ConsentState {
    /// Drop everything whose time is up, so neither list grows without bound.
    fn prune(&mut self, now_ms: u64) {
        self.resolved.retain(|entry| entry.forget_at_ms > now_ms);
        self.codes.prune(now_ms);
    }

    /// Whether a new request may be raised right now.
    ///
    /// Both refusals are the harassment caps, and both answer `access_denied`
    /// without raising anything — the caller is told no, and the human is not
    /// interrupted to say it.
    fn may_raise(&self, now_ms: u64) -> bool {
        self.parked.is_none() && now_ms >= self.cooldown_until_ms
    }

    /// Where a request has got to, and where its caller should be sent.
    fn status_of(&self, id: u64) -> (ConsentStatus, Option<String>) {
        if self.parked == Some(id) {
            return (ConsentStatus::Pending, None);
        }
        match self.resolved.iter().find(|entry| entry.id == id) {
            Some(entry) => (entry.status, Some(entry.redirect.clone())),
            // A request nothing remembers is reported as expired rather than
            // as an error: it is the truthful answer, it is the same answer
            // for an id that was never issued, and it tells a poller nothing
            // about which ids exist.
            None => (ConsentStatus::Expired, None),
        }
    }
}

/// One validated authorization request, parked while a human decides.
///
/// Everything here has already been checked, which is why the consent panel
/// has no error state: an invalid request never reaches it.
#[derive(Debug, Clone)]
struct ParkedAuthorization {
    id: u64,
    client_id: String,
    client_name: String,
    redirect_uri: String,
    code_challenge: String,
    /// The method that challenge was presented under, carried through to the
    /// code so redemption can re-check it. Always [`CHALLENGE_METHOD`] — see
    /// [`CodeRecord::code_challenge_method`] for why it is stored anyway.
    code_challenge_method: String,
    /// The caller's `state`, echoed back untouched. Opaque to this browser.
    state: Option<String>,
}

/// Apply a terminal outcome: clear the slot, arm the cooldown, mint the code
/// if — and only if — a human approved.
///
/// **This function is the grant path, and it reads no environment variable.**
/// The timings arrive as a parameter so a test can pin them, exactly as every
/// clock in this module and in `crate::agents` does; there is nothing here for
/// an override to reach.
fn settle(
    consent: &mut ConsentState,
    parked: &ParkedAuthorization,
    outcome: ConsentOutcome,
    now_ms: u64,
    timings: ConsentTimings,
    issuer: &str,
) {
    if consent.parked == Some(parked.id) {
        consent.parked = None;
    }
    let (status, redirect) = match outcome {
        ConsentOutcome::Approved => {
            let code = consent.codes.mint(
                &parked.client_id,
                &parked.code_challenge,
                &parked.code_challenge_method,
                &parked.redirect_uri,
                now_ms.saturating_add(AUTHORIZATION_CODE_LIFETIME_MS),
            );
            let redirect = redirect_with(&parked.redirect_uri, issuer, parked.state.as_deref(), |query| {
                query.append_pair("code", &code);
            });
            (ConsentStatus::Approved, redirect)
        },
        ConsentOutcome::Denied | ConsentOutcome::Expired => {
            let cooldown = match outcome {
                ConsentOutcome::Expired => timings.expiry_cooldown_ms,
                _ => timings.deny_cooldown_ms,
            };
            consent.cooldown_until_ms = now_ms.saturating_add(cooldown);
            let status = match outcome {
                ConsentOutcome::Expired => ConsentStatus::Expired,
                _ => ConsentStatus::Denied,
            };
            let redirect = redirect_with(&parked.redirect_uri, issuer, parked.state.as_deref(), |query| {
                query.append_pair("error", "access_denied");
                query.append_pair(
                    "error_description",
                    "the person at this browser did not approve the request",
                );
            });
            (status, redirect)
        },
    };
    let redirect = match redirect {
        Some(redirect) => redirect,
        // Unreachable: the redirect URI parsed during validation. Answered
        // rather than asserted, because this runs on a network-reachable path.
        None => parked.redirect_uri.clone(),
    };
    consent.resolved.push(Resolved {
        id: parked.id,
        status,
        redirect,
        forget_at_ms: now_ms.saturating_add(AUTHORIZATION_CODE_LIFETIME_MS),
    });
}

/// A caller's redirect URI with this server's answer appended.
///
/// Built through `url`'s own query serialiser rather than by string
/// concatenation, so a registered URI that already carries a query keeps it and
/// every value is escaped once and correctly. `iss` (RFC 9207) is emitted on
/// every response, success and error alike, and the authorization-server
/// metadata advertises that it is — an emitted parameter the metadata does not
/// claim is as broken as the reverse.
fn redirect_with(
    redirect_uri: &str,
    issuer: &str,
    state: Option<&str>,
    append: impl FnOnce(&mut url::form_urlencoded::Serializer<'_, url::UrlQuery<'_>>),
) -> Option<String> {
    let mut url = Url::parse(redirect_uri).ok()?;
    {
        let mut query = url.query_pairs_mut();
        append(&mut query);
        if let Some(state) = state {
            query.append_pair("state", state);
        }
        query.append_pair("iss", issuer);
    }
    Some(url.to_string())
}

/// One value out of a URL-encoded query string.
fn query_value(query: &str, name: &str) -> Option<String> {
    url::form_urlencoded::parse(query.as_bytes())
        .find(|(key, _)| key == name)
        .map(|(_, value)| value.into_owned())
}

/// Whether a URL's host is a loopback IP literal.
///
/// Literals only. RFC 8252 §8.3 prefers `127.0.0.1` and `[::1]` over the
/// `localhost` *name*, because what `localhost` resolves to is the resolver's
/// business and not this browser's, and a name that resolved elsewhere would
/// turn the loopback exception into a hole.
fn host_is_loopback(url: &Url) -> bool {
    match url.host() {
        Some(url::Host::Ipv4(address)) => address.is_loopback(),
        Some(url::Host::Ipv6(address)) => address.is_loopback(),
        _ => false,
    }
}

/// Whether a redirect URI may be registered at all.
///
/// HTTPS anywhere, or HTTP on a loopback literal — and the loopback exception
/// is the *only* reason an `http` redirect is admissible: OAuth 2.1 §1.5
/// requires HTTPS with exactly that carve-out, which is what a native client
/// binding an ephemeral port needs. A fragment is refused outright: RFC 6749
/// forbids one in a redirect URI, and a URI carrying one could never be
/// matched against what this server would build.
fn admissible_redirect(uri: &str) -> bool {
    let Ok(url) = Url::parse(uri) else {
        return false;
    };
    if url.fragment().is_some() {
        return false;
    }
    match url.scheme() {
        "https" => url.host().is_some(),
        "http" => host_is_loopback(&url),
        _ => false,
    }
}

/// Whether a presented redirect URI is the one that was registered.
///
/// Exact, with the loopback **port** as the single permitted variance — RFC
/// 8252 requires an authorization server to allow any port for a loopback
/// redirect, because a native client binds an ephemeral one and cannot know
/// its number at registration time. That is the one field wide the exception
/// is; scheme, host, path, query and userinfo are all compared as they were
/// registered.
fn redirect_matches(registered: &str, presented: &str) -> bool {
    let (Ok(registered), Ok(presented)) = (Url::parse(registered), Url::parse(presented)) else {
        return false;
    };
    if registered == presented {
        return true;
    }
    if !host_is_loopback(&registered) || !host_is_loopback(&presented) {
        return false;
    }
    registered.scheme() == presented.scheme()
        && registered.host() == presented.host()
        && registered.path() == presented.path()
        && registered.query() == presented.query()
        && registered.fragment() == presented.fragment()
        && registered.username() == presented.username()
        && registered.password() == presented.password()
}

/// Whether a PKCE challenge is well-formed.
///
/// RFC 7636 §4.2: 43 to 128 characters from the unreserved set. Checked before
/// a human is asked anything, so a challenge that could never verify cannot
/// cost somebody an interruption.
fn well_formed_challenge(challenge: &str) -> bool {
    let length = challenge.chars().count();
    (43..=128).contains(&length)
        && challenge
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '.' | '_' | '~'))
}

/// How an authorization request was refused.
///
/// The split is not cosmetic. Until the client and its redirect URI have both
/// been checked there is **nowhere safe to send an error**: reporting to an
/// unverified redirect URI would turn this endpoint into an open redirector.
/// Afterwards, RFC 6749 §4.1.2.1 requires the error to go to that URI, which
/// is also the only way the caller's own flow learns what happened.
#[derive(Debug)]
enum Refusal {
    /// Answered to the caller directly, because no redirect URI is trusted yet.
    Direct { error: &'static str, description: &'static str },
    /// Reported to the validated redirect URI.
    Redirect {
        to: String,
        state: Option<String>,
        error: &'static str,
        description: &'static str,
    },
}

/// The four static holding-page bodies, one per state.
///
/// **No value taken from the request reaches any of them** — not the display
/// name, not the `state`, not the redirect URI, not the client id, not an
/// error detail. That is stricter than escaping: it removes the injection and
/// phishing surface from this page rather than trying to neutralise it. The
/// only interpolated value is this browser's own parked-request id, which the
/// caller did not supply, which is a `u64` and therefore cannot carry markup,
/// and which the page needs in order to find out what happened.
///
/// **No control of any kind.** No form, no button, no link — and, in a
/// deliberate divergence from `04-UI-SPEC.md`'s "the only script is a status
/// poll", **no script at all**: the headers that same contract fixes include
/// `default-src 'none'`, which forbids inline script, so a scripted poll would
/// have been dead on arrival. A `<meta http-equiv="refresh">` does the same
/// job, is not a control, and is not something an agent can drive. A page that
/// cannot approve anything cannot be scripted into approving anything.
fn holding_page(status: ConsentStatus, id: u64) -> String {
    let body = match status {
        ConsentStatus::Pending => {
            "Talaria is asking you to approve this connection. \
             Switch to the Talaria window to answer."
        },
        ConsentStatus::Approved => "Approved. You can close this tab.",
        ConsentStatus::Denied => "Denied. Nothing was granted.",
        ConsentStatus::Expired => {
            "That request expired. Start it again from the program that opened this page."
        },
    };
    let refresh = match status {
        ConsentStatus::Pending => format!(
            "<meta http-equiv=\"refresh\" content=\"1;url={AUTHORIZATION_STATUS_PATH}?id={id}\">"
        ),
        _ => String::new(),
    };
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
         <title>Talaria — approve this connection</title>\n{refresh}\n</head>\n\
         <body>\n<p>{body}</p>\n</body>\n</html>\n"
    )
}

/// The holding page's headers, exactly as `04-UI-SPEC.md` fixes them.
fn holding_page_headers(status: ConsentStatus) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("text/html; charset=utf-8"));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'none'; frame-ancestors 'none'"),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    // The one word a programmatic poller needs, so it does not have to read
    // the prose. The state name and nothing else.
    if let Ok(name) = HeaderName::try_from("x-talaria-consent") {
        if let Ok(value) = HeaderValue::from_str(status.as_str()) {
            headers.insert(name, value);
        }
    }
    headers
}

/// The holding page as a response, with the redirect once there is one.
fn holding_response(status: ConsentStatus, id: u64, redirect: Option<String>) -> Response<GenericBody> {
    let mut headers = holding_page_headers(status);
    let code = match redirect.as_deref().and_then(|to| HeaderValue::from_str(to).ok()) {
        Some(location) => {
            headers.insert(header::LOCATION, location);
            StatusCode::FOUND
        },
        None => StatusCode::OK,
    };
    GenericBody::from_string(holding_page(status, id)).into_response(code, Some(headers))
}

/// An OAuth error object as a JSON response, uncacheable.
fn oauth_error(
    code: StatusCode,
    error: &str,
    description: &str,
) -> Response<GenericBody> {
    let mut headers = HeaderMap::new();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    GenericBody::from_value(&serde_json::json!({
        "error": error,
        "error_description": description,
    }))
    .into_json_response(code, Some(headers))
}

/// The one refusal every failed grant receives, built in one place.
///
/// A function rather than a constant so there is exactly one expression that
/// produces it and no second call site can quietly pass a different status or
/// description — the same arrangement [`refused`] uses on the resource-server
/// side, and for the same reason.
///
/// `invalid_grant` with `400`, per RFC 6749 §5.2. The resource server's
/// `401`/`403` table does not apply at this endpoint: nothing here is
/// presenting a bearer token, and a client that is told `401` looks for a
/// `WWW-Authenticate` challenge that a token endpoint has no business sending.
fn grant_refused() -> (StatusCode, &'static str, &'static str) {
    (StatusCode::BAD_REQUEST, "invalid_grant", GRANT_REFUSAL)
}

/// A PKCE `S256` challenge, re-spelled as the digest [`digest_of`] produces.
///
/// RFC 7636 §4.2 writes a challenge as base64url-without-padding of the
/// SHA-256 of the verifier, while [`digest_of`] — the one token-to-digest
/// mapping this browser has — writes lowercase hex. They are the same
/// thirty-two bytes in two spellings, so the stored challenge is decoded and
/// re-spelled here rather than a second SHA-256 being written in this file.
///
/// `None` means the stored challenge is not thirty-two base64url bytes at all,
/// which no verifier could ever match — the authorization endpoint's
/// well-formedness check admits a few characters base64url does not, so this
/// is reachable, and it lands on the same refusal everything else does.
fn challenge_as_digest(challenge: &str) -> Option<String> {
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(challenge).ok()?;
    (bytes.len() == 32).then(|| hex::encode(bytes))
}

/// A successful token response body (RFC 6749 §5.1).
///
/// Built in one place so the authorization-code grant and the refresh grant
/// cannot drift into answering differently — a client that got a `scope` from
/// one and not the other would be right to treat them as different servers.
fn token_document(access_token: &str, refresh_token: &str) -> serde_json::Value {
    serde_json::json!({
        "access_token": access_token,
        "token_type": "Bearer",
        "expires_in": DEFAULT_ACCESS_TTL_MS / 1_000,
        "refresh_token": refresh_token,
        // One scope, always this one. See `TALARIA_SCOPE`: the field exists
        // because the response is not well-formed without it, not because a
        // decision is made from it.
        "scope": TALARIA_SCOPE,
    })
}

/// A token document as a response: `200`, JSON, and stored nowhere.
///
/// `no-store` **and** `no-cache`, which RFC 6749 §5.1 requires by name: this
/// body carries two live credentials, and an intermediary that kept a copy
/// would be holding the whole grant.
fn token_response(document: &serde_json::Value) -> Response<GenericBody> {
    let mut headers = HeaderMap::new();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    GenericBody::from_value(document).into_json_response(StatusCode::OK, Some(headers))
}

/// A fresh, unguessable identifier for a parked request.
///
/// Random rather than sequential for the reason [`ConsentRequest::id`] gives:
/// the holding page's status poll is keyed on it, so a counter would let any
/// local caller walk the ids of requests it did not make.
fn random_request_id() -> u64 {
    let mut bytes = [0u8; 8];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    u64::from_le_bytes(bytes)
}

/// Talaria's OAuth provider: the resource server today, and in 04-06 the
/// authorization server beside it.
///
/// Every derived string is built once, at construction, from the address the
/// listener actually bound — see [`canonical_resource`] for why a second
/// construction site would be a bug rather than a duplication.
pub struct TalariaAuth {
    /// The live store. Read on every verification and never copied out of.
    agents: SharedAgents,
    /// This server's RFC 8707 canonical resource identifier.
    resource: String,
    /// The absolute URL of the metadata document, for the `401` challenge.
    metadata_url: String,
    /// The `host:port` this listener bound, which is also the issuer identity
    /// the metadata document names as its authorization server.
    bound: String,
    /// This server's issuer identifier, `http://{bound}` — the value the RFC
    /// 8414 document names as `issuer` and the value the RFC 9207 `iss`
    /// parameter carries. Built once so the two cannot disagree.
    issuer: String,
    /// The endpoints this provider declares. The SDK builds its auth router by
    /// folding over these keys, so an entry here *is* a route.
    endpoints: HashMap<String, OauthEndpoint>,
    /// The single required scope, as the vector shape the trait wants.
    scopes: Vec<String>,
    /// The one parked request, the cooldown, and the codes minted so far.
    consent: SharedConsent,
    /// How the consent panel is raised. See [`ConsentRaiser`].
    raise_consent: ConsentRaiser,
    /// The three consent durations, resolved once when the listener was built.
    ///
    /// Once, not per request: the environment cannot change under a running
    /// process, and a per-request read would be a lock and an allocation on
    /// every authorization — the same reasoning `Gui` gives for reading its
    /// own test-hook gate in its constructor.
    timings: ConsentTimings,
}

impl TalariaAuth {
    /// Build a provider for a listener bound at `bound` (`host:port`).
    pub fn new(agents: SharedAgents, raise_consent: ConsentRaiser, bound: &str) -> Self {
        // The resource server's document, and the authorization server this
        // browser co-hosts: its metadata, registration, authorization and
        // token endpoints, plus the path the holding page refreshes to.
        //
        // The **token** endpoint is declared and answered — 04-07 filled in
        // the arm 04-06 stubbed, so the `token_endpoint` the metadata names is
        // a path that redeems codes and rotates refresh tokens. The
        // **revocation** endpoint arrives here the same way: declared in this
        // map and answered in `handle_request`, in one change, because an
        // endpoint declared without a handler is a `500` waiting for the first
        // client that follows the metadata document to it.
        //
        // What is **never** declared: the token introspection endpoint. The
        // authorization server and the resource server are the same process
        // reading the same `Vec`; there is nothing to introspect across, and
        // an introspection endpoint would only be a second, remote-shaped way
        // to ask a question this process answers by a function call.
        let mut endpoints = HashMap::new();
        endpoints.insert(
            OAUTH_PROTECTED_RESOURCE_BASE.to_owned(),
            OauthEndpoint::ProtectedResourceMetadata,
        );
        // `/.well-known/oauth-authorization-server` — the RFC 8414 document,
        // through the SDK's own constant rather than a second spelling of the
        // path, for the reason `canonical_resource` gives about strings that
        // are compared rather than merely printed. The issuer this browser
        // names has no path component, so the well-known URI is the bare one.
        endpoints.insert(
            WELL_KNOWN_OAUTH_AUTHORIZATION_SERVER.to_owned(),
            OauthEndpoint::AuthorizationServerMetadata,
        );
        endpoints.insert(AUTHORIZATION_PATH.to_owned(), OauthEndpoint::AuthorizationEndpoint);
        endpoints
            .insert(AUTHORIZATION_STATUS_PATH.to_owned(), OauthEndpoint::AuthorizationEndpoint);
        endpoints.insert(TOKEN_PATH.to_owned(), OauthEndpoint::TokenEndpoint);
        endpoints.insert(REGISTRATION_PATH.to_owned(), OauthEndpoint::RegistrationEndpoint);
        endpoints.insert(REVOCATION_PATH.to_owned(), OauthEndpoint::RevocationEndpoint);
        Self {
            agents,
            resource: canonical_resource(bound),
            metadata_url: metadata_url(bound),
            bound: bound.to_owned(),
            issuer: format!("http://{bound}"),
            endpoints,
            scopes: vec![TALARIA_SCOPE.to_owned()],
            consent: SharedConsent::default(),
            raise_consent,
            timings: consent_timings(),
        }
    }

    /// Verify a presented token against the store as it is *right now*.
    ///
    /// Split out from [`AuthProvider::verify_token`] with the clock as a
    /// parameter so a unit test can pin a moment — the same reason every
    /// function in [`crate::agents`] takes its time as an argument.
    ///
    /// Two properties, both load-bearing:
    ///
    /// 1. **The lookup is live.** It reaches the shared store on every call
    ///    and keeps nothing. That is the entire mechanism behind a revoked
    ///    token failing on the *next* request rather than at the next restart.
    /// 2. **Every failure is the same failure.** Unknown, expired, consumed,
    ///    wrong-kind and wrong-audience all return [`REFUSAL`], because a
    ///    caller that could distinguish them could enumerate which tokens
    ///    exist and learn that a stolen value was genuine.
    fn verify(&self, token: &str, now_ms: u64) -> Result<AuthInfo, AuthenticationError> {
        // The store's own mapping, never a second definition: the digest a
        // record was written with and the digest a presented token is searched
        // by must come from one function or tokens are issued and then never
        // work. See `agents::digest_of`.
        let digest = digest_of(token);
        // One synchronous critical section, no `await` inside it, and
        // everything needed afterwards is copied out before the guard drops —
        // so the main thread is never waiting on this thread to finish an
        // engine round trip.
        let found = {
            let store = match self.agents.lock() {
                Ok(store) => store,
                Err(poisoned) => {
                    // A poisoned lock means some other thread panicked while
                    // holding the store, so its contents may be mid-mutation.
                    // Refuse rather than read it: this module's degrade
                    // direction is deny, and a store nobody can vouch for is
                    // exactly the case for it.
                    log::error!(
                        "agent store lock is poisoned; refusing every credential until restart \
                         (token {})",
                        digest_prefix(&digest)
                    );
                    drop(poisoned);
                    return Err(refused());
                },
            };
            store.lookup(&digest, now_ms).map(|record| {
                (
                    record.client_id.clone(),
                    record.audience.clone(),
                    record.scope.clone(),
                    record.expires_at_ms,
                )
            })
        };
        let Some((client_id, audience, scope, expires_at_ms)) = found else {
            log::debug!("remote access refused token {}", digest_prefix(&digest));
            return Err(refused());
        };

        // RFC 8707. Byte for byte against the one canonical identifier — see
        // `canonical_resource`. A token minted for another resource server is
        // refused here and, just as importantly, is never accepted, stored or
        // passed onward: this is the confused-deputy defence, and the whole of
        // it is that Talaria only ever honours tokens addressed to Talaria.
        if audience != self.resource {
            log::warn!(
                "remote access refused token {} presented by client {client_id}: \
                 it was issued for another resource",
                digest_prefix(&digest)
            );
            return Err(refused());
        }

        let Some(expires_at) = expiry_at(expires_at_ms) else {
            log::warn!(
                "remote access refused token {}: its expiry cannot be represented",
                digest_prefix(&digest)
            );
            return Err(refused());
        };

        log::debug!("remote access accepted token {} for client {client_id}",
            digest_prefix(&digest));
        Ok(AuthInfo {
            // The digest, not the token. It is unique per token, it is what
            // the store is keyed by, and it is not a credential — so a
            // session that carries it carries nothing worth stealing.
            token_unique_id: digest,
            client_id: Some(client_id),
            // Single-user software: there is no user directory and nobody to
            // be. The human is authenticated by having the window.
            user_id: None,
            scopes: Some(vec![scope]),
            // The SDK's middleware re-checks this against the wall clock on
            // every request, which is a second, independent expiry check we
            // deliberately do not try to skip.
            expires_at: Some(expires_at),
            audience: Some(Audience::Single(audience)),
            extra: None,
        })
    }

    /// The RFC 9728 protected-resource metadata document.
    ///
    /// This is the document `04-RESEARCH.md`'s Pitfall 3 is about: a server
    /// that mints and validates tokens but publishes nothing is one a real MCP
    /// client cannot *find the flow for*, and the client gives up before the
    /// flow begins.
    fn metadata_document(&self) -> serde_json::Value {
        serde_json::json!({
            // The identifier a client must ask its authorization server for,
            // and the one every token is checked against.
            "resource": self.resource,
            // Talaria is its own authorization server. 04-06 publishes the
            // RFC 8414 document at this issuer.
            "authorization_servers": [format!("http://{}", self.bound)],
            // Header only. The specification forbids access tokens in query
            // strings, and saying so here is how a conformant client knows not
            // to try — a URL is logged, cached and sent in a `Referer`.
            "bearer_methods_supported": ["header"],
            "scopes_supported": [TALARIA_SCOPE],
            "resource_name": "Talaria browser",
            // Deliberately absent: `client_id_metadata_document_supported`.
            // See `04-CONTEXT.md` D-04-02 — CIMD requires the authorization
            // server to fetch an attacker-supplied HTTPS URL, which is an
            // unauthenticated SSRF primitive inside the process holding the
            // credential vault. Advertising a capability this browser does not
            // implement would also break conformant clients that prefer it.
        })
    }

    /// The metadata document as a response: `200`, JSON, and uncacheable.
    ///
    /// `no-store` because the document names the address this listener is
    /// bound to, and that changes when the human switches remote access off
    /// and on again. A cached copy pointing at a port nothing is listening on
    /// would send a client to a closed door with no way to tell it was stale.
    fn metadata_response(&self) -> Response<GenericBody> {
        let mut headers = HeaderMap::new();
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        GenericBody::from_value(&self.metadata_document())
            .into_json_response(StatusCode::OK, Some(headers))
    }

    /// The RFC 8414 authorization-server metadata document.
    ///
    /// Every key here is a capability this browser actually implements, and
    /// the omissions are as deliberate as the entries:
    ///
    /// - **`code_challenge_methods_supported` is `["S256"]` and nothing else.**
    ///   A conformant client refuses to proceed without this key, and a client
    ///   that saw `plain` in it would be entitled to use `plain` — which is no
    ///   protection at all, since the "verifier" is then the challenge.
    /// - **No `client_id_metadata_document_supported`.** `04-CONTEXT.md`
    ///   D-04-02: that mechanism requires the authorization server to fetch an
    ///   attacker-supplied HTTPS URL, which is an unauthenticated
    ///   request-forgery primitive inside the process holding the credential
    ///   vault. Not implemented, therefore not advertised — advertising it
    ///   would also break a conformant client that prefers it.
    /// - **`revocation_endpoint` is named, and answered** (RFC 7009). Its
    ///   `revocation_endpoint_auth_methods_supported` is `["none"]` for the
    ///   same reason the token endpoint's is: this browser issues credentials
    ///   to public clients, which have no secret to authenticate with, and the
    ///   token a caller presents *is* the authorization to revoke it.
    /// - **`authorization_response_iss_parameter_supported` is `true`, and
    ///   this browser does emit `iss`.** The two have to agree: an emitted
    ///   parameter the metadata does not claim is as broken as a claimed one
    ///   that never arrives.
    fn authorization_server_metadata(&self) -> serde_json::Value {
        serde_json::json!({
            "issuer": self.issuer,
            "authorization_endpoint": format!("{}{AUTHORIZATION_PATH}", self.issuer),
            "token_endpoint": format!("{}{TOKEN_PATH}", self.issuer),
            "registration_endpoint": format!("{}{REGISTRATION_PATH}", self.issuer),
            "revocation_endpoint": format!("{}{REVOCATION_PATH}", self.issuer),
            "revocation_endpoint_auth_methods_supported": ["none"],
            "scopes_supported": [TALARIA_SCOPE],
            "response_types_supported": ["code"],
            "response_modes_supported": ["query"],
            "grant_types_supported": ["authorization_code", "refresh_token"],
            "code_challenge_methods_supported": [CHALLENGE_METHOD],
            // A public client with no secret: there is nothing for a local
            // agent to keep secret from a human who owns the machine, and
            // PKCE is what stands in for client authentication.
            "token_endpoint_auth_methods_supported": ["none"],
            "authorization_response_iss_parameter_supported": true,
        })
    }

    /// Dynamic client registration (RFC 7591).
    ///
    /// Unauthenticated by necessity — a client that has not registered has no
    /// credential to present — and safe to expose for one reason worth stating
    /// plainly: **nothing here is fetched.** The redirect URIs are parsed,
    /// compared and later displayed; not one of them is ever dereferenced, and
    /// this module constructs no outbound HTTP client at all. That is the
    /// whole of what keeps an open registration endpoint from being a
    /// request-forgery primitive.
    ///
    /// A registration is **inert** until a human approves it: it holds no
    /// tokens and appears in no list, so flooding the registry buys an
    /// attacker a bounded amount of disk and nothing else. The cap in
    /// `crate::agents` refuses rather than evicting, so a flood cannot
    /// displace a legitimate client either.
    fn handle_registration(&self, body: &str, now_ms: u64) -> Response<GenericBody> {
        match self.register_client(body, now_ms) {
            Ok(document) => {
                let mut headers = HeaderMap::new();
                headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
                GenericBody::from_value(&document)
                    .into_json_response(StatusCode::CREATED, Some(headers))
            },
            Err((code, error, description)) => oauth_error(code, error, description),
        }
    }

    /// [`TalariaAuth::handle_registration`] without the response wrapping, so
    /// the document a caller actually receives is a value a test can read.
    fn register_client(
        &self,
        body: &str,
        now_ms: u64,
    ) -> Result<serde_json::Value, (StatusCode, &'static str, &'static str)> {
        let Ok(request) = serde_json::from_str::<serde_json::Value>(body) else {
            return Err((
                StatusCode::BAD_REQUEST,
                "invalid_client_metadata",
                "the registration request is not a JSON object",
            ));
        };
        let client_name = request
            .get("client_name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let Some(uris) = request.get("redirect_uris").and_then(serde_json::Value::as_array) else {
            return Err((
                StatusCode::BAD_REQUEST,
                "invalid_redirect_uri",
                "redirect_uris is required and must be an array",
            ));
        };
        let redirect_uris: Vec<String> = uris
            .iter()
            .filter_map(serde_json::Value::as_str)
            .map(str::to_owned)
            .collect();
        // Refused **whole**, never stored in part. A registration that kept
        // the admissible half of a list would leave a client whose
        // registration is not the one it asked for, and the mismatch would
        // only surface later as an authorization that inexplicably fails.
        if redirect_uris.len() != uris.len()
            || redirect_uris.is_empty()
            || !redirect_uris.iter().all(|uri| admissible_redirect(uri))
        {
            return Err((
                StatusCode::BAD_REQUEST,
                "invalid_redirect_uri",
                "every redirect URI must be an https URL or an http URL on a loopback address",
            ));
        }
        let registered = {
            let mut store = match self.agents.lock() {
                Ok(store) => store,
                Err(_) => {
                    log::error!("agent store lock is poisoned; refusing registration");
                    return Err((
                        StatusCode::SERVICE_UNAVAILABLE,
                        "temporarily_unavailable",
                        "the agent store cannot be read",
                    ));
                },
            };
            store.register(client_name.clone(), redirect_uris.clone(), now_ms)
        };
        let Some(client) = registered else {
            return Err((
                StatusCode::BAD_REQUEST,
                "invalid_client_metadata",
                "this browser is already holding as many client registrations as it will keep",
            ));
        };
        log::info!("registered agent client {}", client.client_id);
        Ok(serde_json::json!({
            "client_id": client.client_id,
            "client_id_issued_at": now_ms / 1_000,
            "client_name": client.client_name,
            "redirect_uris": client.redirect_uris,
            "grant_types": ["authorization_code", "refresh_token"],
            "response_types": ["code"],
            "token_endpoint_auth_method": "none",
        }))
    }

    /// Check every parameter of an authorization request, in the order that
    /// keeps each refusal answerable.
    ///
    /// **Every refusal here happens before a human is asked anything**, which
    /// is why the consent panel has no error state: a request that reaches the
    /// chrome has already had its client, its redirect URI, its challenge and
    /// its audience checked, so there is no invalid request left for the panel
    /// to render.
    fn validate_authorization(&self, query: &str) -> Result<ParkedAuthorization, Refusal> {
        let Some(client_id) = query_value(query, "client_id") else {
            return Err(Refusal::Direct {
                error: "invalid_request",
                description: "client_id is required",
            });
        };
        let Some(presented_redirect) = query_value(query, "redirect_uri") else {
            return Err(Refusal::Direct {
                error: "invalid_request",
                description: "redirect_uri is required",
            });
        };
        // The client and its redirect URI first, together, because until both
        // are known good there is nowhere an error may be sent: answering to
        // an unverified redirect URI would make this endpoint an open
        // redirector. One refusal covers both, so the endpoint does not become
        // an oracle for which client ids are registered.
        let client = {
            let store = match self.agents.lock() {
                Ok(store) => store,
                Err(_) => {
                    log::error!("agent store lock is poisoned; refusing authorization");
                    return Err(Refusal::Direct {
                        error: "temporarily_unavailable",
                        description: "the agent store cannot be read",
                    });
                },
            };
            store
                .clients()
                .iter()
                .find(|client| client.client_id == client_id)
                .filter(|client| {
                    client
                        .redirect_uris
                        .iter()
                        .any(|registered| redirect_matches(registered, &presented_redirect))
                })
                .cloned()
        };
        let Some(client) = client else {
            return Err(Refusal::Direct {
                error: "invalid_request",
                description: "unknown client, or a redirect URI that is not the registered one",
            });
        };
        // From here the redirect URI is trusted, so RFC 6749 §4.1.2.1's rule
        // applies: the error goes to it, which is the only way the caller's
        // own flow learns what happened.
        let state = query_value(query, "state");
        let refuse = |error: &'static str, description: &'static str| Refusal::Redirect {
            to: presented_redirect.clone(),
            state: state.clone(),
            error,
            description,
        };
        if query_value(query, "response_type").as_deref() != Some("code") {
            return Err(refuse(
                "unsupported_response_type",
                "this authorization server issues authorization codes and nothing else",
            ));
        }
        // Pitfall 5. `plain` is refused along with everything else that is not
        // S256, and it is refused *by omission* rather than by a match arm
        // naming it: an arm for `plain` is the warning sign, because it is one
        // edit away from being an arm that accepts it.
        if query_value(query, "code_challenge_method").as_deref() != Some(CHALLENGE_METHOD) {
            return Err(refuse(
                "invalid_request",
                "code_challenge_method must be S256",
            ));
        }
        let challenge = query_value(query, "code_challenge").unwrap_or_default();
        if !well_formed_challenge(&challenge) {
            return Err(refuse(
                "invalid_request",
                "code_challenge must be 43 to 128 unreserved characters",
            ));
        }
        // RFC 8707. Required, not merely checked when present: the MCP
        // specification obliges a client to name the resource it wants a token
        // for, and a token minted without one could not be audience-bound at
        // all — which is the confused-deputy defence the resource server half
        // of this file depends on.
        if query_value(query, "resource").as_deref() != Some(self.resource.as_str()) {
            return Err(refuse(
                "invalid_target",
                "resource must name this browser's MCP endpoint",
            ));
        }
        Ok(ParkedAuthorization {
            id: random_request_id(),
            client_id: client.client_id,
            client_name: client.client_name,
            redirect_uri: presented_redirect,
            code_challenge: challenge,
            code_challenge_method: CHALLENGE_METHOD.to_owned(),
            state,
        })
    }

    /// A refusal as the response the caller receives.
    fn refusal_response(&self, refusal: Refusal) -> Response<GenericBody> {
        match refusal {
            Refusal::Direct { error, description } => {
                oauth_error(StatusCode::BAD_REQUEST, error, description)
            },
            Refusal::Redirect { to, state, error, description } => {
                let redirect = redirect_with(&to, &self.issuer, state.as_deref(), |query| {
                    query.append_pair("error", error);
                    query.append_pair("error_description", description);
                });
                match redirect.and_then(|to| HeaderValue::from_str(&to).ok()) {
                    Some(location) => {
                        let mut headers = HeaderMap::new();
                        headers.insert(header::LOCATION, location);
                        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
                        GenericBody::empty().into_response(StatusCode::FOUND, Some(headers))
                    },
                    // Unreachable — the URI parsed during validation.
                    None => oauth_error(StatusCode::BAD_REQUEST, error, description),
                }
            },
        }
    }

    /// The authorization endpoint.
    ///
    /// Validate, refuse the harassment caps, park, raise the panel, and answer
    /// with a page that can approve nothing. The decision itself happens in
    /// native chrome, where page JavaScript, the `evaluate` tool and the
    /// control socket all structurally cannot reach it — see the panel's own
    /// doc comments in `crate::gui`.
    async fn handle_authorization(
        &self,
        request: &Request<&str>,
        now: u64,
    ) -> Response<GenericBody> {
        // `HEAD` and `OPTIONS` are in this endpoint's verb table and must not
        // park anything: a request that raises a consent panel as a side
        // effect of being probed is a request an attacker sends in a loop.
        if request.method() != Method::GET {
            return GenericBody::empty()
                .into_response(StatusCode::OK, Some(holding_page_headers(ConsentStatus::Pending)));
        }
        let query = request.uri().query().unwrap_or_default();
        let parked = match self.validate_authorization(query) {
            Ok(parked) => parked,
            Err(refusal) => return self.refusal_response(refusal),
        };
        let timings = self.timings;
        // The two harassment caps, checked together and answered without
        // raising anything: the caller is told no, and the human is not
        // interrupted in order to say it.
        {
            let mut consent = match self.consent.lock() {
                Ok(consent) => consent,
                Err(_) => {
                    log::error!("consent state lock is poisoned; refusing authorization");
                    return self.refusal_response(Refusal::Redirect {
                        to: parked.redirect_uri.clone(),
                        state: parked.state.clone(),
                        error: "access_denied",
                        description: "this browser cannot take an authorization request",
                    });
                },
            };
            consent.prune(now);
            if !consent.may_raise(now) {
                log::info!(
                    "refusing an authorization request from client {}: another is on screen \
                     or a cooldown is in force",
                    parked.client_id
                );
                return self.refusal_response(Refusal::Redirect {
                    to: parked.redirect_uri.clone(),
                    state: parked.state.clone(),
                    error: "access_denied",
                    description: "this browser is not taking an authorization request right now",
                });
            }
            consent.parked = Some(parked.id);
        }
        let (reply, decision) = tokio::sync::oneshot::channel();
        let raised = (self.raise_consent)(ConsentRequest {
            id: parked.id,
            client_id: parked.client_id.clone(),
            client_name: parked.client_name.clone(),
            redirect_uri: parked.redirect_uri.clone(),
            raised_at_ms: now,
            reply,
        });
        if raised.is_err() {
            // The event loop is gone; the browser is shutting down. Give the
            // slot straight back rather than leaving it held by a request
            // nobody will ever see.
            if let Ok(mut consent) = self.consent.lock() {
                consent.parked = None;
            }
            return self.refusal_response(Refusal::Redirect {
                to: parked.redirect_uri.clone(),
                state: parked.state.clone(),
                error: "access_denied",
                description: "the browser is no longer accepting authorization requests",
            });
        }
        // The wait happens in a task of its own, not in this handler: the
        // holding page has to reach the caller's browser *now*, and it is what
        // tells the human where to go. Dropping `decision` when this task ends
        // is also what closes the parked request's channel, which is how the
        // panel learns it has been given up on.
        let consent = self.consent.clone();
        let issuer = self.issuer.clone();
        let id = parked.id;
        tokio::spawn(async move {
            let outcome =
                match tokio::time::timeout(Duration::from_millis(timings.lifetime_ms), decision)
                    .await
                {
                    Ok(Ok(ConsentDecision::Approve)) => ConsentOutcome::Approved,
                    Ok(Ok(ConsentDecision::Deny)) => ConsentOutcome::Denied,
                    // The chrome dropped the request without answering it —
                    // treated as a refusal, because the degrade direction here
                    // is deny and a request nobody answered granted nothing.
                    Ok(Err(_)) => ConsentOutcome::Denied,
                    Err(_) => ConsentOutcome::Expired,
                };
            // The clock is read at the moment this settles, not at the moment
            // the request arrived — a difference of up to the whole parked
            // lifetime, which is exactly the interval a cooldown is measured
            // over.
            let settled_at = now_ms();
            match consent.lock() {
                Ok(mut state) => settle(&mut state, &parked, outcome, settled_at, timings, &issuer),
                Err(_) => log::error!("consent state lock is poisoned; a decision was lost"),
            }
        });
        holding_response(ConsentStatus::Pending, id, None)
    }

    /// The path the holding page refreshes itself to.
    ///
    /// Answers with the state's own static page while the request is pending,
    /// and with a `302` to the caller's registered redirect URI once it is
    /// not. The state name also travels in a header, so a programmatic poller
    /// reads one word rather than prose.
    fn handle_authorization_status(&self, query: &str, now_ms: u64) -> Response<GenericBody> {
        let id = query_value(query, "id").and_then(|id| id.parse::<u64>().ok());
        let Some(id) = id else {
            return holding_response(ConsentStatus::Expired, 0, None);
        };
        let (status, redirect) = match self.consent.lock() {
            Ok(mut consent) => {
                consent.prune(now_ms);
                consent.status_of(id)
            },
            Err(_) => {
                log::error!("consent state lock is poisoned; reporting the request as expired");
                (ConsentStatus::Expired, None)
            },
        };
        holding_response(status, id, redirect)
    }

    /// The token endpoint: an authorization code, or a refresh token, becomes
    /// a credential.
    ///
    /// Unauthenticated, like registration, and for the same structural reason:
    /// this browser issues no client secret, because there is nothing a local
    /// agent could keep secret from a human who owns the machine. PKCE stands
    /// in for client authentication, which is why the verifier comparison
    /// below is the load-bearing check and not a formality.
    fn handle_token(&self, body: &str, now_ms: u64) -> Response<GenericBody> {
        match self.token_grant(body, now_ms) {
            Ok(document) => token_response(&document),
            Err((code, error, description)) => oauth_error(code, error, description),
        }
    }

    /// [`TalariaAuth::handle_token`] without the response wrapping, so both
    /// the document a client receives and the refusal it receives instead are
    /// values a test can read and compare — the same split
    /// [`TalariaAuth::register_client`] uses.
    fn token_grant(
        &self,
        body: &str,
        now_ms: u64,
    ) -> Result<serde_json::Value, (StatusCode, &'static str, &'static str)> {
        if body.len() > MAX_TOKEN_REQUEST_BYTES {
            return Err((
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "the token request is larger than this endpoint accepts",
            ));
        }
        // A form-encoded body is a query string by another name — the same
        // `application/x-www-form-urlencoded` production — so it goes through
        // the same parser. A body that is not form-encoded at all simply
        // yields no parameters and lands on a refusal, which is why nothing
        // here has an error path of its own to panic down.
        match query_value(body, "grant_type").as_deref() {
            Some(AUTHORIZATION_CODE_GRANT) => self.redeem_code(body, now_ms),
            Some(REFRESH_TOKEN_GRANT) => self.refresh_grant(body, now_ms),
            // Two arms and no third. OAuth 2.1 removed the implicit and
            // password grants outright, so there is nothing else to support
            // and no deprecated arm to leave lying around.
            _ => Err((
                StatusCode::BAD_REQUEST,
                "unsupported_grant_type",
                "this authorization server implements the authorization-code and \
                 refresh-token grants and nothing else",
            )),
        }
    }

    /// The authorization-code grant.
    ///
    /// **The code is consumed first and validated afterwards**, which is the
    /// order the whole property depends on. [`AuthorizationCodes::take`]
    /// removes and returns in one operation, so two concurrent exchanges of
    /// one code cannot both find it — single-use is then a property of the
    /// store rather than of how fast the two callers happened to be. And a
    /// code that fails any check below stays consumed: a code that survived a
    /// refused redemption would be retryable by whoever intercepted it, which
    /// is precisely T-5's attacker.
    ///
    /// **Re-checking the client and the redirect URI is not duplication.** The
    /// authorization endpoint checked both at minting, but the code then
    /// travelled through a redirect a process racing the loopback port may
    /// have intercepted. The binding made at minting is what makes an
    /// intercepted code useless, and a binding that is never checked is not a
    /// binding. Do not "simplify" either of these away.
    fn redeem_code(
        &self,
        body: &str,
        now_ms: u64,
    ) -> Result<serde_json::Value, (StatusCode, &'static str, &'static str)> {
        let (Some(code), Some(client_id), Some(redirect_uri), Some(verifier)) = (
            query_value(body, "code"),
            query_value(body, "client_id"),
            query_value(body, "redirect_uri"),
            query_value(body, "code_verifier"),
        ) else {
            return Err(grant_refused());
        };
        let record = {
            let mut consent = match self.consent.lock() {
                Ok(consent) => consent,
                Err(_) => {
                    log::error!("consent state lock is poisoned; refusing a code redemption");
                    return Err((
                        StatusCode::SERVICE_UNAVAILABLE,
                        "temporarily_unavailable",
                        "this browser cannot exchange an authorization code right now",
                    ));
                },
            };
            consent.prune(now_ms);
            consent.codes.take(&code, now_ms)
        };
        // Unknown, already redeemed and expired are one answer. Expiry is
        // checked inside `take`, against the same clock the code was stamped
        // from — this module's `now_ms`, which is the only clock in the
        // authorization path.
        let Some(record) = record else {
            return Err(grant_refused());
        };
        if record.client_id != client_id {
            log::warn!(
                "client {client_id} presented an authorization code issued to another client"
            );
            return Err(grant_refused());
        }
        // Exact, with no loopback-port exception. RFC 6749 §4.1.3 requires the
        // redirect URI at redemption to be *identical* to the one in the
        // authorization request, and the code recorded that exact string —
        // `redirect_matches` widened the *registration* comparison by a port
        // and has no business here.
        if record.redirect_uri != redirect_uri {
            log::warn!(
                "client {client_id} presented an authorization code with a redirect URI \
                 other than the one it was issued for"
            );
            return Err(grant_refused());
        }
        // PKCE (RFC 7636). The algorithm is three lines and entirely
        // uninteresting; the two things that go wrong are a comparison that
        // returns early on the first differing byte and a server that accepts
        // the unsafe `plain` method. Both are refused here, and the method was
        // already refused at the authorization endpoint in 04-06 — this is the
        // second gate, and both exist on purpose. If the first were ever
        // edited away, a code carrying anything but S256 would still grant
        // nothing.
        if record.code_challenge_method != CHALLENGE_METHOD {
            log::warn!(
                "client {client_id} presented an authorization code minted under a challenge \
                 method this browser does not accept"
            );
            return Err(grant_refused());
        }
        let Some(expected) = challenge_as_digest(&record.code_challenge) else {
            return Err(grant_refused());
        };
        // `digest_of` and `digests_match` are both `crate::agents`'s, and are
        // deliberately not redefined here: one token-to-digest mapping and one
        // timing-safe comparison in this browser, or the two drift. The
        // ordinary `==` on these two strings is the insecure version — see
        // `agents::digests_match`'s own doc comment for what it leaks.
        if !digests_match(&digest_of(&verifier), &expected) {
            log::warn!("client {client_id} presented a verifier that does not match its challenge");
            return Err(grant_refused());
        }
        let issued = {
            let mut store = match self.agents.lock() {
                Ok(store) => store,
                Err(_) => {
                    log::error!("agent store lock is poisoned; refusing a code redemption");
                    return Err((
                        StatusCode::SERVICE_UNAVAILABLE,
                        "temporarily_unavailable",
                        "the agent store cannot be written",
                    ));
                },
            };
            // The store mints the family, marks the client approved and
            // returns both raw tokens exactly once. The audience is this
            // browser's own canonical identifier, which is what the resource
            // server half then compares byte for byte on every request.
            store.authorize(
                &client_id,
                now_ms,
                &self.resource,
                TALARIA_SCOPE,
                DEFAULT_ACCESS_TTL_MS,
                DEFAULT_REFRESH_TTL_MS,
            )
        };
        // `None` means no client with that id is registered — it was revoked
        // between the authorization and the exchange, say. Same refusal.
        let Some((access_token, refresh_token)) = issued else {
            return Err(grant_refused());
        };
        log::info!("issued a token pair to client {client_id}");
        Ok(token_document(&access_token, &refresh_token))
    }

    /// The refresh-token grant.
    ///
    /// Rotation and reuse detection are the store's, implemented and
    /// unit-tested there in 04-04, including the case that is easy to get
    /// backwards: a client retrying with its **current** token has presented a
    /// token nothing has consumed, and that rotates normally. Only an
    /// already-consumed token is reuse. This function is the protocol that
    /// calls that, not a second implementation of it.
    fn refresh_grant(
        &self,
        body: &str,
        now_ms: u64,
    ) -> Result<serde_json::Value, (StatusCode, &'static str, &'static str)> {
        // The parameter's name and the grant type's name are the same string by
        // RFC 6749's own choice; this is the parameter.
        let Some(presented) = query_value(body, "refresh_token") else {
            return Err(grant_refused());
        };
        let digest = digest_of(&presented);
        let outcome = {
            let mut store = match self.agents.lock() {
                Ok(store) => store,
                Err(_) => {
                    log::error!("agent store lock is poisoned; refusing a refresh");
                    return Err((
                        StatusCode::SERVICE_UNAVAILABLE,
                        "temporarily_unavailable",
                        "the agent store cannot be written",
                    ));
                },
            };
            store.rotate_refresh(&digest, now_ms, DEFAULT_ACCESS_TTL_MS, DEFAULT_REFRESH_TTL_MS)
        };
        match outcome {
            RefreshOutcome::Rotated { access_token, refresh_token } => {
                log::info!("rotated the refresh token {}", digest_prefix(&digest));
                Ok(token_document(&access_token, &refresh_token))
            },
            // The two failures are **indistinguishable on the wire**, and
            // deliberately: a caller who could tell a revoked family from a
            // token that was never issued would have an oracle for both. They
            // are told apart only in the log, which is this browser's own
            // record and not an answer to anybody.
            RefreshOutcome::ReuseDetected => {
                log::warn!(
                    "a consumed refresh token {} was presented again; its family is revoked",
                    digest_prefix(&digest)
                );
                Err(grant_refused())
            },
            RefreshOutcome::Unknown => {
                log::debug!("refusing an unknown refresh token {}", digest_prefix(&digest));
                Err(grant_refused())
            },
        }
    }

    /// Token revocation (RFC 7009).
    ///
    /// **The answer is `200` before the request is read, and that is the
    /// specification's own requirement rather than a shortcut.** RFC 7009 §2.2
    /// says the server responds with `200` for an invalid token as well as a
    /// valid one, and the reason is the same one every other refusal in this
    /// module is built around: an endpoint that distinguished "revoked
    /// something" from "that token was never valid" would tell an
    /// **unauthenticated** caller — this route runs with an empty middleware
    /// chain, like every other auth route — which tokens exist. There is
    /// deliberately no error path here for a caller to read a fact out of, so
    /// a missing parameter, an oversized body, an unparseable form and an
    /// unknown token are one answer.
    ///
    /// The `token_type_hint` parameter is accepted and **ignored**, which the
    /// specification explicitly permits: the store is keyed by digest across
    /// both kinds, so a hint could only ever make the lookup faster, and a
    /// server that trusted a wrong hint would refuse to revoke a real token.
    ///
    /// **Revoking either half takes the whole family**, and that is this
    /// module's decision rather than the specification's — RFC 7009 §2.1 makes
    /// it a SHOULD in one direction (a refresh token takes its access tokens)
    /// and leaves the other to the server. Talaria takes it in both. A client
    /// revoking its own access token has finished with this browser; leaving
    /// its refresh token live would mean the credential it just disowned can
    /// be exchanged straight back into a working pair, which is revocation
    /// that revokes nothing.
    ///
    /// What this endpoint does **not** do is remove the client's registration
    /// or terminate its open streams. That is the human's revoke — see
    /// `crate::gui`'s Access panel and `UiAction::RevokeClient` — and the
    /// difference is who is asking: a client disowning its own credential is
    /// not the same act as a human withdrawing an agent's access.
    fn handle_revocation(&self, body: &str) -> Response<GenericBody> {
        // The return value is deliberately dropped rather than branched on.
        // It exists for the log line inside, and for a unit test that wants to
        // know the store actually changed — never for the response.
        let _ = self.revoke_presented_token(body);
        let mut headers = HeaderMap::new();
        headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        GenericBody::empty().into_response(StatusCode::OK, Some(headers))
    }

    /// [`TalariaAuth::handle_revocation`] without the response, reporting
    /// whether the store actually lost anything.
    ///
    /// Split out for the same reason [`TalariaAuth::register_client`] and
    /// [`TalariaAuth::token_grant`] are: the *effect* is then a value a test
    /// can assert on, while the response above stays a constant that no test
    /// can accidentally make conditional.
    fn revoke_presented_token(&self, body: &str) -> bool {
        if body.len() > MAX_TOKEN_REQUEST_BYTES {
            return false;
        }
        let Some(presented) = query_value(body, "token") else {
            return false;
        };
        let digest = digest_of(&presented);
        let mut store = match self.agents.lock() {
            Ok(store) => store,
            Err(_) => {
                log::error!("agent store lock is poisoned; a revocation was not applied");
                return false;
            },
        };
        // Two statements rather than one chain, because the first borrows the
        // store and the second mutates it. The family id is copied out, which
        // is what ends the first borrow.
        //
        // A record whose `family_id` is empty — only reachable from a stored
        // document an older or hand-edited build wrote, since every minting
        // path sets one — groups with every other ungrouped record. That
        // direction is the safe one for a revocation, and reaching it at all
        // requires holding one of those tokens.
        let family = store
            .tokens()
            .iter()
            .find(|record| digests_match(&record.digest, &digest))
            .map(|record| record.family_id.clone());
        let Some(family_id) = family else {
            // Debug, not warn: presenting a token this server never issued is
            // exactly what a client does when it revokes a credential twice,
            // and it is not evidence of anything.
            log::debug!("revocation named an unknown token {}", digest_prefix(&digest));
            return false;
        };
        let changed = store.revoke_family(&family_id);
        if changed {
            log::info!("revoked the family of token {}", digest_prefix(&digest));
        }
        changed
    }
}

/// The one refusal value, built in one place.
///
/// A function rather than a constant so there is exactly one expression that
/// produces it and no second call site can quietly pass a different
/// description — see [`REFUSAL`].
fn refused() -> AuthenticationError {
    AuthenticationError::InvalidToken { description: REFUSAL }
}

#[async_trait::async_trait]
impl AuthProvider for TalariaAuth {
    async fn verify_token(&self, access_token: String) -> Result<AuthInfo, AuthenticationError> {
        // Note what is *not* here: no length check, no shape check, no
        // "that is not even a Talaria token" fast path. Hashing an arbitrary
        // byte string is cheap and cannot fail, and a fast path would be a
        // side channel that told a caller which of their guesses were the
        // right shape. Every presented value takes the same route.
        self.verify(&access_token, now_ms())
    }

    /// The single scope every issued token carries — see [`TALARIA_SCOPE`].
    ///
    /// The SDK's middleware requires the token to carry **all** of these, so
    /// this is the resource server's own floor, checked after `verify_token`
    /// has already produced an identity.
    fn required_scopes(&self) -> Option<&Vec<String>> {
        Some(&self.scopes)
    }

    fn auth_endpoints(&self) -> Option<&HashMap<String, OauthEndpoint>> {
        Some(&self.endpoints)
    }

    async fn handle_request(
        &self,
        request: Request<&str>,
        _state: Arc<McpAppState>,
    ) -> Result<Response<GenericBody>, McpHttpError> {
        // The verb table is checked here because **nothing else checks it**.
        // `AuthProvider` ships `validate_allowed_methods` with a complete
        // per-endpoint table and builds the `405` for you, and the SDK never
        // calls it — `handle_auth_requests` goes straight to this function.
        // Measured in `04-02-SPIKE.md`, where `GET /token` returned `200`.
        // Every provider is obliged to make this call itself.
        let Some(endpoint) = self.endpoint_type(&request) else {
            // Unreachable while the router is built by folding over the very
            // map `endpoint_type` reads. Answered rather than asserted,
            // because a panic on a network-reachable path is a denial of
            // service waiting for the day that stops being true.
            return Ok(GenericBody::create_404_response());
        };
        if let Some(refusal) = self.validate_allowed_methods(endpoint, request.method()) {
            return Ok(refusal);
        }
        let now = now_ms();
        match endpoint {
            OauthEndpoint::ProtectedResourceMetadata => Ok(self.metadata_response()),
            OauthEndpoint::AuthorizationServerMetadata => {
                let mut headers = HeaderMap::new();
                headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
                Ok(GenericBody::from_value(&self.authorization_server_metadata())
                    .into_json_response(StatusCode::OK, Some(headers)))
            },
            OauthEndpoint::RegistrationEndpoint => {
                Ok(self.handle_registration(request.body(), now))
            },
            OauthEndpoint::TokenEndpoint => Ok(self.handle_token(request.body(), now)),
            OauthEndpoint::RevocationEndpoint => Ok(self.handle_revocation(request.body())),
            // Two paths share this endpoint kind — the authorization endpoint
            // itself and the path its holding page refreshes to. See
            // `AUTHORIZATION_STATUS_PATH` for why that is one endpoint rather
            // than two.
            OauthEndpoint::AuthorizationEndpoint => {
                match request.uri().path() == AUTHORIZATION_STATUS_PATH {
                    true => Ok(self
                        .handle_authorization_status(request.uri().query().unwrap_or_default(), now)),
                    false => Ok(self.handle_authorization(&request, now).await),
                }
            },
            // Unreachable while the router is built by folding over the very
            // map `endpoint_type` reads, and answered rather than asserted for
            // the same reason as above. `OauthEndpoint` has no `Debug`, so
            // there is deliberately nothing here that tries to name it.
            _ => Ok(GenericBody::create_404_response()),
        }
    }

    /// The absolute URL the SDK puts into the `WWW-Authenticate` challenge.
    ///
    /// **This is not decoration.** A client that knows only the MCP endpoint
    /// URL — which is all a human writes into a client's configuration — has
    /// no way to discover the authorization server except by following this
    /// pointer from the `401`. `04-RESEARCH.md`'s Pitfall 3 is a server that
    /// mints and validates tokens while returning a `401` that points nowhere,
    /// and it is why Success Criterion 2 was reworded during planning to name
    /// discovery explicitly.
    ///
    /// 04-03's interim provider returned `None` here on purpose, because the
    /// document did not exist yet and a dangling pointer is worse than none.
    /// It exists now.
    fn protected_resource_metadata_url(&self) -> Option<&str> {
        Some(&self.metadata_url)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use rust_mcp_sdk::mcp_http::http::Method;

    use crate::agents::TokenKind;

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    const BOUND: &str = "127.0.0.1:40501";
    const OTHER: &str = "127.0.0.1:40502";

    /// The `agents.json` path idiom `agents.rs` and `bookmarks.rs` share:
    /// a uniquely-named file in the temp directory that removes itself, and
    /// its `.tmp` staging sibling, when the test ends. `tempfile` is not a
    /// dependency of this crate and this plan may not add one.
    struct TempPath(PathBuf);

    impl TempPath {
        fn new() -> Self {
            let unique = COUNTER.fetch_add(1, Ordering::Relaxed);
            let name = format!("talaria-oauth-{}-{unique}.json", std::process::id());
            Self(std::env::temp_dir().join(name))
        }

        fn path(&self) -> PathBuf {
            self.0.clone()
        }
    }

    impl Drop for TempPath {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
            let name = self.0.file_name().map(|name| name.to_string_lossy().into_owned());
            if let Some(name) = name {
                let _ = fs::remove_file(self.0.with_file_name(format!("{name}.tmp")));
            }
        }
    }

    /// A pinned moment, so nothing here depends on the wall clock.
    const NOW: u64 = 1_700_000_000_000;

    /// A provider over a fresh store, plus the store handle and the client id
    /// its one authorized client was given.
    fn fixture(path: PathBuf) -> (TalariaAuth, SharedAgents, String, String) {
        let mut store = Agents::load_from(path);
        let client = store
            .register("e2e agent".to_owned(), vec!["http://127.0.0.1/cb".to_owned()], NOW)
            .expect("register the fixture client");
        let (access, _refresh) = store
            .authorize(
                &client.client_id,
                NOW,
                &canonical_resource(BOUND),
                TALARIA_SCOPE,
                60_000,
                600_000,
            )
            .expect("authorize the fixture client");
        let agents: SharedAgents = Arc::new(Mutex::new(store));
        let auth = TalariaAuth::new(agents.clone(), Chrome::default().raiser(), BOUND);
        (auth, agents, client.client_id, access)
    }

    /// A stand-in for the browser chrome: it parks whatever the authorization
    /// endpoint hands it, so a test can answer a consent request the way a
    /// human would — by consuming the parked request and sending a decision.
    ///
    /// This is what makes the whole authorization path testable without a
    /// window, and it is why [`ConsentRaiser`] is a callback rather than an
    /// event-loop proxy held in this module.
    #[derive(Clone, Default)]
    struct Chrome(Arc<Mutex<Vec<ConsentRequest>>>);

    impl Chrome {
        fn raiser(&self) -> ConsentRaiser {
            let parked = self.0.clone();
            Arc::new(move |request| {
                parked.lock().map_err(|_| ())?.push(request);
                Ok(())
            })
        }

        /// The most recently raised request, taken off the stand-in chrome.
        fn take(&self) -> Option<ConsentRequest> {
            self.0.lock().ok()?.pop()
        }

        /// How many requests have been raised and not yet answered.
        fn raised(&self) -> usize {
            self.0.lock().map(|parked| parked.len()).unwrap_or_default()
        }
    }

    /// The message the SDK would render for a refusal, which is what the
    /// caller actually sees.
    fn message(error: &AuthenticationError) -> String {
        error.as_json_value().to_string()
    }

    #[test]
    fn a_live_token_for_this_server_yields_an_identity() {
        let temp = TempPath::new();
        let (auth, _agents, client_id, token) = fixture(temp.path());
        let info = auth.verify(&token, NOW).expect("a live token verifies");
        assert_eq!(info.client_id.as_deref(), Some(client_id.as_str()));
        assert_eq!(info.scopes, Some(vec![TALARIA_SCOPE.to_owned()]));
        assert_eq!(info.audience, Some(Audience::Single(canonical_resource(BOUND))));
        assert_eq!(info.token_unique_id, digest_of(&token));
        assert!(!info.token_unique_id.contains(&token), "the identity carries the token itself");
        // Never `None`: the SDK's middleware rejects an `AuthInfo` with no
        // expiry outright, so a token that verified but carried none would be
        // a token that could never be used.
        assert!(info.expires_at.is_some());
    }

    #[test]
    fn an_unknown_token_is_refused() {
        let temp = TempPath::new();
        let (auth, _agents, _client_id, _token) = fixture(temp.path());
        let error = auth.verify("not-a-token-this-browser-ever-issued", NOW).unwrap_err();
        assert_eq!(message(&error), message(&refused()));
    }

    #[test]
    fn an_expired_token_is_refused() {
        let temp = TempPath::new();
        let (auth, _agents, _client_id, token) = fixture(temp.path());
        // The fixture's access token lives 60s; ask an hour later.
        let error = auth.verify(&token, NOW + 3_600_000).unwrap_err();
        assert_eq!(message(&error), message(&refused()));
    }

    #[test]
    fn a_token_minted_for_another_audience_is_refused() {
        let temp = TempPath::new();
        let (auth, agents, client_id, _token) = fixture(temp.path());
        let foreign = agents
            .lock()
            .expect("lock the store")
            .mint_for_test(
                TokenKind::Access,
                &client_id,
                "some-family",
                NOW + 600_000,
                // Minted for a different resource server entirely.
                &canonical_resource(OTHER),
                TALARIA_SCOPE,
            );
        let error = auth.verify(&foreign, NOW).unwrap_err();
        assert_eq!(message(&error), message(&refused()));
        // And the same token verifies against a provider whose canonical
        // identifier *is* that other one — so this test is about the audience
        // check and not about the token being broken.
        let elsewhere = TalariaAuth::new(agents.clone(), Chrome::default().raiser(), OTHER);
        assert!(elsewhere.verify(&foreign, NOW).is_ok());
    }

    #[test]
    fn unknown_expired_and_wrong_audience_are_indistinguishable() {
        let temp = TempPath::new();
        let (auth, agents, client_id, token) = fixture(temp.path());
        let foreign = agents.lock().expect("lock the store").mint_for_test(
            TokenKind::Access,
            &client_id,
            "some-family",
            NOW + 600_000,
            &canonical_resource(OTHER),
            TALARIA_SCOPE,
        );
        let unknown = message(&auth.verify("no-such-token", NOW).unwrap_err());
        let expired = message(&auth.verify(&token, NOW + 3_600_000).unwrap_err());
        let wrong_audience = message(&auth.verify(&foreign, NOW).unwrap_err());
        assert_eq!(unknown, expired);
        assert_eq!(unknown, wrong_audience);
        // And it says nothing about which check failed.
        assert!(!unknown.contains("audience"), "{unknown}");
        assert!(!unknown.contains("expire"), "{unknown}");
    }

    #[test]
    fn a_token_revoked_a_moment_ago_is_refused_by_the_very_next_call() {
        let temp = TempPath::new();
        let (auth, agents, client_id, token) = fixture(temp.path());
        assert!(auth.verify(&token, NOW).is_ok(), "the token works before the revoke");
        assert!(agents.lock().expect("lock the store").revoke_client(&client_id));
        let error = auth.verify(&token, NOW).unwrap_err();
        assert_eq!(message(&error), message(&refused()));
    }

    #[test]
    fn a_refresh_token_is_not_an_access_token() {
        let temp = TempPath::new();
        let (auth, agents, client_id, _token) = fixture(temp.path());
        let refresh = agents.lock().expect("lock the store").mint_for_test(
            TokenKind::Refresh,
            &client_id,
            "some-family",
            NOW + 600_000,
            &canonical_resource(BOUND),
            TALARIA_SCOPE,
        );
        let error = auth.verify(&refresh, NOW).unwrap_err();
        assert_eq!(message(&error), message(&refused()));
    }

    #[test]
    fn an_empty_token_is_refused_rather_than_panicking() {
        let temp = TempPath::new();
        let (auth, _agents, _client_id, _token) = fixture(temp.path());
        assert_eq!(message(&auth.verify("", NOW).unwrap_err()), message(&refused()));
    }

    #[test]
    fn a_multi_megabyte_token_is_refused_rather_than_panicking() {
        let temp = TempPath::new();
        let (auth, _agents, _client_id, _token) = fixture(temp.path());
        let huge = "A".repeat(4 * 1024 * 1024);
        assert_eq!(message(&auth.verify(&huge, NOW).unwrap_err()), message(&refused()));
    }

    #[test]
    fn a_non_ascii_token_is_refused_rather_than_panicking() {
        let temp = TempPath::new();
        let (auth, _agents, _client_id, _token) = fixture(temp.path());
        let odd = "🔑\u{0}\u{feff}αβγ ünïcödé";
        assert_eq!(message(&auth.verify(odd, NOW).unwrap_err()), message(&refused()));
        // And the log-line helper survives it too: `digest_prefix` slices on a
        // character boundary, so a value that is not hex cannot panic it.
        assert!(!digest_prefix(odd).is_empty());
    }

    #[test]
    fn a_broken_clock_refuses_rather_than_admits() {
        let temp = TempPath::new();
        let (auth, _agents, _client_id, token) = fixture(temp.path());
        // `now_ms` saturates to u64::MAX when the clock cannot be read, which
        // must read as "everything has expired", not "nothing has".
        assert_eq!(message(&auth.verify(&token, u64::MAX).unwrap_err()), message(&refused()));
    }

    #[test]
    fn the_metadata_document_names_the_resource_and_an_authorization_server() {
        let temp = TempPath::new();
        let (auth, _agents, _client_id, _token) = fixture(temp.path());
        let document = auth.metadata_document();
        assert_eq!(document["resource"], serde_json::json!(canonical_resource(BOUND)));
        let servers = document["authorization_servers"]
            .as_array()
            .expect("authorization_servers is an array")
            .clone();
        assert_eq!(servers, vec![serde_json::json!(format!("http://{BOUND}"))]);
        assert_eq!(document["bearer_methods_supported"], serde_json::json!(["header"]));
        assert_eq!(document["scopes_supported"], serde_json::json!([TALARIA_SCOPE]));
    }

    #[test]
    fn the_metadata_document_advertises_no_capability_this_browser_lacks() {
        let temp = TempPath::new();
        let (auth, _agents, _client_id, _token) = fixture(temp.path());
        let document = auth.metadata_document();
        // D-04-02: CIMD is deliberately not implemented, so it is deliberately
        // not advertised — a conformant client that prefers it would otherwise
        // be sent down a road that dead-ends.
        assert!(document.get("client_id_metadata_document_supported").is_none(), "{document}");
        // Nor a query-string bearer method, which the specification forbids.
        let methods = document["bearer_methods_supported"].to_string();
        assert!(!methods.contains("query"), "{methods}");
    }

    #[test]
    fn the_metadata_response_is_json_and_uncacheable() {
        let temp = TempPath::new();
        let (auth, _agents, _client_id, _token) = fixture(temp.path());
        let response = auth.metadata_response();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE).and_then(|value| value.to_str().ok()),
            Some("application/json"),
        );
        assert_eq!(
            response.headers().get(header::CACHE_CONTROL).and_then(|value| value.to_str().ok()),
            Some("no-store"),
        );
    }

    #[test]
    fn the_protected_resource_metadata_endpoint_is_declared_and_introspection_never_is() {
        let temp = TempPath::new();
        let (auth, _agents, _client_id, _token) = fixture(temp.path());
        let endpoints = auth.auth_endpoints().expect("endpoints are declared");
        assert!(endpoints.contains_key(OAUTH_PROTECTED_RESOURCE_BASE));
        // Never declared at all, in this plan or any later one. 04-05 checked
        // this as "and nothing else"; the authorization server it named as
        // 04-06's is now declared beside it, so the exhaustive half of that
        // claim moved to `every_declared_endpoint_is_one_this_plan_answers_
        // and_carries_a_verb_table`, which is where the current set is pinned.
        assert!(!endpoints.contains_key("/introspect"));
    }

    #[test]
    fn the_challenge_url_is_absolute_and_is_the_endpoint_that_is_declared() {
        let temp = TempPath::new();
        let (auth, _agents, _client_id, _token) = fixture(temp.path());
        let url = auth.protected_resource_metadata_url().expect("a metadata URL");
        assert_eq!(url, format!("http://{BOUND}{OAUTH_PROTECTED_RESOURCE_BASE}"));
        assert!(url.starts_with("http://"), "{url}");
        // The pointer and the route agree, which is the whole point of the
        // challenge: a client that follows it must arrive somewhere real.
        assert!(auth.auth_endpoints().is_some_and(|map| map
            .contains_key(url.trim_start_matches(&format!("http://{BOUND}")))));
    }

    #[test]
    fn a_wrong_verb_on_the_metadata_endpoint_is_refused_with_405() {
        let temp = TempPath::new();
        let (auth, _agents, _client_id, _token) = fixture(temp.path());
        // The SDK never makes this call for us — see `handle_request`.
        let refusal = auth
            .validate_allowed_methods(&OauthEndpoint::ProtectedResourceMetadata, &Method::POST)
            .expect("POST is not an allowed verb for a metadata document");
        assert_eq!(refusal.status(), StatusCode::METHOD_NOT_ALLOWED);
        assert!(auth
            .validate_allowed_methods(&OauthEndpoint::ProtectedResourceMetadata, &Method::GET)
            .is_none());
    }

    #[test]
    fn the_required_scope_is_the_single_one_every_record_carries() {
        let temp = TempPath::new();
        let (auth, _agents, _client_id, token) = fixture(temp.path());
        let required = auth.required_scopes().expect("a required scope");
        assert_eq!(required, &vec![TALARIA_SCOPE.to_owned()]);
        // And the middleware's "the token must carry all of these" holds for
        // a token this browser actually issued.
        let info = auth.verify(&token, NOW).expect("a live token verifies");
        let carried = info.scopes.unwrap_or_default();
        assert!(required.iter().all(|scope| carried.contains(scope)), "{carried:?}");
    }

    #[test]
    fn the_canonical_identifier_comes_from_the_bound_address_and_nowhere_else() {
        assert_eq!(canonical_resource(BOUND), format!("http://{BOUND}/mcp"));
        assert_ne!(canonical_resource(BOUND), canonical_resource(OTHER));
        // `localhost` is a different byte string, and therefore a different
        // audience. That is the intended reading: the identifier is compared
        // byte for byte, and the listener reports `127.0.0.1:port`.
        assert_ne!(canonical_resource("127.0.0.1:1"), canonical_resource("localhost:1"));
    }

    #[test]
    fn a_log_prefix_is_short_and_is_not_the_whole_digest() {
        let digest = digest_of("some token");
        let prefix = digest_prefix(&digest);
        assert_eq!(prefix.len(), DIGEST_PREFIX_LEN);
        assert!(digest.starts_with(prefix));
        assert_ne!(prefix, digest);
        // A value shorter than the prefix length is returned whole rather
        // than sliced past its end.
        assert_eq!(digest_prefix("abc"), "abc");
        assert_eq!(digest_prefix(""), "");
    }

    // -----------------------------------------------------------------
    // The authorization server: timings, metadata, registration, the
    // validation chain, and the consent flow end to end.
    // -----------------------------------------------------------------

    /// The RFC 7636 example challenge: 43 unreserved characters.
    const CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
    /// The redirect URI the authorization fixture's client registers.
    const REDIRECT: &str = "http://127.0.0.1:8765/callback";

    /// A store with one *registered but unapproved* client, the provider over
    /// it, and the stand-in chrome its consent requests land in.
    fn authorized_fixture(path: PathBuf) -> (TalariaAuth, SharedAgents, Chrome, String) {
        let mut store = Agents::load_from(path);
        let client = store
            .register("Example Agent".to_owned(), vec![REDIRECT.to_owned()], NOW)
            .expect("register the fixture client");
        let agents: SharedAgents = Arc::new(Mutex::new(store));
        let chrome = Chrome::default();
        let auth = TalariaAuth::new(agents.clone(), chrome.raiser(), BOUND);
        (auth, agents, chrome, client.client_id)
    }

    /// An authorization query with every parameter valid, then whatever the
    /// caller wants to override or remove.
    fn authorization_query(client_id: &str, overrides: &[(&str, Option<&str>)]) -> String {
        let resource = canonical_resource(BOUND);
        let mut pairs: Vec<(String, String)> = vec![
            ("response_type".to_owned(), "code".to_owned()),
            ("client_id".to_owned(), client_id.to_owned()),
            ("redirect_uri".to_owned(), REDIRECT.to_owned()),
            ("code_challenge".to_owned(), CHALLENGE.to_owned()),
            ("code_challenge_method".to_owned(), "S256".to_owned()),
            ("resource".to_owned(), resource),
            ("state".to_owned(), "opaque-state-value".to_owned()),
        ];
        for (name, value) in overrides {
            pairs.retain(|(key, _)| key != name);
            if let Some(value) = value {
                pairs.push(((*name).to_owned(), (*value).to_owned()));
            }
        }
        url::form_urlencoded::Serializer::new(String::new()).extend_pairs(pairs).finish()
    }

    /// A `GET` of the authorization endpoint carrying `query`.
    fn authorization_request(query: &str) -> Request<&'static str> {
        Request::builder()
            .method(Method::GET)
            .uri(format!("{AUTHORIZATION_PATH}?{query}"))
            .body("")
            .expect("a well-formed test request")
    }

    /// The `Location` a response redirects to, if it redirects.
    fn location(response: &Response<GenericBody>) -> Option<String> {
        response.headers().get(header::LOCATION)?.to_str().ok().map(str::to_owned)
    }

    /// Timings small enough for a test to wait out, built the way an override
    /// at its floor builds them — through the same clamp, so this is the
    /// shortest any environment variable could make them.
    fn floor_timings() -> ConsentTimings {
        ConsentTimings {
            lifetime_ms: shorten(CONSENT_LIFETIME_MS, Some("0".to_owned())),
            deny_cooldown_ms: shorten(DENY_COOLDOWN_MS, Some("0".to_owned())),
            expiry_cooldown_ms: shorten(EXPIRY_COOLDOWN_MS, Some("0".to_owned())),
        }
    }

    #[test]
    fn an_over_large_timing_override_yields_the_compiled_default() {
        assert_eq!(shorten(CONSENT_LIFETIME_MS, Some("600000".to_owned())), CONSENT_LIFETIME_MS);
        assert_eq!(shorten(DENY_COOLDOWN_MS, Some("99999".to_owned())), DENY_COOLDOWN_MS);
        assert_eq!(shorten(EXPIRY_COOLDOWN_MS, Some("99999".to_owned())), EXPIRY_COOLDOWN_MS);
    }

    #[test]
    fn a_zero_timing_override_yields_the_floor_and_never_zero() {
        assert_eq!(shorten(CONSENT_LIFETIME_MS, Some("0".to_owned())), CONSENT_FLOOR_MS);
        assert_eq!(shorten(DENY_COOLDOWN_MS, Some("0".to_owned())), CONSENT_FLOOR_MS);
        assert_eq!(shorten(EXPIRY_COOLDOWN_MS, Some("0".to_owned())), CONSENT_FLOOR_MS);
        // The floor is what stops a cooldown being switched off, so it is
        // asserted non-zero at compile time rather than at run time.
        const { assert!(CONSENT_FLOOR_MS > 0) };
    }

    #[test]
    fn an_unparseable_timing_override_yields_the_compiled_default() {
        // A negative number does not parse as a `u64`, which is the same
        // branch as a word or an empty string — and all three degrade to the
        // constant rather than to zero or to unbounded.
        for raw in ["-5", "soon", "", "12.5", "9999999999999999999999"] {
            assert_eq!(
                shorten(CONSENT_LIFETIME_MS, Some(raw.to_owned())),
                CONSENT_LIFETIME_MS,
                "{raw:?} should have degraded to the compiled default"
            );
        }
    }

    #[test]
    fn an_absent_override_leaves_every_duration_compiled() {
        assert_eq!(shorten(CONSENT_LIFETIME_MS, None), CONSENT_LIFETIME_MS);
        assert_eq!(shorten(DENY_COOLDOWN_MS, None), DENY_COOLDOWN_MS);
        assert_eq!(shorten(EXPIRY_COOLDOWN_MS, None), EXPIRY_COOLDOWN_MS);
    }

    #[test]
    fn an_override_can_only_shorten_a_duration_never_lengthen_one() {
        // The property, rather than three examples of it: whatever is asked
        // for, what comes back is never longer than the compiled constant and
        // never shorter than the floor.
        for raw in ["1", "199", "200", "5000", "120000", "120001", "4000000"] {
            let value = shorten(CONSENT_LIFETIME_MS, Some(raw.to_owned()));
            assert!(value <= CONSENT_LIFETIME_MS, "{raw:?} lengthened the parked lifetime");
            assert!(value >= CONSENT_FLOOR_MS, "{raw:?} took the parked lifetime under the floor");
        }
    }

    #[test]
    fn the_authorization_server_metadata_advertises_only_the_s256_challenge_method() {
        let temp = TempPath::new();
        let (auth, _agents, _chrome, _client_id) = authorized_fixture(temp.path());
        let document = auth.authorization_server_metadata();
        assert_eq!(
            document["code_challenge_methods_supported"],
            serde_json::json!(["S256"]),
            "a client that saw `plain` here would be entitled to use it"
        );
        assert_eq!(document["issuer"], serde_json::json!(format!("http://{BOUND}")));
        assert_eq!(
            document["grant_types_supported"],
            serde_json::json!(["authorization_code", "refresh_token"])
        );
        assert_eq!(document["response_types_supported"], serde_json::json!(["code"]));
    }

    #[test]
    fn the_authorization_server_metadata_claims_no_client_metadata_document_capability() {
        let temp = TempPath::new();
        let (auth, _agents, _chrome, _client_id) = authorized_fixture(temp.path());
        let document = auth.authorization_server_metadata();
        let object = document.as_object().expect("the metadata document is an object");
        // Asserted on the rendered document rather than by a source grep,
        // because the absence has to hold in the JSON a client receives.
        assert!(!object.contains_key("client_id_metadata_document_supported"), "{document}");
        // And nothing else claims a capability this phase did not build.
        // Revocation *is* built, and is asserted positively in
        // `the_metadata_names_the_revocation_endpoint_it_answers`.
        assert!(!object.contains_key("introspection_endpoint"), "never built; {document}");
    }

    #[test]
    fn the_issuer_parameter_is_advertised_because_it_is_emitted() {
        let temp = TempPath::new();
        let (auth, _agents, _chrome, _client_id) = authorized_fixture(temp.path());
        let document = auth.authorization_server_metadata();
        assert_eq!(document["authorization_response_iss_parameter_supported"], true);
        // The other half of the agreement: what the metadata claims is what
        // the redirect actually carries.
        let redirect = redirect_with(REDIRECT, &auth.issuer, None, |query| {
            query.append_pair("code", "anything");
        })
        .expect("the fixture redirect parses");
        assert!(redirect.contains("iss="), "{redirect}");
    }

    #[test]
    fn every_endpoint_the_metadata_names_is_one_this_provider_declares() {
        let temp = TempPath::new();
        let (auth, _agents, _chrome, _client_id) = authorized_fixture(temp.path());
        let document = auth.authorization_server_metadata();
        let declared = auth.auth_endpoints().expect("endpoints are declared");
        for key in ["authorization_endpoint", "token_endpoint", "registration_endpoint"] {
            let named = document[key].as_str().expect("a string endpoint").to_owned();
            let path = named.strip_prefix(&auth.issuer).expect("an absolute URL at this issuer");
            assert!(declared.contains_key(path), "{key} names {path}, which routes nowhere");
        }
        // And the poll path the holding page refreshes to is routed too.
        assert!(declared.contains_key(AUTHORIZATION_STATUS_PATH));
    }

    #[test]
    fn registering_mints_an_identifier_the_caller_did_not_choose() {
        let temp = TempPath::new();
        let (auth, agents, _chrome, _client_id) = authorized_fixture(temp.path());
        let document = auth
            .register_client(
                &serde_json::json!({
                    "client_name": "Second Agent",
                    "redirect_uris": ["http://127.0.0.1:9/cb"],
                })
                .to_string(),
                NOW,
            )
            .expect("a well-formed registration");
        let client_id = document["client_id"].as_str().expect("a client id").to_owned();
        assert!(!client_id.is_empty());
        assert_eq!(document["client_name"], serde_json::json!("Second Agent"));
        assert_eq!(document["token_endpoint_auth_method"], serde_json::json!("none"));
        // Registered, and *inert*: no human has approved it, so it holds no
        // authorization time and therefore no tokens.
        let store = agents.lock().expect("the store");
        let stored = store
            .clients()
            .iter()
            .find(|client| client.client_id == client_id)
            .expect("the registration is on file");
        assert!(stored.authorized_at_ms.is_none(), "a registration is inert until approved");
        assert!(store.tokens().is_empty(), "a registration alone minted a token");
    }

    #[test]
    fn a_registration_with_one_inadmissible_redirect_uri_is_refused_whole() {
        let temp = TempPath::new();
        let (auth, agents, _chrome, _client_id) = authorized_fixture(temp.path());
        let before = agents.lock().expect("the store").len();
        let refusal = auth
            .register_client(
                &serde_json::json!({
                    "client_name": "Mixed",
                    "redirect_uris": ["https://example.com/cb", "file:///etc/passwd"],
                })
                .to_string(),
                NOW,
            )
            .expect_err("a list with one bad entry is refused");
        assert_eq!(refusal.1, "invalid_redirect_uri");
        assert_eq!(
            agents.lock().expect("the store").len(),
            before,
            "the admissible half of the list was stored anyway"
        );
    }

    #[test]
    fn a_registration_with_a_non_loopback_http_redirect_is_refused() {
        let temp = TempPath::new();
        let (auth, _agents, _chrome, _client_id) = authorized_fixture(temp.path());
        for uri in [
            "http://example.com/cb",
            // The loopback exception is for IP literals: what `localhost`
            // resolves to is the resolver's business, not this browser's.
            "http://localhost:9/cb",
            "https://example.com/cb#fragment",
            "javascript:alert(1)",
            "not a url",
        ] {
            let refusal = auth
                .register_client(
                    &serde_json::json!({ "redirect_uris": [uri] }).to_string(),
                    NOW,
                )
                .expect_err("{uri} should have been refused");
            assert_eq!(refusal.1, "invalid_redirect_uri", "{uri}");
        }
        // And the two admissible shapes are admitted.
        assert!(admissible_redirect("https://example.com/cb"));
        assert!(admissible_redirect("http://127.0.0.1:0/cb"));
        assert!(admissible_redirect("http://[::1]:1234/cb"));
    }

    /// CR-04, at the endpoint rather than at the store: an unauthenticated
    /// flood cannot wedge registration, and cannot take a human's decision.
    ///
    /// `/register` needs no credential by design, so thirty-two POSTs used to
    /// fill the registry for good — refused past the cap, no expiry, and no
    /// Access-panel row to revoke, because the panel lists approved clients
    /// only. The human saw "Nothing authorized yet" and the only recovery was
    /// to quit the browser and edit `agents.json`.
    #[test]
    fn a_registration_flood_displaces_unapproved_entries_and_never_an_approved_client() {
        let temp = TempPath::new();
        let (auth, agents, _chrome, first) = authorized_fixture(temp.path());
        // A human approved this one. Nothing an unauthenticated flood does may
        // take that away — the half of the old behaviour that was right.
        agents
            .lock()
            .expect("the store")
            .authorize(
                &first,
                NOW,
                &canonical_resource(BOUND),
                TALARIA_SCOPE,
                crate::agents::DEFAULT_ACCESS_TTL_MS,
                crate::agents::DEFAULT_REFRESH_TTL_MS,
            )
            .expect("a registered client can be authorized");
        let body =
            serde_json::json!({ "redirect_uris": ["http://127.0.0.1:9/cb"] }).to_string();

        // Well past the cap, all at the same instant, which is what a `fetch`
        // loop or a two-line shell script produces. Every one is accepted:
        // there is always an unapproved entry to displace.
        for _ in 1..(crate::agents::MAX_REGISTERED_CLIENTS + 16) {
            auth.register_client(&body, NOW)
                .expect("the endpoint refused a registration while the flood held slots");
        }

        let store = agents.lock().expect("the store");
        assert_eq!(store.len(), crate::agents::MAX_REGISTERED_CLIENTS, "the cap moved");
        assert!(
            store.clients().iter().any(|client| client.client_id == first),
            "a flood displaced the client a human had approved"
        );
    }

    #[test]
    fn a_loopback_redirect_on_another_port_matches_the_registration() {
        // RFC 8252: a native client binds an ephemeral port and cannot know
        // its number at registration time, so the port is the one field the
        // match ignores — and only for a loopback literal.
        assert!(redirect_matches("http://127.0.0.1:8765/cb", "http://127.0.0.1:41999/cb"));
        assert!(redirect_matches("http://[::1]:1/cb", "http://[::1]:65535/cb"));
    }

    #[test]
    fn a_non_loopback_port_difference_is_refused() {
        assert!(!redirect_matches("https://example.com:443/cb", "https://example.com:8443/cb"));
        assert!(redirect_matches("https://example.com/cb", "https://example.com/cb"));
    }

    #[test]
    fn a_redirect_uri_that_differs_anywhere_but_the_loopback_port_is_refused() {
        assert!(!redirect_matches("http://127.0.0.1:1/cb", "http://127.0.0.1:1/other"));
        assert!(!redirect_matches("http://127.0.0.1:1/cb", "http://127.0.0.2:1/cb"));
        assert!(!redirect_matches("http://127.0.0.1:1/cb", "https://127.0.0.1:1/cb"));
        assert!(!redirect_matches("http://127.0.0.1:1/cb", "http://127.0.0.1:1/cb?extra=1"));
        assert!(!redirect_matches("http://127.0.0.1:1/cb", "http://evil@127.0.0.1:1/cb"));
        assert!(!redirect_matches("http://127.0.0.1:1/cb", "nonsense"));
    }

    #[test]
    fn an_unknown_client_is_refused_before_any_panel_is_raised() {
        let temp = TempPath::new();
        let (auth, _agents, chrome, _client_id) = authorized_fixture(temp.path());
        let query = authorization_query("0123456789abcdef0123456789abcdef", &[]);
        let refusal = auth.validate_authorization(&query).expect_err("refused");
        assert!(matches!(refusal, Refusal::Direct { .. }), "an unverified redirect was answered to");
        assert_eq!(chrome.raised(), 0, "a panel was raised for an unknown client");
    }

    #[test]
    fn a_mismatched_redirect_uri_is_refused_before_any_panel_is_raised() {
        let temp = TempPath::new();
        let (auth, _agents, chrome, client_id) = authorized_fixture(temp.path());
        let query =
            authorization_query(&client_id, &[("redirect_uri", Some("https://evil.example/cb"))]);
        let refusal = auth.validate_authorization(&query).expect_err("refused");
        // Direct, never a redirect: answering an unverified redirect URI
        // would make this endpoint an open redirector.
        assert!(matches!(refusal, Refusal::Direct { .. }));
        assert_eq!(chrome.raised(), 0);
    }

    #[test]
    fn an_unknown_client_and_a_mismatched_redirect_are_one_refusal() {
        let temp = TempPath::new();
        let (auth, _agents, _chrome, client_id) = authorized_fixture(temp.path());
        let unknown = authorization_query("0123456789abcdef0123456789abcdef", &[]);
        let mismatched =
            authorization_query(&client_id, &[("redirect_uri", Some("https://evil.example/cb"))]);
        let describe = |query: &str| match auth.validate_authorization(query) {
            Err(Refusal::Direct { error, description }) => format!("{error}:{description}"),
            _ => "unrefused".to_owned(),
        };
        assert_eq!(
            describe(&unknown),
            describe(&mismatched),
            "the endpoint distinguishes an unknown client from a wrong redirect URI, \
             which makes it an oracle for which client ids are registered"
        );
    }

    #[test]
    fn an_unsafe_challenge_method_is_refused() {
        let temp = TempPath::new();
        let (auth, _agents, chrome, client_id) = authorized_fixture(temp.path());
        // `plain` accepts a "verifier" equal to the challenge, which is no
        // protection at all. It is refused along with every other value that
        // is not S256, and by omission rather than by an arm naming it.
        for method in ["plain", "PLAIN", "s256", "S512", ""] {
            let query =
                authorization_query(&client_id, &[("code_challenge_method", Some(method))]);
            let refusal = auth.validate_authorization(&query).expect_err("refused");
            assert!(matches!(refusal, Refusal::Redirect { error: "invalid_request", .. }), "{method}");
        }
        let absent = authorization_query(&client_id, &[("code_challenge_method", None)]);
        assert!(auth.validate_authorization(&absent).is_err());
        assert_eq!(chrome.raised(), 0, "an unsafe challenge method reached a human");
    }

    #[test]
    fn a_missing_or_malformed_challenge_is_refused() {
        let temp = TempPath::new();
        let (auth, _agents, chrome, client_id) = authorized_fixture(temp.path());
        let absent = authorization_query(&client_id, &[("code_challenge", None)]);
        assert!(auth.validate_authorization(&absent).is_err());
        for challenge in ["short", &"a".repeat(129), "has spaces in it", &"+".repeat(43)] {
            let query = authorization_query(&client_id, &[("code_challenge", Some(challenge))]);
            assert!(auth.validate_authorization(&query).is_err(), "{challenge}");
        }
        assert!(well_formed_challenge(CHALLENGE));
        assert_eq!(chrome.raised(), 0);
    }

    #[test]
    fn the_wrong_resource_parameter_is_refused() {
        let temp = TempPath::new();
        let (auth, _agents, chrome, client_id) = authorized_fixture(temp.path());
        for resource in [canonical_resource(OTHER), "http://127.0.0.1:40501".to_owned()] {
            let query = authorization_query(&client_id, &[("resource", Some(&resource))]);
            let refusal = auth.validate_authorization(&query).expect_err("refused");
            assert!(matches!(refusal, Refusal::Redirect { error: "invalid_target", .. }));
        }
        let absent = authorization_query(&client_id, &[("resource", None)]);
        assert!(auth.validate_authorization(&absent).is_err(), "resource is required");
        assert_eq!(chrome.raised(), 0);
    }

    #[test]
    fn a_response_type_other_than_code_is_refused() {
        let temp = TempPath::new();
        let (auth, _agents, chrome, client_id) = authorized_fixture(temp.path());
        for response_type in ["token", "id_token", "code token", ""] {
            let query =
                authorization_query(&client_id, &[("response_type", Some(response_type))]);
            let refusal = auth.validate_authorization(&query).expect_err("refused");
            assert!(
                matches!(refusal, Refusal::Redirect { error: "unsupported_response_type", .. }),
                "{response_type}"
            );
        }
        assert_eq!(chrome.raised(), 0);
    }

    #[test]
    fn a_valid_request_is_accepted_and_carries_the_registered_name_as_a_claim() {
        let temp = TempPath::new();
        let (auth, _agents, _chrome, client_id) = authorized_fixture(temp.path());
        let query = authorization_query(&client_id, &[]);
        let parked = auth.validate_authorization(&query).expect("a valid request");
        assert_eq!(parked.client_id, client_id);
        assert_eq!(parked.client_name, "Example Agent");
        assert_eq!(parked.redirect_uri, REDIRECT);
        assert_eq!(parked.code_challenge, CHALLENGE);
        assert_eq!(parked.state.as_deref(), Some("opaque-state-value"));
    }

    #[test]
    fn the_holding_page_carries_no_request_derived_value_and_no_control() {
        for status in [
            ConsentStatus::Pending,
            ConsentStatus::Approved,
            ConsentStatus::Denied,
            ConsentStatus::Expired,
        ] {
            let page = holding_page(status, 1234567890);
            for control in ["<form", "<button", "<a ", "<input", "<script", "onclick"] {
                assert!(!page.contains(control), "{control} in the {} page", status.as_str());
            }
            for supplied in ["Example Agent", REDIRECT, CHALLENGE, "opaque-state-value"] {
                assert!(!page.contains(supplied), "a request value reached the holding page");
            }
        }
        // Two states, byte for byte the same page apart from the id: the
        // body is chosen by the state and by nothing the caller sent.
        assert_eq!(holding_page(ConsentStatus::Denied, 1), holding_page(ConsentStatus::Denied, 2));
    }

    #[test]
    fn the_holding_page_carries_the_framing_and_caching_headers() {
        let response = holding_response(ConsentStatus::Pending, 7, None);
        let header_value = |name: header::HeaderName| {
            response.headers().get(name).and_then(|value| value.to_str().ok()).unwrap_or_default()
        };
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(header_value(header::CONTENT_TYPE), "text/html; charset=utf-8");
        assert_eq!(header_value(header::CACHE_CONTROL), "no-store");
        assert_eq!(
            header_value(header::CONTENT_SECURITY_POLICY),
            "default-src 'none'; frame-ancestors 'none'"
        );
        assert_eq!(header_value(header::X_FRAME_OPTIONS), "DENY");
        assert_eq!(header_value(header::REFERRER_POLICY), "no-referrer");
    }

    #[test]
    fn a_code_is_removed_and_returned_in_one_operation_and_carries_its_binding() {
        let mut codes = AuthorizationCodes::default();
        let code = codes.mint("client-1", CHALLENGE, CHALLENGE_METHOD, REDIRECT, NOW + 60_000);
        let record = codes.take(&code, NOW).expect("a live code redeems");
        assert_eq!(record.client_id, "client-1");
        assert_eq!(record.code_challenge, CHALLENGE);
        assert_eq!(record.redirect_uri, REDIRECT);
        // Gone in the same operation that returned it — which is what makes
        // single-use a property of the store rather than of the caller.
        assert!(codes.take(&code, NOW).is_none(), "a code was redeemable twice");
    }

    #[test]
    fn an_expired_code_is_not_returned() {
        let mut codes = AuthorizationCodes::default();
        let code = codes.mint("client-1", CHALLENGE, CHALLENGE_METHOD, REDIRECT, NOW + 60_000);
        assert!(codes.take(&code, NOW + 60_001).is_none());
        let live = codes.mint("client-1", CHALLENGE, CHALLENGE_METHOD, REDIRECT, NOW + 60_000);
        codes.prune(NOW + 60_001);
        assert!(codes.entries.is_empty(), "pruning left an expired code behind");
        assert!(codes.take(&live, NOW).is_none());
    }

    #[tokio::test]
    async fn a_valid_request_raises_exactly_one_consent_request_and_answers_a_holding_page() {
        let temp = TempPath::new();
        let (auth, _agents, chrome, client_id) = authorized_fixture(temp.path());
        let query = authorization_query(&client_id, &[]);
        let response = auth.handle_authorization(&authorization_request(&query), now_ms()).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert!(location(&response).is_none(), "the caller was redirected before deciding");
        assert_eq!(chrome.raised(), 1);
        let request = chrome.take().expect("one parked request");
        assert_eq!(request.client_id, client_id);
        assert_eq!(request.client_name, "Example Agent");
        assert_eq!(request.redirect_uri, REDIRECT);
        assert!(!request.is_abandoned(), "the HTTP side gave up immediately");
    }

    #[tokio::test]
    async fn a_second_authorization_while_one_is_parked_is_refused_without_raising_anything() {
        let temp = TempPath::new();
        let (auth, _agents, chrome, client_id) = authorized_fixture(temp.path());
        let query = authorization_query(&client_id, &[]);
        let first = auth.handle_authorization(&authorization_request(&query), now_ms()).await;
        assert_eq!(first.status(), StatusCode::OK);
        let second = auth.handle_authorization(&authorization_request(&query), now_ms()).await;
        assert_eq!(second.status(), StatusCode::FOUND);
        let redirect = location(&second).expect("the refusal goes to the registered URI");
        assert!(redirect.contains("error=access_denied"), "{redirect}");
        assert_eq!(chrome.raised(), 1, "a second panel was raised while one was on screen");
    }

    #[tokio::test]
    async fn approving_mints_a_code_and_redirects_to_the_registered_uri_with_the_state() {
        let temp = TempPath::new();
        let (mut auth, agents, chrome, client_id) = authorized_fixture(temp.path());
        auth.timings = floor_timings();
        let query = authorization_query(&client_id, &[]);
        let raised_at = now_ms();
        auth.handle_authorization(&authorization_request(&query), raised_at).await;
        let request = chrome.take().expect("one parked request");
        let id = request.id;
        request.resolve(ConsentDecision::Approve);
        tokio::time::sleep(Duration::from_millis(50)).await;

        let status = auth
            .handle_authorization_status(&format!("id={id}"), now_ms());
        assert_eq!(status.status(), StatusCode::FOUND);
        let redirect = location(&status).expect("a redirect back to the client");
        assert!(redirect.starts_with(REDIRECT), "{redirect}");
        assert!(redirect.contains("code="), "{redirect}");
        assert!(redirect.contains("state=opaque-state-value"), "{redirect}");
        assert!(redirect.contains("iss="), "{redirect}");
        assert!(!redirect.contains("error="), "{redirect}");

        // Bound at minting to everything redemption re-checks.
        let mut consent = auth.consent.lock().expect("the consent state");
        let code = Url::parse(&redirect)
            .expect("the redirect parses")
            .query_pairs()
            .find(|(key, _)| key == "code")
            .map(|(_, value)| value.into_owned())
            .expect("a code");
        let record = consent.codes.take(&code, now_ms()).expect("the code is live");
        assert_eq!(record.client_id, client_id);
        assert_eq!(record.code_challenge, CHALLENGE);
        assert_eq!(record.redirect_uri, REDIRECT);
        // An approval mints a *code*, never a token. The exchange happens at
        // the token endpoint, and until it does the client holds nothing —
        // which is also why a code alone is worth intercepting only to
        // somebody who has the verifier.
        assert!(agents.lock().expect("the store").tokens().is_empty());
    }

    #[tokio::test]
    async fn an_approval_arms_no_cooldown() {
        let temp = TempPath::new();
        let (mut auth, _agents, chrome, client_id) = authorized_fixture(temp.path());
        auth.timings = floor_timings();
        let query = authorization_query(&client_id, &[]);
        auth.handle_authorization(&authorization_request(&query), now_ms()).await;
        chrome.take().expect("one parked request").resolve(ConsentDecision::Approve);
        tokio::time::sleep(Duration::from_millis(50)).await;
        // A client that just received a code has no reason to ask again — and
        // if it does, the one-on-screen cap is what answers, not a cooldown.
        let next = auth.handle_authorization(&authorization_request(&query), now_ms()).await;
        assert_eq!(next.status(), StatusCode::OK, "an approval armed a cooldown");
        assert_eq!(chrome.raised(), 1);
    }

    #[tokio::test]
    async fn denying_returns_access_denied_and_arms_the_shorter_cooldown() {
        let temp = TempPath::new();
        let (mut auth, agents, chrome, client_id) = authorized_fixture(temp.path());
        auth.timings = ConsentTimings { deny_cooldown_ms: 3_000, ..floor_timings() };
        let query = authorization_query(&client_id, &[]);
        auth.handle_authorization(&authorization_request(&query), now_ms()).await;
        let request = chrome.take().expect("one parked request");
        let id = request.id;
        request.resolve(ConsentDecision::Deny);
        tokio::time::sleep(Duration::from_millis(50)).await;

        let status = auth.handle_authorization_status(&format!("id={id}"), now_ms());
        let redirect = location(&status).expect("the denial goes back to the client");
        assert!(redirect.contains("error=access_denied"), "{redirect}");
        assert!(!redirect.contains("code="), "a denial minted a code");
        // The cooldown is in force, and no panel is raised to say so.
        let next = auth.handle_authorization(&authorization_request(&query), now_ms()).await;
        assert_eq!(next.status(), StatusCode::FOUND);
        assert!(location(&next).unwrap_or_default().contains("error=access_denied"));
        assert_eq!(chrome.raised(), 0, "the cooldown still interrupted the human");
        assert!(agents.lock().expect("the store").tokens().is_empty());
    }

    #[tokio::test]
    async fn an_unanswered_request_expires_and_arms_the_longer_cooldown() {
        let temp = TempPath::new();
        let (mut auth, _agents, chrome, client_id) = authorized_fixture(temp.path());
        auth.timings = ConsentTimings { expiry_cooldown_ms: 3_000, ..floor_timings() };
        let query = authorization_query(&client_id, &[]);
        auth.handle_authorization(&authorization_request(&query), now_ms()).await;
        let request = chrome.take().expect("one parked request");
        let id = request.id;
        // Nobody answers. The HTTP side gives up on the parked lifetime and
        // drops its receiver, which is how the panel learns to close itself.
        tokio::time::sleep(Duration::from_millis(floor_timings().lifetime_ms + 200)).await;
        assert!(request.is_abandoned(), "the parked request was never given up on");

        let status = auth.handle_authorization_status(&format!("id={id}"), now_ms());
        let redirect = location(&status).expect("the expiry goes back to the client");
        assert!(redirect.contains("error=access_denied"), "{redirect}");
        // The expiry cooldown is what a denial-only cooldown would have left
        // open: a peer could otherwise wait out a human and re-raise at once.
        let next = auth.handle_authorization(&authorization_request(&query), now_ms()).await;
        assert_eq!(next.status(), StatusCode::FOUND);
        assert_eq!(chrome.raised(), 0);
    }

    #[tokio::test]
    async fn with_every_timing_override_at_its_floor_an_unanswered_request_is_still_denied() {
        let temp = TempPath::new();
        let (mut auth, agents, chrome, client_id) = authorized_fixture(temp.path());
        // The shortest any override can make them, built through the same
        // clamp the environment goes through. This is the whole question the
        // knob raises: can *any* combination of overrides approve something?
        auth.timings = floor_timings();
        let query = authorization_query(&client_id, &[]);
        auth.handle_authorization(&authorization_request(&query), now_ms()).await;
        let request = chrome.take().expect("one parked request");
        let id = request.id;
        drop(request);
        tokio::time::sleep(Duration::from_millis(floor_timings().lifetime_ms + 200)).await;

        let status = auth.handle_authorization_status(&format!("id={id}"), now_ms());
        let redirect = location(&status).unwrap_or_default();
        assert!(redirect.contains("error=access_denied"), "an unanswered request was granted");
        assert!(!redirect.contains("code="), "an unanswered request minted a code");
        assert!(
            auth.consent.lock().expect("the consent state").codes.entries.is_empty(),
            "an unanswered request minted a code"
        );
        let store = agents.lock().expect("the store");
        assert!(
            store.clients().iter().all(|client| client.authorized_at_ms.is_none()),
            "an unanswered request marked a client authorized"
        );
        assert!(store.tokens().is_empty());
    }

    #[tokio::test]
    async fn a_request_whose_status_nothing_remembers_reads_as_expired() {
        let temp = TempPath::new();
        let (auth, _agents, _chrome, _client_id) = authorized_fixture(temp.path());
        for query in ["id=99999999", "id=not-a-number", ""] {
            let status = auth.handle_authorization_status(query, now_ms());
            assert_eq!(status.status(), StatusCode::OK);
            assert!(location(&status).is_none());
            assert_eq!(
                status.headers().get("x-talaria-consent").and_then(|value| value.to_str().ok()),
                Some("expired"),
                "{query}"
            );
        }
    }

    #[tokio::test]
    async fn probing_the_authorization_endpoint_raises_nothing() {
        let temp = TempPath::new();
        let (auth, _agents, chrome, client_id) = authorized_fixture(temp.path());
        let query = authorization_query(&client_id, &[]);
        for method in [Method::HEAD, Method::OPTIONS] {
            let request = Request::builder()
                .method(method.clone())
                .uri(format!("{AUTHORIZATION_PATH}?{query}"))
                .body("")
                .expect("a well-formed test request");
            let response = auth.handle_authorization(&request, now_ms()).await;
            assert_eq!(response.status(), StatusCode::OK, "{method}");
        }
        assert_eq!(chrome.raised(), 0, "a probe raised a consent panel");
    }

    #[tokio::test]
    async fn a_request_raised_while_the_chrome_is_gone_is_refused_and_gives_the_slot_back() {
        let temp = TempPath::new();
        let mut store = Agents::load_from(temp.path());
        let client = store
            .register("Example Agent".to_owned(), vec![REDIRECT.to_owned()], NOW)
            .expect("register the fixture client");
        let agents: SharedAgents = Arc::new(Mutex::new(store));
        // The browser is shutting down: nothing is there to raise a panel.
        let closed: ConsentRaiser = Arc::new(|_| Err(()));
        let auth = TalariaAuth::new(agents, closed, BOUND);
        let query = authorization_query(&client.client_id, &[]);
        let response = auth.handle_authorization(&authorization_request(&query), now_ms()).await;
        assert_eq!(response.status(), StatusCode::FOUND);
        assert!(location(&response).unwrap_or_default().contains("error=access_denied"));
        // The slot is back: a request that nobody ever saw does not hold the
        // one-on-screen cap against the next one.
        assert!(auth.consent.lock().expect("the consent state").parked.is_none());
    }

    // -----------------------------------------------------------------
    // The token endpoint: redemption, PKCE, single use, and rotation.
    // -----------------------------------------------------------------

    /// The verifier RFC 7636 Appendix B pairs with [`CHALLENGE`].
    ///
    /// The specification's own example rather than one this suite generated,
    /// so a reader can check the pairing against the document instead of
    /// against this file.
    const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";

    /// A form-encoded token request body, with whatever a caller wants to
    /// override or remove — the same shape [`authorization_query`] has.
    fn token_body(pairs: &[(&str, &str)]) -> String {
        url::form_urlencoded::Serializer::new(String::new()).extend_pairs(pairs).finish()
    }

    /// A complete authorization-code exchange for `code`, then whatever a
    /// caller wants to break about it.
    fn exchange_body(code: &str, client_id: &str, overrides: &[(&str, &str)]) -> String {
        let mut pairs: Vec<(String, String)> = vec![
            ("grant_type".to_owned(), AUTHORIZATION_CODE_GRANT.to_owned()),
            ("code".to_owned(), code.to_owned()),
            ("client_id".to_owned(), client_id.to_owned()),
            ("redirect_uri".to_owned(), REDIRECT.to_owned()),
            ("code_verifier".to_owned(), VERIFIER.to_owned()),
        ];
        for (name, value) in overrides {
            pairs.retain(|(key, _)| key != name);
            pairs.push(((*name).to_owned(), (*value).to_owned()));
        }
        url::form_urlencoded::Serializer::new(String::new()).extend_pairs(pairs).finish()
    }

    /// Put a code straight into the provider's store, bound to whatever this
    /// test wants to prove redemption re-checks.
    ///
    /// The same [`AuthorizationCodes::mint`] an approval calls, reached
    /// through the same lock — so this stages the state an approval would have
    /// left without spending a consent round trip on every assertion. One test
    /// below does drive the whole approval path, so the seam between the two
    /// halves is pinned rather than assumed.
    fn stage_code(auth: &TalariaAuth, client_id: &str, method: &str, redirect: &str) -> String {
        auth.consent
            .lock()
            .expect("the consent state")
            .codes
            .mint(client_id, CHALLENGE, method, redirect, NOW + AUTHORIZATION_CODE_LIFETIME_MS)
    }

    /// A raw string field of a token document.
    fn field(document: &serde_json::Value, name: &str) -> String {
        document
            .get(name)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned()
    }

    #[test]
    fn a_code_becomes_a_pair_carrying_the_type_the_expiry_and_the_scope() {
        let temp = TempPath::new();
        let (auth, agents, _chrome, client_id) = authorized_fixture(temp.path());
        let code = stage_code(&auth, &client_id, CHALLENGE_METHOD, REDIRECT);
        let document = auth
            .token_grant(&exchange_body(&code, &client_id, &[]), NOW)
            .expect("a correct exchange");
        assert!(!field(&document, "access_token").is_empty(), "{document}");
        assert!(!field(&document, "refresh_token").is_empty(), "{document}");
        assert_eq!(field(&document, "token_type"), "Bearer");
        assert_eq!(field(&document, "scope"), TALARIA_SCOPE);
        assert_eq!(document.get("expires_in").and_then(serde_json::Value::as_u64), Some(3_600));
        // The pair came from the store's own minting, and the access token
        // resolves through the very lookup every request goes through.
        let access = field(&document, "access_token");
        let info = auth.verify(&access, NOW).expect("the issued token verifies");
        assert_eq!(info.client_id.as_deref(), Some(client_id.as_str()));
        let store = agents.lock().expect("the store");
        assert_eq!(store.tokens().len(), 2, "one pair, not one token and not three");
    }

    #[tokio::test]
    async fn a_real_approval_produces_a_code_the_token_endpoint_redeems() {
        let temp = TempPath::new();
        let (mut auth, _agents, chrome, client_id) = authorized_fixture(temp.path());
        auth.timings = floor_timings();
        let query = authorization_query(&client_id, &[]);
        auth.handle_authorization(&authorization_request(&query), now_ms()).await;
        let request = chrome.take().expect("one parked request");
        let id = request.id;
        request.resolve(ConsentDecision::Approve);
        tokio::time::sleep(Duration::from_millis(50)).await;
        let status = auth.handle_authorization_status(&format!("id={id}"), now_ms());
        let redirect = location(&status).expect("a redirect back to the client");
        let code = Url::parse(&redirect)
            .expect("the redirect parses")
            .query_pairs()
            .find(|(key, _)| key == "code")
            .map(|(_, value)| value.into_owned())
            .expect("a code");
        // The seam: the code the consent half minted is the one this half
        // takes, bound to the client and the redirect the approval carried.
        let document = auth
            .token_grant(&exchange_body(&code, &client_id, &[]), now_ms())
            .expect("the approved code redeems");
        assert!(!field(&document, "access_token").is_empty(), "{document}");
    }

    #[test]
    fn a_code_redeemed_twice_yields_one_pair_and_then_nothing() {
        let temp = TempPath::new();
        let (auth, agents, _chrome, client_id) = authorized_fixture(temp.path());
        let code = stage_code(&auth, &client_id, CHALLENGE_METHOD, REDIRECT);
        let body = exchange_body(&code, &client_id, &[]);
        assert!(auth.token_grant(&body, NOW).is_ok());
        assert_eq!(auth.token_grant(&body, NOW), Err(grant_refused()));
        assert_eq!(
            agents.lock().expect("the store").tokens().len(),
            2,
            "a second redemption issued a second pair"
        );
    }

    #[test]
    fn edge_probe_concurrency_two_exchanges_of_one_code_yield_exactly_one_pair() {
        let temp = TempPath::new();
        let (auth, agents, _chrome, client_id) = authorized_fixture(temp.path());
        let code = stage_code(&auth, &client_id, CHALLENGE_METHOD, REDIRECT);
        let body = exchange_body(&code, &client_id, &[]);
        // Two real threads on one code. Single use has to be a property of the
        // store's remove-and-return, not of which caller happened to be first:
        // a read-then-remove pair would let both of these through.
        let (first, second) = std::thread::scope(|scope| {
            let left = scope.spawn(|| auth.token_grant(&body, NOW).is_ok());
            let right = scope.spawn(|| auth.token_grant(&body, NOW).is_ok());
            (
                left.join().unwrap_or_default(),
                right.join().unwrap_or_default(),
            )
        });
        assert!(first ^ second, "two concurrent exchanges of one code: {first} and {second}");
        assert_eq!(
            agents.lock().expect("the store").tokens().len(),
            2,
            "one code minted more than one pair"
        );
    }

    #[test]
    fn an_expired_code_cannot_be_redeemed() {
        let temp = TempPath::new();
        let (auth, agents, _chrome, client_id) = authorized_fixture(temp.path());
        let code = stage_code(&auth, &client_id, CHALLENGE_METHOD, REDIRECT);
        let body = exchange_body(&code, &client_id, &[]);
        // One millisecond past the lifetime, against the same clock the code
        // was stamped from.
        let expired = NOW + AUTHORIZATION_CODE_LIFETIME_MS + 1;
        assert_eq!(auth.token_grant(&body, expired), Err(grant_refused()));
        assert!(agents.lock().expect("the store").tokens().is_empty());
    }

    #[test]
    fn a_wrong_verifier_is_refused_and_leaves_the_code_spent() {
        let temp = TempPath::new();
        let (auth, agents, _chrome, client_id) = authorized_fixture(temp.path());
        let code = stage_code(&auth, &client_id, CHALLENGE_METHOD, REDIRECT);
        let wrong = exchange_body(&code, &client_id, &[("code_verifier", "not-the-verifier")]);
        assert_eq!(auth.token_grant(&wrong, NOW), Err(grant_refused()));
        // And the *correct* verifier no longer helps: the refused redemption
        // consumed the code, which is what makes an intercepted code useless
        // to the interceptor even on a second attempt.
        let right = exchange_body(&code, &client_id, &[]);
        assert_eq!(auth.token_grant(&right, NOW), Err(grant_refused()));
        assert!(agents.lock().expect("the store").tokens().is_empty());
    }

    #[test]
    fn a_verifier_one_byte_from_correct_is_refused_like_any_other() {
        let temp = TempPath::new();
        let (auth, _agents, _chrome, client_id) = authorized_fixture(temp.path());
        let code = stage_code(&auth, &client_id, CHALLENGE_METHOD, REDIRECT);
        let mut near = VERIFIER.to_owned();
        near.pop();
        near.push('X');
        let body = exchange_body(&code, &client_id, &[("code_verifier", &near)]);
        assert_eq!(auth.token_grant(&body, NOW), Err(grant_refused()));
    }

    #[test]
    fn a_code_presented_by_another_client_is_refused() {
        let temp = TempPath::new();
        let (auth, agents, _chrome, client_id) = authorized_fixture(temp.path());
        let other = agents
            .lock()
            .expect("the store")
            .register("Another Agent".to_owned(), vec![REDIRECT.to_owned()], NOW)
            .expect("a second registration")
            .client_id;
        let code = stage_code(&auth, &client_id, CHALLENGE_METHOD, REDIRECT);
        let body = exchange_body(&code, &other, &[]);
        assert_eq!(auth.token_grant(&body, NOW), Err(grant_refused()));
        assert!(agents.lock().expect("the store").tokens().is_empty());
    }

    #[test]
    fn a_code_presented_with_another_redirect_uri_is_refused() {
        let temp = TempPath::new();
        let (auth, _agents, _chrome, client_id) = authorized_fixture(temp.path());
        let code = stage_code(&auth, &client_id, CHALLENGE_METHOD, REDIRECT);
        // A different port on the same loopback host — the one variance the
        // *registration* comparison allows, and the one redemption does not.
        let body =
            exchange_body(&code, &client_id, &[("redirect_uri", "http://127.0.0.1:8766/callback")]);
        assert_eq!(auth.token_grant(&body, NOW), Err(grant_refused()));
    }

    #[test]
    fn a_code_minted_under_a_method_other_than_s256_cannot_be_redeemed() {
        let temp = TempPath::new();
        let (auth, _agents, _chrome, client_id) = authorized_fixture(temp.path());
        // The authorization endpoint refuses this already — asserted in
        // `an_unsafe_challenge_method_is_refused` — so a code like this cannot
        // exist in a correct build. It is staged here anyway, because the
        // point of the second gate is that it holds if the first is ever
        // edited away. `plain` would make the "verifier" equal the challenge.
        let code = stage_code(&auth, &client_id, "plain", REDIRECT);
        let body = exchange_body(&code, &client_id, &[("code_verifier", CHALLENGE)]);
        assert_eq!(auth.token_grant(&body, NOW), Err(grant_refused()));
    }

    #[test]
    fn a_code_whose_challenge_is_not_a_digest_at_all_is_refused() {
        let temp = TempPath::new();
        let (auth, _agents, _chrome, client_id) = authorized_fixture(temp.path());
        // Forty-three unreserved characters, which the authorization
        // endpoint's well-formedness check admits, and not base64url — so no
        // verifier could ever produce it.
        let odd = "~".repeat(43);
        let code = auth.consent.lock().expect("the consent state").codes.mint(
            &client_id,
            &odd,
            CHALLENGE_METHOD,
            REDIRECT,
            NOW + AUTHORIZATION_CODE_LIFETIME_MS,
        );
        assert_eq!(
            auth.token_grant(&exchange_body(&code, &client_id, &[]), NOW),
            Err(grant_refused())
        );
    }

    #[test]
    fn a_registration_alone_carries_no_code_and_no_token() {
        let temp = TempPath::new();
        let (auth, agents, _chrome, client_id) = authorized_fixture(temp.path());
        // The fixture's client is registered and has never been approved.
        assert!(auth.consent.lock().expect("the consent state").codes.entries.is_empty());
        let invented = token_body(&[
            ("grant_type", AUTHORIZATION_CODE_GRANT),
            ("code", "a-code-nobody-issued"),
            ("client_id", &client_id),
            ("redirect_uri", REDIRECT),
            ("code_verifier", VERIFIER),
        ]);
        assert_eq!(auth.token_grant(&invented, NOW), Err(grant_refused()));
        assert!(agents.lock().expect("the store").tokens().is_empty());
    }

    #[test]
    fn a_refresh_token_rotates_into_a_pair_the_client_can_use() {
        let temp = TempPath::new();
        let (auth, _agents, _chrome, client_id) = authorized_fixture(temp.path());
        let code = stage_code(&auth, &client_id, CHALLENGE_METHOD, REDIRECT);
        let first = auth
            .token_grant(&exchange_body(&code, &client_id, &[]), NOW)
            .expect("a correct exchange");
        let refresh = field(&first, "refresh_token");
        let rotated = auth
            .token_grant(
                &token_body(&[("grant_type", REFRESH_TOKEN_GRANT), ("refresh_token", &refresh)]),
                NOW,
            )
            .expect("the current refresh token rotates");
        let access = field(&rotated, "access_token");
        assert_ne!(access, field(&first, "access_token"), "rotation reissued the same token");
        assert_ne!(field(&rotated, "refresh_token"), refresh, "the refresh token did not rotate");
        assert!(auth.verify(&access, NOW).is_ok(), "the rotated access token does not work");
    }

    #[test]
    fn a_consumed_refresh_token_is_refused_and_takes_its_family_with_it() {
        let temp = TempPath::new();
        let (auth, _agents, _chrome, client_id) = authorized_fixture(temp.path());
        let code = stage_code(&auth, &client_id, CHALLENGE_METHOD, REDIRECT);
        let first = auth
            .token_grant(&exchange_body(&code, &client_id, &[]), NOW)
            .expect("a correct exchange");
        let spent = field(&first, "refresh_token");
        let rotated = auth
            .token_grant(
                &token_body(&[("grant_type", REFRESH_TOKEN_GRANT), ("refresh_token", &spent)]),
                NOW,
            )
            .expect("the first rotation succeeds");
        // The replay. Somebody else has a copy of a token the client already
        // spent, and there is no telling from here which of the two is the
        // client — so the family goes.
        assert_eq!(
            auth.token_grant(
                &token_body(&[("grant_type", REFRESH_TOKEN_GRANT), ("refresh_token", &spent)]),
                NOW
            ),
            Err(grant_refused())
        );
        assert!(
            auth.verify(&field(&rotated, "access_token"), NOW).is_err(),
            "the family's access token survived a detected reuse"
        );
        assert!(
            auth.verify(&field(&first, "access_token"), NOW).is_err(),
            "the family's original access token survived a detected reuse"
        );
    }

    #[test]
    fn a_client_rotating_its_current_token_is_not_treated_as_reuse() {
        let temp = TempPath::new();
        let (auth, _agents, _chrome, client_id) = authorized_fixture(temp.path());
        let code = stage_code(&auth, &client_id, CHALLENGE_METHOD, REDIRECT);
        let mut document = auth
            .token_grant(&exchange_body(&code, &client_id, &[]), NOW)
            .expect("a correct exchange");
        // Three honest rotations in a row, each with the token the previous
        // one returned. This is the false positive that would make reuse
        // detection unusable, and it is the case 04-04 unit-tests in the
        // store — asserted again here through the protocol that calls it.
        for round in 0..3 {
            let refresh = field(&document, "refresh_token");
            document = auth
                .token_grant(
                    &token_body(&[
                        ("grant_type", REFRESH_TOKEN_GRANT),
                        ("refresh_token", &refresh),
                    ]),
                    NOW,
                )
                .unwrap_or_else(|_| panic!("rotation {round} was mistaken for a replay"));
        }
        assert!(auth.verify(&field(&document, "access_token"), NOW).is_ok());
    }

    #[test]
    fn an_unknown_refresh_token_and_a_detected_reuse_are_one_answer() {
        let temp = TempPath::new();
        let (auth, _agents, _chrome, client_id) = authorized_fixture(temp.path());
        let code = stage_code(&auth, &client_id, CHALLENGE_METHOD, REDIRECT);
        let first = auth
            .token_grant(&exchange_body(&code, &client_id, &[]), NOW)
            .expect("a correct exchange");
        let spent = field(&first, "refresh_token");
        auth.token_grant(
            &token_body(&[("grant_type", REFRESH_TOKEN_GRANT), ("refresh_token", &spent)]),
            NOW,
        )
        .expect("the first rotation succeeds");
        let reuse = auth.token_grant(
            &token_body(&[("grant_type", REFRESH_TOKEN_GRANT), ("refresh_token", &spent)]),
            NOW,
        );
        let unknown = auth.token_grant(
            &token_body(&[
                ("grant_type", REFRESH_TOKEN_GRANT),
                ("refresh_token", "a-token-nobody-issued"),
            ]),
            NOW,
        );
        // Compared as values, not by inspection: the response is a pure
        // function of this tuple, so equal tuples are byte-identical bodies.
        // A caller who could tell a revoked family from a token that never
        // existed would have an oracle for both.
        assert_eq!(reuse, unknown);
        assert_eq!(reuse, Err(grant_refused()));
        let (left, right) = (auth.handle_token(&token_body(&[
            ("grant_type", REFRESH_TOKEN_GRANT),
            ("refresh_token", &spent),
        ]), NOW), auth.handle_token(&token_body(&[
            ("grant_type", REFRESH_TOKEN_GRANT),
            ("refresh_token", "a-token-nobody-issued"),
        ]), NOW));
        assert_eq!(left.status(), right.status());
        assert_eq!(left.headers(), right.headers());
    }

    #[test]
    fn an_unsupported_grant_type_is_refused_rather_than_falling_through() {
        let temp = TempPath::new();
        let (auth, agents, _chrome, _client_id) = authorized_fixture(temp.path());
        // The two OAuth 2.1 removed, one that never existed, and none at all.
        for grant in ["password", "implicit", "client_credentials", "urn:ietf:params:oauth:🙂"] {
            let refusal = auth.token_grant(&token_body(&[("grant_type", grant)]), NOW);
            assert_eq!(
                refusal.map_err(|(code, error, _)| (code, error)),
                Err((StatusCode::BAD_REQUEST, "unsupported_grant_type")),
                "{grant}"
            );
        }
        assert!(auth.token_grant("", NOW).is_err(), "an empty body was granted something");
        assert!(agents.lock().expect("the store").tokens().is_empty());
    }

    #[test]
    fn a_malformed_or_oversized_token_request_is_answered_rather_than_panicking() {
        let temp = TempPath::new();
        let (auth, agents, _chrome, client_id) = authorized_fixture(temp.path());
        let code = stage_code(&auth, &client_id, CHALLENGE_METHOD, REDIRECT);
        let bodies = [
            ("empty", String::new()),
            ("not form encoded", "{\"grant_type\":\"authorization_code\"}".to_owned()),
            ("a bare percent", "grant_type=authorization_code&code=%".to_owned()),
            ("no parameters at all", "&&&=&".to_owned()),
            ("a missing verifier", exchange_body(&code, &client_id, &[("code_verifier", "")])),
            ("nul bytes", "grant_type=\u{0}\u{0}\u{0}".to_owned()),
            ("oversized", format!("grant_type=refresh_token&refresh_token={}", "A".repeat(64 * 1024))),
        ];
        for (label, body) in bodies {
            let response = auth.handle_token(&body, NOW);
            assert!(
                response.status().is_client_error(),
                "{label} was answered {}",
                response.status()
            );
        }
        // And nothing was issued along the way.
        assert!(agents.lock().expect("the store").tokens().is_empty());
    }

    #[test]
    fn the_token_response_is_json_and_is_stored_nowhere() {
        let temp = TempPath::new();
        let (auth, _agents, _chrome, client_id) = authorized_fixture(temp.path());
        let code = stage_code(&auth, &client_id, CHALLENGE_METHOD, REDIRECT);
        let response = auth.handle_token(&exchange_body(&code, &client_id, &[]), NOW);
        let header_value = |name: header::HeaderName| {
            response.headers().get(name).and_then(|value| value.to_str().ok()).unwrap_or_default()
        };
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(header_value(header::CONTENT_TYPE), "application/json");
        assert_eq!(header_value(header::CACHE_CONTROL), "no-store");
        assert_eq!(header_value(header::PRAGMA), "no-cache");
    }

    #[test]
    fn a_challenge_is_the_same_digest_the_token_store_writes() {
        // RFC 7636 Appendix B's own pair, so the two spellings of one SHA-256
        // are checked against the specification rather than against this file.
        assert_eq!(challenge_as_digest(CHALLENGE), Some(digest_of(VERIFIER)));
        // And nothing that is not thirty-two base64url bytes is one.
        assert_eq!(challenge_as_digest(""), None);
        assert_eq!(challenge_as_digest(&"~".repeat(43)), None);
        assert_eq!(challenge_as_digest("QQ"), None, "two bytes is not a digest");
    }

    #[test]
    fn every_declared_endpoint_is_one_this_plan_answers_and_carries_a_verb_table() {
        let temp = TempPath::new();
        let (auth, _agents, _chrome, _client_id) = authorized_fixture(temp.path());
        let declared = auth.auth_endpoints().expect("endpoints are declared");
        // Seven paths, six endpoint kinds — the authorization endpoint carries
        // two paths, for the reason `AUTHORIZATION_STATUS_PATH` records.
        assert_eq!(declared.len(), 7, "{:?}", declared.keys().collect::<Vec<_>>());
        for path in [
            OAUTH_PROTECTED_RESOURCE_BASE,
            WELL_KNOWN_OAUTH_AUTHORIZATION_SERVER,
            AUTHORIZATION_PATH,
            AUTHORIZATION_STATUS_PATH,
            TOKEN_PATH,
            REGISTRATION_PATH,
            REVOCATION_PATH,
        ] {
            assert!(declared.contains_key(path), "{path} is not routed");
        }
        // Never declared, and never built at all: the authorization server and
        // the resource server are one process reading one `Vec`, so there is
        // nothing to introspect across.
        assert!(!declared.contains_key("/introspect"));
        // The verb table is checked in every arm because **nothing else checks
        // it** — the SDK ships `validate_allowed_methods` and never calls it,
        // measured in `04-02-SPIKE.md` where `GET /token` returned `200`.
        for (endpoint, refused) in [
            (OauthEndpoint::AuthorizationServerMetadata, Method::POST),
            (OauthEndpoint::AuthorizationEndpoint, Method::DELETE),
            (OauthEndpoint::TokenEndpoint, Method::GET),
            (OauthEndpoint::RegistrationEndpoint, Method::HEAD),
            (OauthEndpoint::RevocationEndpoint, Method::GET),
        ] {
            assert_eq!(
                auth.validate_allowed_methods(&endpoint, &refused)
                    .expect("a verb outside the table is refused")
                    .status(),
                StatusCode::METHOD_NOT_ALLOWED
            );
        }
    }

    // ---- the revocation endpoint (RFC 7009) ----------------------------

    /// A form body for the revocation endpoint.
    fn revocation_body(pairs: &[(&str, &str)]) -> String {
        url::form_urlencoded::Serializer::new(String::new()).extend_pairs(pairs).finish()
    }

    /// A second authorized client on the same store, with its own pair.
    fn second_client(agents: &SharedAgents) -> (String, String, String) {
        let mut store = agents.lock().expect("lock the store");
        let client = store
            .register("Another Agent".to_owned(), vec!["http://127.0.0.1/other".to_owned()], NOW)
            .expect("register a second client");
        let (access, refresh) = store
            .authorize(
                &client.client_id,
                NOW,
                &canonical_resource(BOUND),
                TALARIA_SCOPE,
                60_000,
                600_000,
            )
            .expect("authorize the second client");
        (client.client_id, access, refresh)
    }

    #[test]
    fn revoking_a_live_an_already_revoked_and_a_never_valid_token_are_one_answer() {
        let temp = TempPath::new();
        let (auth, agents, _client_id, token) = fixture(temp.path());
        let live = auth.handle_revocation(&revocation_body(&[("token", &token)]));
        // The same token again: nothing is left to remove.
        let again = auth.handle_revocation(&revocation_body(&[("token", &token)]));
        let never = auth.handle_revocation(&revocation_body(&[
            ("token", "tal_this_value_was_never_issued_by_anyone"),
        ]));
        for (label, response) in
            [("live", &live), ("already revoked", &again), ("never valid", &never)]
        {
            assert_eq!(response.status(), StatusCode::OK, "{label}");
        }
        // Status *and* headers compared as values. The body below them is one
        // expression with no branch above it — `handle_revocation` discards
        // what the store said before it builds a response — so equal status
        // and equal headers is the whole of the answer a caller can read.
        assert_eq!(live.headers(), again.headers());
        assert_eq!(live.headers(), never.headers());
        // And the effect is indistinguishable from outside too: only the first
        // attempt changed anything.
        assert!(!auth.revoke_presented_token(&revocation_body(&[("token", "x")])));
        assert!(agents.lock().expect("the store").tokens().is_empty());
    }

    #[test]
    fn revoking_a_refresh_token_removes_the_access_token_issued_beside_it() {
        let temp = TempPath::new();
        let (auth, agents, _client_id, _token) = fixture(temp.path());
        let (_second_id, access, refresh) = second_client(&agents);
        assert!(auth.verify(&access, NOW).is_ok(), "the pair works before the revoke");
        assert!(auth.revoke_presented_token(&revocation_body(&[("token", &refresh)])));
        // The access token issued alongside it is gone, not merely unrelated.
        assert_eq!(
            message(&auth.verify(&access, NOW).unwrap_err()),
            message(&refused()),
            "the refresh token's sibling survived its family being revoked"
        );
    }

    #[test]
    fn revoking_an_access_token_takes_its_family_with_it() {
        let temp = TempPath::new();
        let (auth, agents, _client_id, _token) = fixture(temp.path());
        let (second_id, access, refresh) = second_client(&agents);
        assert!(auth.revoke_presented_token(&revocation_body(&[("token", &access)])));
        // The decision this module makes and RFC 7009 leaves open: the refresh
        // token goes too, or the credential a client just disowned can be
        // exchanged straight back into a working pair.
        let outcome = agents.lock().expect("the store").rotate_refresh(
            &digest_of(&refresh),
            NOW,
            DEFAULT_ACCESS_TTL_MS,
            DEFAULT_REFRESH_TTL_MS,
        );
        assert_eq!(outcome, RefreshOutcome::Unknown);
        // The registration is untouched — that is the *human's* revoke, not a
        // client disowning its own credential.
        let store = agents.lock().expect("the store");
        assert!(store.clients().iter().any(|client| client.client_id == second_id));
    }

    #[test]
    fn revoking_one_clients_token_leaves_another_clients_alone() {
        let temp = TempPath::new();
        let (auth, agents, _first_id, first_token) = fixture(temp.path());
        let (_second_id, second_token, _refresh) = second_client(&agents);
        assert!(auth.revoke_presented_token(&revocation_body(&[("token", &first_token)])));
        assert!(
            auth.verify(&second_token, NOW).is_ok(),
            "revoking one client's token took another client's with it"
        );
    }

    #[test]
    fn a_revocation_with_nothing_to_act_on_changes_nothing_and_still_answers_success() {
        let temp = TempPath::new();
        let (auth, agents, _client_id, token) = fixture(temp.path());
        let before = agents.lock().expect("the store").tokens().len();
        for body in [
            String::new(),
            revocation_body(&[("token_type_hint", "access_token")]),
            revocation_body(&[("nonsense", "value")]),
            "not form encoded at all".to_owned(),
            // Larger than the endpoint parses, which is a refusal that must
            // still look like every other one.
            revocation_body(&[("token", &"A".repeat(MAX_TOKEN_REQUEST_BYTES + 1))]),
        ] {
            assert!(!auth.revoke_presented_token(&body), "{body:.40}");
            assert_eq!(auth.handle_revocation(&body).status(), StatusCode::OK);
        }
        assert_eq!(agents.lock().expect("the store").tokens().len(), before);
        // And the credential that was there all along still works, so none of
        // the above quietly revoked something.
        assert!(auth.verify(&token, NOW).is_ok());
    }

    #[test]
    fn a_wrong_token_type_hint_does_not_stop_a_real_revocation() {
        let temp = TempPath::new();
        let (auth, agents, _client_id, _token) = fixture(temp.path());
        let (_second_id, access, _refresh) = second_client(&agents);
        // The hint says refresh; the token is an access token. Ignored, per
        // RFC 7009 §2.1, because a server that trusted a wrong hint would
        // refuse to revoke a real token.
        assert!(auth.revoke_presented_token(&revocation_body(&[
            ("token", &access),
            ("token_type_hint", "refresh_token"),
        ])));
        assert_eq!(message(&auth.verify(&access, NOW).unwrap_err()), message(&refused()));
    }

    #[test]
    fn the_metadata_names_the_revocation_endpoint_it_answers() {
        let temp = TempPath::new();
        let (auth, _agents, _chrome, _client_id) = authorized_fixture(temp.path());
        let document = auth.authorization_server_metadata();
        assert_eq!(
            document["revocation_endpoint"],
            serde_json::json!(format!("http://{BOUND}{REVOCATION_PATH}"))
        );
        assert_eq!(
            document["revocation_endpoint_auth_methods_supported"],
            serde_json::json!(["none"])
        );
        // The advertised path is one the provider routes, which is what stops
        // the document pointing a conformant client at a 404.
        assert!(auth
            .auth_endpoints()
            .expect("endpoints are declared")
            .contains_key(REVOCATION_PATH));
    }
}

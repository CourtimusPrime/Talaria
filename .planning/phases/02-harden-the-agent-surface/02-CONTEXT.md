# Phase 2: Harden the Agent Surface - Context

**Gathered:** 2026-08-15
**Status:** Ready for planning

<domain>
## Phase Boundary

Close every gap the 2026-08-15 codebase audit surfaced on the agent-facing surface:

- An agent cannot reach the local filesystem through `navigate`/`evaluate`
- One wedged tab cannot stall the rest of the session
- The control socket cannot be driven by another local user
- `download` is bounded and cannot clobber user files
- Tab lifecycle events reach MCP clients as notifications instead of being discarded
- The credential vault is populatable and usable from inside the browser
- CI exists and enforces the above

**Not in this phase:** history, bookmarks, downloads UI, configurable search engine (Phase 3);
OAuth and HTTP transport (Phase 4); anything distributed (Phase 5).

</domain>

<decisions>
## Implementation Decisions

All decisions below were auto-selected (recommended option) — the user declined the interactive
gate. Each carries its rationale so any of them can be reversed cheaply during planning.

### Scheme allowlist (MCP-09)

- **D-01:** Agent-supplied URLs are allowlisted to `http`, `https`, `about:blank`, and `data:`.
  Every other scheme — `file:`, `blob:`, `javascript:`, custom schemes — is refused by
  `parse_agent_url` with a clear, distinct tool error naming the rejected scheme.
- **D-02:** The human URL bar is untouched. `resolve_location`
  (`crates/talaria-shell/src/app.rs:853`) is a separate function from `parse_agent_url`
  (`:841`), so restricting agents costs the user nothing.
- **D-03:** **No opt-in "let agents read local files" setting in this phase.** That is a new
  capability plus a new settings surface — deferred. Refusing by default is the reversible
  direction; an escape hatch can be added later without rework.
- **Rationale:** This is the one place where the project's "no gatekeeping" philosophy collides
  with local-filesystem read. The philosophy is about not policing an *authorized* agent's
  browsing, not about handing it the user's disk. `about:blank` is required for popup adoption;
  `data:` grants nothing an agent can't already do through `evaluate`.

### Wedged-tab semantics (MCP-10)

- **D-04:** Add per-tab in-flight tracking. At most one script-evaluating command per tab.
- **D-05:** A second script command against a tab that already has one in flight fails
  **immediately** with a distinct error (shape: `tab {id} busy — a previous evaluate is still
  running`), rather than queueing or burning the timeout.
- **D-06:** `TALARIA_COMMAND_TIMEOUT_SECS` (default 30s, `crates/talaria-shell/src/control.rs:185`)
  stays as the outer bound — fail-fast is added in front of it, not instead of it.
- **D-07:** Non-script commands against a wedged tab **must keep working** — `screenshot`,
  `tabs_close`, `tabs_list`, and `tabs_focus` are the human's route back in.
- **Rationale:** The real fix is upstream (a SpiderMonkey slow-script interrupt exposed through
  libservo) and is out of scope. Fail-fast converts a 30-second stall into an instant actionable
  error. Keeping `screenshot`/`tabs_close` alive is what preserves takeover — the product's
  entire reactive backstop — on a page that has wedged itself.

### Pipelining and retry correctness (MCP-11)

- **D-08:** Both sides pipeline. `ShellConnection`'s reader
  (`crates/talaria-mcp/src/socket.rs:27-46`) splits into a background task with an
  id → oneshot map, replacing the single `Mutex<Option<Wire>>` round-trip. Shell-side,
  `handle_connection` (`crates/talaria-shell/src/control.rs:166-215`) stops awaiting each
  request's outcome before reading the next line.
- **D-09:** **No wire-format change.** The protocol already IDs every request and documents
  out-of-order replies (`crates/talaria-protocol/src/lib.rs:5-8`) — only the socket layer changes
  shape.
- **D-10:** Fix the stale-connection retry bug in the same change: retry only when the failure
  happened before any byte was written, or when the connection was never established this call
  (`crates/talaria-mcp/src/socket.rs:29-45`).
- **Rationale:** D-10 rides along because pipelining makes the existing bug *worse* — today a
  retried `tabs_open` or `download` can execute twice, and concurrency raises the odds. Same
  file, same refactor, so splitting them across plans would mean touching it twice.

### MCP tab notifications (AGENT-04)

- **D-11:** Emit `TabCrashed` and `TabClosed` to MCP clients as notifications instead of
  discarding them at `crates/talaria-mcp/src/socket.rs:75-77`.
- **D-12:** Notifications are **owner-filtered** — a session sees events only for its own tabs.
  Today `broadcast_event` (`crates/talaria-shell/src/app.rs:368-376`) sends every session every
  event, leaking other agents' tab IDs.
- **D-13:** Replace the silent `try_borrow` drops on the event paths (`:369`, `:1290`, `:1328`,
  `:1339`, `:1364`, `:1385`, `:1398`) with the existing deferred-work queue pattern
  (`pending_captures` / `pending_loads` / `pending_evals`). A notification path that silently
  drops under reentrancy is not a notification path.
- **D-14:** **Sequencing constraint** — D-11 depends on D-08. The current request/reply mutex
  design structurally cannot raise an unsolicited notification; pipelining must land first.
- **Rationale:** Owner filtering is correct addressing, not the per-agent policy layer that
  PROJECT.md rules out of scope. It is free at the moment the notification path is built and
  expensive to retrofit.

### Control socket authentication (SEC-01)

- **D-15:** `SO_PEERCRED` check on accept — reject any peer whose UID differs from the shell's.
- **D-16:** `set_permissions(0o600)` on the socket after bind, and create the `/tmp` fallback
  inside a `0700` per-UID directory rather than as a bare `/tmp/talaria-$UID.sock`
  (`crates/talaria-protocol/src/lib.rs:15-21`, `crates/talaria-shell/src/control.rs:93`).
- **Rationale:** Scoped explicitly as local IPC hygiene — stopping *another OS user* from driving
  the browser — and deliberately distinct from the per-agent permission scoping that PROJECT.md
  keeps out of scope. Today, on the `/tmp` fallback path, any local user can connect, send
  `Hello`, and get the user's cookies, sessions, and vault contents.

### Download bounds (MCP-12)

- **D-17:** Size cap, default 2 GB, overridable via `TALARIA_MAX_DOWNLOAD_BYTES`. On exceed:
  abort and delete the partial file.
- **D-18:** Name collision uniquifies — `file (1).pdf` — never clobbers, never prompts. An
  agent-initiated download must not block on a UI the agent cannot see.
- **D-19:** Give the request a timeout and a cancellation handle so the detached thread cannot
  outlive the command timeout and keep writing after the agent already got an error
  (`crates/talaria-shell/src/app.rs:1250-1269`).
- **D-20:** **Routing through Servo's network stack is deferred**, not done here. It is the right
  fix for downloads behind a login silently fetching the login page instead, but it is a real
  re-architecture (`ureq` → Servo fetch) and Phase 3 builds the storage/progress model anyway.
  Record it as a known limitation in the tool description so agents aren't surprised.
- **Rationale:** Bounding is cheap and closes the disk-fill and silent-clobber holes now.
  Re-routing is separable work with a natural home in Phase 3.

### Credential vault (CRED-02, CRED-03, SEC-02)

- **D-21:** Add `Vault::upsert(entry)` plus an explicit **credentials panel** in the egui chrome
  where the user can add, edit, and delete an entry per domain. This is the phase's capture path.
- **D-22:** A browser-style "save password?" prompt on form submit is **deferred** — it requires
  intercepting form submission through Servo's delegate, which is a materially bigger surface
  than the rest of this phase.
- **D-23:** Autofill surfaces as a **toolbar suggestion** in the existing egui chrome, not as
  inline page-DOM injection. Injection would collide with the agent's own `evaluate` and needs
  form-field detection.
- **D-24:** After a successful plaintext import, delete the source `~/.config/talaria/vault.json`
  (or rename to `vault.json.imported` at `0600`) and surface the import once in the UI, not only
  in the log (`crates/talaria-shell/src/vault.rs:115-131`).
- **D-25:** Check the `set_permissions` result on the key-file fallback instead of discarding it
  with `let _ =` (`crates/talaria-shell/src/vault.rs:60-85`), and surface the keychain-unavailable
  downgrade in the UI once.
- **Rationale:** Manual entry first is what gets `cookies_read` off `{"entries": []}` this phase
  with no new Servo delegate surface. Encryption at rest already works, so all of this is
  additive.

### CI (TEST-03)

- **D-26:** GitHub Actions on push and PR, `ubuntu-latest`: `cargo build --release` →
  `cargo clippy -- -D warnings` → `cargo test` → Xvfb e2e (`python3 tests/e2e/run_all.py`).
- **D-27:** A separate scheduled job regenerates `Cargo.lock` from scratch so the hand-pinned
  `primeorder 0.14.0-rc.14` drift is caught deliberately instead of ambushing a fresh checkout.
- **Rationale:** Xvfb runs fine in CI, and the Python e2e suite carries essentially all real
  coverage — only three `#[test]` functions exist in the workspace. A build-only CI would be
  theatre.

### Overnight lock liveness (TEST-02 follow-up)

- **D-28:** Add a PID liveness check (`kill -0`) to `overnight_lock.py acquire` so stale locks
  self-clear, and clear the currently-held dead lock (pid 2796577, owner `c85a1ef3`).
- **D-29:** Call `acquire` from the loop's own preflight (or a `pre-commit` hook) so it stops
  being optional. Nothing invokes it today.
- **Rationale:** Not a new requirement — this closes the audit's "refused by a corpse" finding
  under TEST-02's existing scope. The failure it was written to prevent is still possible.

### Claude's Discretion

- Exact error message wording and error-code taxonomy for D-01 and D-05
- Whether per-tab in-flight tracking lives on `TabManager` or on `Shared`'s pending queues
- Internal structure of the `ShellConnection` background reader (channel type, map type)
- Credentials-panel layout within the existing egui chrome
- CI job matrix shape and caching strategy

</decisions>

<canonical_refs>
## Canonical References

**Downstream agents MUST read these before planning or implementing.**

### Audit evidence (read first — this phase exists because of it)
- `.planning/codebase/CONCERNS.md` — every item in this phase traces to a numbered finding here, with file:line evidence and a per-claim verdict against the code
- `.planning/codebase/ARCHITECTURE.md` — the shell/control/MCP layering, the `Rc<Shared>` + `RefCell` reentrancy model, and the deferred-work queue pattern D-13 must follow

### Project decisions that constrain this phase
- `.planning/PROJECT.md` — "browser is infrastructure, not a policy layer" (Out of Scope), and the note distinguishing SEC-01 from it
- `.planning/REQUIREMENTS.md` — MCP-09/10/11/12, SEC-01/02, AGENT-04, CRED-02/03, TEST-03 with verified current status
- `.planning/ROADMAP.md` §Phase 2 — goal, success criteria, and the 8 plan slots

### Historical record
- `OVERNIGHT_LOG.md` — lines 13–14 are the two items ("needs your call") that became MCP-09 and MCP-10; also the record of the two-session collision behind D-28/D-29
- `SPEC.md` — the original design intent. **Treat as aspirational, not as status**: the audit found several SPEC items marked "resolved" that describe design, not shipped behavior.
- `.planning/BRIEF.md` — the originating brief, preserved verbatim. **Known stale** on six items; superseded by REQUIREMENTS.md.

### Conventions
- `.planning/codebase/CONVENTIONS.md` — error handling and naming patterns to match
- `.planning/codebase/TESTING.md` — the Xvfb harness contract that D-26's CI job must satisfy

</canonical_refs>

<code_context>
## Existing Code Insights

### Reusable Assets
- **Deferred-work queue pattern** (`pending_captures` / `pending_loads` / `pending_evals` on `Shared`, `crates/talaria-shell/src/app.rs`): the established way to do work that can't run inside a Servo delegate callback. D-13 must use it rather than inventing a second mechanism.
- **Request ID correlation** (`crates/talaria-protocol/src/lib.rs:5-8`): already present and already documents out-of-order replies — D-08 needs no wire change.
- **Vault encryption** (`crates/talaria-shell/src/vault.rs`): ChaCha20-Poly1305 + keychain works today. D-21's write path is purely additive.
- **`tests/e2e/harness.py`**: shared Xvfb + socket harness that new e2e tests (scheme refusal, wedge fail-fast, peer-UID rejection) should extend rather than reimplement.
- **`tests/e2e/overnight_lock.py`**: complete acquire/status/release/`--force` implementation; D-28 adds a liveness check to existing code.

### Established Patterns
- **Single Servo thread**: all engine work is on the winit main thread; everything else reaches it via `EventLoopProxy::send_event(AppEvent)`. Nothing in this phase may block that thread.
- **Never hold a `borrow_mut()` across a call into Servo** — this is exactly what produced the silent-drop bugs D-13 fixes.
- **Agent path and human path are separate functions** — `parse_agent_url` vs `resolve_location`. D-01 depends on this separation holding.

### Integration Points
- `crates/talaria-shell/src/app.rs:841-849` — `parse_agent_url`, the single chokepoint for D-01
- `crates/talaria-mcp/src/socket.rs:27-46` — `ShellConnection`, the chokepoint for D-08/D-10/D-11
- `crates/talaria-shell/src/control.rs:90-156` — accept/handshake path, the chokepoint for D-15/D-16
- `crates/talaria-shell/src/gui.rs` — the only egui surface; D-21's panel and D-23's suggestion both land here
- `crates/talaria-shell/src/app.rs:1250-1269` — the detached download thread, chokepoint for D-17/D-18/D-19

### Fragility Warnings
- `crates/talaria-shell/src/app.rs` is 1425 lines and holds the `ApplicationHandler`, the `WebViewDelegate`, command dispatch, JS wrapping, PNG encode, downloads, and URL parsing. Most of this phase touches it. Splitting it is **out of scope** — but expect merge friction between parallel plans in this file and sequence accordingly.
- `wrap_script` (`:1088-1115`) depends on SpiderMonkey's exact error taxonomy and error-message wording. Do not touch it in this phase; MCP-10 is solved by in-flight tracking outside it, not by changing the trampoline.

</code_context>

<specifics>
## Specific Ideas

**Pre-flight blocker — must be the first plan's first action.** `HEAD` (`5b1f4db`, the
`window.open`/`target=_blank` popup work) has **no full e2e pass on record**. The closing
regression run of the 2026-08-16 loop was deliberately skipped because the suite would have
killed a parallel session's processes. Run `python3 tests/e2e/run_all.py` against a fresh release
build and establish a green baseline **before** changing any code, or every subsequent failure in
this phase is ambiguous.

**Suggested plan sequencing** (dependency-driven, not arbitrary):
1. Green baseline (above) + D-28/D-29 lock liveness — cheap, unblocks safe iteration
2. D-01/D-02/D-03 scheme allowlist — self-contained, one function
3. D-15/D-16 socket peer auth — self-contained, different file
4. D-17/D-18/D-19 download bounds — self-contained
5. D-08/D-09/D-10 pipelining + retry fix — **must precede** notifications
6. D-04..D-07 per-tab in-flight tracking — benefits from pipelining being in place
7. D-11/D-12/D-13 MCP notifications + deferred-work fix for the drop paths
8. D-21..D-25 vault write path, panel, autofill, import cleanup
9. D-26/D-27 CI — last, so it locks in a state that already passes

Items 2, 3, 4 are mutually independent and can run in parallel. Items 5→6→7 are a chain.

</specifics>

<deferred>
## Deferred Ideas

- **Capture-on-form-submit credential prompt** — needs Servo form-submission interception; natural follow-on to D-21's manual path
- **Route `download` through Servo's network stack** — fixes downloads behind a login; pairs naturally with Phase 3's downloads UI
- **Opt-in "agents may read local files" setting** — the escape hatch D-03 declines to build now
- **Lazy framebuffer allocation** (~12MB/tab allocated eagerly at open, `crates/talaria-shell/src/tabs.rs`) — cost not leak; soak RSS was flat
- **Gate `__talaria_sim_crash__` behind a `test-hooks` cargo feature** instead of the runtime `TALARIA_TEST_HOOKS=1` env check — harmless today, bad precedent
- **Bounded event channel per connection** (`crates/talaria-shell/src/control.rs:150`) — unbounded memory driven by a peer that stops reading; low risk with only two event types
- **Split `crates/talaria-shell/src/app.rs`** (1425 lines) — real debt, but a refactor of this size inside a hardening phase would swamp the actual fixes
- **Direct tests for delegate callbacks** (`notify_crashed`, `notify_closed`, `request_create_new`) — the crash path is currently tested via a hook that bypasses real `notify_crashed` entirely
- **Tab-volume caps / popup-loop protection** — SPEC explicitly defers this; a runaway agent can still open tabs until memory runs out

</deferred>

---

*Phase: 2-Harden the Agent Surface*
*Context gathered: 2026-08-15*

---
quick_id: 260817-jec
slug: unblock-ci-and-close-phase-2-split-ci-in
date: 2026-08-17
status: in-progress
---

# Quick task: unblock CI and close Phase 2

Phase 2 finished all eleven plans on 2026-08-16 but could not close, because
TEST-03 required a green CI run and the repository had no git remote. The remote
now exists (`git@github.com:CourtimusPrime/Talaria.git`, public), `main` is
pushed, and a self-hosted runner named `thinkpad` is registered and online.

That changes what the CI gate should look like. The original `ci.yml` was written
for `ubuntu-latest` with a 180-minute budget and an `actions/cache` entry
covering `target/`. Two measurements make that shape unworkable as a per-push
gate:

- the local `target/` directory is **42 GB**, and an `actions/cache` entry is
  capped at **10 GB per repository** — the cache can never be written, so every
  hosted run is a cold Servo build against a runner with roughly 25–30 GB free
- a warm incremental rebuild of only the two workspace leaf crates takes
  **2m47s** on the ThinkPad (thin LTO, `opt-level = 2` on all 934 dependencies)

So the same verification costs about three hours cold on a hosted runner and
about three minutes warm on the machine that already holds the build directory.

## Tasks

### Task 1 — Split the CI gate across three workflows

`ci.yml` becomes the fast per-push gate on the self-hosted runner: build, clippy,
unit tests. The fourteen Xvfb e2e suites move to `e2e.yml`, which runs on pushes
to `main`, nightly, and on demand. `cold-build.yml` keeps `ubuntu-latest`, but as
a weekly from-scratch canary rather than a per-push gate.

The original file's central guarantee must survive the split: no `paths:` filter
anywhere, because GitHub reports "no runs required" as a green tick, and a commit
that verified nothing would look verified.

Public-repo safety. A self-hosted runner executing a fork's pull request runs a
stranger's code on the ThinkPad. Repository settings already require approval
from all external contributors, but that is one control, and it is administered
outside this repository. Add a second one inside it: every self-hosted job
carries an `if:` gate admitting only pushes and same-repository pull requests.
Defence in depth, because the setting can be changed by anyone with admin rights
and the workflow condition cannot be changed without a reviewable commit.

**Acceptance:** three workflow files exist; no path filters; every
`runs-on: [self-hosted, thinkpad]` job carries the fork gate; `actionlint` is
clean.

### Task 2 — SECURITY.md with a known-limitations section

Two Phase 2 requirements are half-closed for reasons that are not effort, and
carrying them as open blockers misrepresents the phase as unfinished when the
tractable work is done.

- **MCP-09** — `parse_agent_url` refuses `file:`, `javascript:` and `blob:`, but
  an agent holding an `evaluate` handle still reaches the filesystem through
  `location.href = 'file://…'`. Closing it needs a
  `WebViewDelegate::request_navigation` policy on agent-owned tabs, which
  conflicts with D-02, the decision that the human is the trust root during
  takeover.
- **MCP-10** — a second `evaluate` against a wedged tab is now refused
  instantly, and the other tools keep working, but the first call still runs to
  the command timeout. Interrupting it needs a SpiderMonkey slow-script
  interrupt exposed through libservo. Upstream.

Both belong in a published, honest limitations section rather than in a
tracker's open column.

**Acceptance:** `SECURITY.md` exists with a reporting section and a
known-limitations section naming MCP-09 and MCP-10, each with the mechanism, the
blocking reason, and what closing it would take.

### Task 3 — Close Phase 2

Move MCP-09 and MCP-10 from In Progress to a documented-limitation status in
`.planning/REQUIREMENTS.md`, and mark TEST-03 complete **only after** a green run
on the new gate — not before. Then close Phase 2 in `.planning/STATE.md` and
`.planning/ROADMAP.md`.

**Acceptance:** no requirement is silently marked complete; TEST-03's completion
cites a real run id; ROADMAP shows Phase 2 complete and Phase 3 as current.

### Task 4 — Declare v1 as Phases 1–3, defer 4–7 to v2

Phases 4–7 are each their own product: an OAuth 2.1 authorization server,
distributed mode over Tailscale at a 30–60ms takeover budget, a Windows port of a
Servo application, and an updater whose mechanism is still an open decision
(REL-02, open since project init). Phases 1–3 are a browser a person can actually
use every day, which is a shippable thing.

**Acceptance:** ROADMAP states the v1 boundary explicitly; phases 4–7 are labelled
v2 without being deleted or renumbered.

### Task 5 — Reduce planning artifact overhead

Phase 2 produced roughly 10,000 lines of PLAN and SUMMARY markdown against a
4,734-line Rust codebase. The artifacts are good; the volume is not proportional
for a solo project.

New rule: a PLAN.md is required for any plan touching the security or protocol
surface (`crates/talaria-protocol/`, `vault.rs`, `control.rs`, the MCP tool
surface, anything with a threat register). UI and mechanical work gets a SUMMARY
after the fact and no plan up front.

**Acceptance:** the rule is recorded where a future planning session will read
it, not only in this file.
</content>

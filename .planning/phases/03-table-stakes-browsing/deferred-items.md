# Deferred items — Phase 03

Out-of-scope discoveries logged during execution. Not fixed here.

## ~~`vault_ui_test.py`'s toolbar coordinate moves on every Phase 3 plan~~ — CLOSED

**Resolution: the second candidate fix, built.** The shell answers a
`chrome_rects` control-socket command, gated on `TALARIA_TEST_HOOKS=1`
exactly as the `evaluate` crash hook is, that reports the logical rect of
each named chrome control — the toolbar's buttons and the rows of whichever
panel is open — collected from the `Response`s `Gui` already computes rather
than by recomputing layout. `CREDENTIALS_BUTTON` is now the element name
`toolbar.credentials`, and the comment recording the ~29-points-per-icon-button
rate is gone with the constant it explained.

The first candidate (drive the panel with `Ctrl+K`) was not taken, for the
reason recorded below: it stops pinning the toolbar-button path the suite's
own docstring claims to cover. The suite still clicks the real button.

The measurement recipe below is also obsolete — the shell reports the rect,
so nobody needs to bind a `Response` and `log::warn!` it again. It is left in
place because the Xvfb/`pkill` trap it records is a real one that bit two
plans, and it is not specific to measuring a button.

The new surface is a test-only one and is fenced as such: it is refused as an
unknown command without the hook (asserted by `tests/e2e/panel_click_test.py`),
and no MCP tool exposes it. Chrome geometry is a map of the human's own
controls; an agent that could read it would know where to aim synthetic input
at the credentials button, the bookmark star or a downloads row.

The original report follows.

## `vault_ui_test.py`'s toolbar coordinate moves on every Phase 3 plan

**Found during plan 03-01, Task 3.** `tests/e2e/vault_ui_test.py` opens the
credentials panel by clicking a hardcoded logical point (`CREDENTIALS_BUTTON`).
That suite documents the constant as tracking the toolbar's control order, and
03-01 duly moved it (239 → 283) when it inserted the separator and the History
button ahead of the credentials button. The fix was correct and the suite is
green again.

The point being deferred is that this will recur. `03-UI-SPEC.md` puts the
Bookmark star, Bookmarks, Downloads and Settings buttons in the same new group,
ahead of Credentials, so plans 03-02, 03-03 and 03-04 will each move that
constant again, and each will discover it as a red suite rather than as a known
cost. Two candidate fixes, neither in 03-01's scope:

- Drive the panel open with `Ctrl+K` instead of a click. Coordinate-free and
  immune to toolbar churn, but it stops pinning the toolbar-button path the
  suite's own docstring claims to cover.
- Have the shell expose the button's rect behind `TALARIA_TEST_HOOKS=1` so the
  suite can look it up rather than hardcode it. Keeps the coverage; costs a new
  test-only surface.

Measuring the next value needs no guesswork: temporarily bind the button's
`Response` in `crates/talaria-shell/src/gui.rs` and `log::warn!` its `.rect`.

**Update from 03-03.** It recurred exactly as predicted, and was measured the
same way: `[[359.3 2.0] - [380.3 20.0]]`, so 341 → 370. Three moves now
(239 → 283 → 341 → 370) put the cost of an icon button at ~29 logical points,
which gives 03-04 a number to check its own measurement against. Still
deferred — neither candidate fix got cheaper, and 03-04 is the last plan that
will move it this phase.

## ~~`resolve_location` sends a `data:` URL to the search engine~~ — DECIDED in 03-03

**Resolution (plan 03-03): deliberately not changed.** The asymmetry runs the
right way and is now a tested decision rather than an accident —
`app.rs`'s `a_data_url_is_searched_for_rather_than_opened` pins it, with the
reasoning in the test's own doc comment and in `03-03-SUMMARY.md`.

The short version: "the human typed it, so trust it" is at its weakest exactly
for `data:`, because the paste-this-into-your-address-bar attack makes the
human a courier for someone else's payload rather than the author of it.
`parse_agent_url` admits `data:` because an agent constructs its own data URLs
and SEC-01 hardened that path on its own terms; parity between the two trust
roots was never the goal, and `SECURITY.md` says so. Separately, 03-03's own
must-have required byte-for-byte behavioural identity with the pre-BROWSE-03
address bar, so changing this here was out of bounds regardless.

One residual, accepted rather than fixed: the whole `data:` URL — payload
included — is URL-encoded into the search query, so a pasted data URL's
contents reach the search engine. That is the same exposure any non-URL text
typed into an omnibox has, and it is not specific to `data:`. Revisit only if
the address bar ever grows a "this looks like a URL we refuse to open" state,
which would be a new affordance nothing in v1 has.

The original report follows.

## `resolve_location` sends a `data:` URL to the search engine

**Found during plan 03-01, Task 3**, while choosing pages for the e2e suite.
`resolve_location` (`crates/talaria-shell/src/app.rs`) accepts a parsed URL only
when it has a host, or when its scheme is exactly `about` or `file`. A `data:`
URL has no host and neither of those schemes, so a human pasting one into the
address bar does not open it — it is URL-encoded and handed to DuckDuckGo as a
search query. The agent path (`parse_agent_url`) does admit `data:`, so the two
trust roots disagree in the direction that surprises the human.

Not fixed here: 03-01 touches neither function, and `resolve_location` is
`03-03-PLAN.md`'s business (BROWSE-03 gives it a `&SearchEngine` parameter).
Worth deciding there rather than rediscovering it a third time.

## Same-document (`pushState`) navigation produces no history row

**Found during plan 03-01, Task 3** — this is the measured answer to
`03-RESEARCH.md`'s Open Question 1, not a surprise. Servo 0.4.0 does not re-fire
`LoadStatus::Complete` for an in-page `history.pushState`, so a single-page
app's route changes are not recorded. `tests/e2e/history_test.py` prints the
observation (`PUSHSTATE PROBE: 3 -> 3 rows`) on every run rather than asserting
it, so the day Servo's behaviour changes, the suite says so.

Accepted as the v1 gap `03-RESEARCH.md` Pitfall 4 already named. Closing it
needs a signal libservo does not currently expose to embedders; revisit if
Servo gains one.

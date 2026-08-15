# Phase 2 — Edge Probe Report

**Generated:** 2026-08-15 by the spec-less probe fallback in `/gsd-plan-phase 2 --skip-research`.

This phase has no SPEC.md, so `## Edge Coverage` was absent and the deterministic edge probe
(`gsd-core/bin/lib/edge-probe.cjs`) ran against the 10 phase requirement IDs instead. It surfaced
**20 applicable edges**, all `unresolved` pending the planner’s `--auto` resolution.

Persisted so the 20/20 accounting in the plans is **checkable, not asserted** (plan-checker warning,
2026-08-15). Each row below must appear in some plan’s `must_haves` — as a plain truth string, as a
structured `{statement, verification: backstop}` marker, or as a `<flagged_assumptions>` entry.

| # | Requirement | Category | Probe question | Disposition |
|---|-------------|----------|----------------|-------------|
| 1 | MCP-09 | concurrency | If interrupted or run in parallel, what is guaranteed? | planner-resolved |
| 2 | MCP-10 | unclassified | unclassified — review manually | **flagged assumption** (unresolved — never auto-backstop) |
| 3 | MCP-11 | concurrency | If interrupted or run in parallel, what is guaranteed? | planner-resolved |
| 4 | MCP-12 | boundary | What happens exactly at each min/max/threshold — and one step either side? | planner-resolved |
| 5 | MCP-12 | precision | Where can precision loss, overflow, or rounding/tie-breaking occur — and what is the exact contract (e.g. half-up vs half-to-even, ceil/floor/truncate)? | planner-resolved |
| 6 | MCP-12 | concurrency | If interrupted or run in parallel, what is guaranteed? | planner-resolved |
| 7 | SEC-01 | adjacency | When two things are exactly equal or just touch, do they merge, collide, or separate? | planner-resolved |
| 8 | SEC-01 | empty | What is the result for empty, single-element, or null input? | planner-resolved |
| 9 | SEC-01 | ordering | When elements compare equal, is output order specified and stable? | planner-resolved |
| 10 | SEC-01 | concurrency | If interrupted or run in parallel, what is guaranteed? | planner-resolved |
| 11 | SEC-02 | idempotency | What happens if this runs twice on the same input? | planner-resolved |
| 12 | SEC-02 | concurrency | If interrupted or run in parallel, what is guaranteed? | planner-resolved |
| 13 | AGENT-04 | unclassified | unclassified — review manually | **flagged assumption** (unresolved — never auto-backstop) |
| 14 | CRED-02 | unclassified | unclassified — review manually | **flagged assumption** (unresolved — never auto-backstop) |
| 15 | CRED-03 | idempotency | What happens if this runs twice on the same input? | planner-resolved |
| 16 | CRED-03 | concurrency | If interrupted or run in parallel, what is guaranteed? | planner-resolved |
| 17 | TEST-03 | adjacency | When two things are exactly equal or just touch, do they merge, collide, or separate? | planner-resolved |
| 18 | TEST-03 | empty | What is the result for empty, single-element, or null input? | planner-resolved |
| 19 | TEST-03 | ordering | When elements compare equal, is output order specified and stable? | planner-resolved |
| 20 | TEST-03 | concurrency | If interrupted or run in parallel, what is guaranteed? | planner-resolved |

## Coverage summary

```json
{
  "applicable": 20,
  "resolved": 0,
  "unresolved": 20,
  "byVerification": {
    "explicit": 0,
    "backstop": 0
  }
}
```

## Accounting rule

(# probe-surfaced items) == (# authored into `must_haves`) + (# surfaced as flagged assumptions).
**20 in, 20 must be accounted for.** The three `unclassified` rows (MCP-10, AGENT-04, CRED-02) stay
`unresolved` as flagged assumptions — auto-backstopping or auto-dismissing any of them is a defect.

## Plan mapping

Every plan states its own accounting in a `Probe-edge accounting` section (or, for the three
unclassified rows, in its `<flagged_assumptions>` section). Both forms carry a greppable
`Accounting:` line. This table is the cross-check: each row below appears in exactly one plan.

| Plan | Rows absorbed | Count |
|------|---------------|-------|
| 02-01 | 17, 20 | 2 |
| 02-02 | 1 | 1 |
| 02-03 | 7, 8, 9, 10 | 4 |
| 02-04 | 4, 5, 6 | 3 |
| 02-05 | 3 | 1 |
| 02-06 | 2 (flagged assumption) | 1 |
| 02-07 | — | 0 |
| 02-08 | 13 (flagged assumption) | 1 |
| 02-09 | 11, 12, 15, 16 | 4 |
| 02-10 | 14 (flagged assumption) | 1 |
| 02-11 | 18, 19 | 2 |
| **Total** | **rows 1–20, each once** | **20** |

Two requirements are served by two plans each, so their rows are split rather than duplicated:
TEST-03's four rows go 17 and 20 to plan 02-01 (the branch-lock half) and 18 and 19 to plan 02-11
(the CI half); CRED-03's two rows both go to plan 02-09 (the storage half), leaving plan 02-10 with
only the CRED-02 row. AGENT-04's single row sits with plan 02-08, so plan 02-07 absorbs none.

---
*Probe run: 2026-08-15*

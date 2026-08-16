# Parallel worktree execution

`wt.toml` in this directory configures [worktrunk](https://worktrunk.dev) so that
a Talaria worktree is cheap enough to create per agent. Read the comments there
for why each hook exists; this file covers how to drive it.

## What a worktree gets

| Resource | Value | Why |
|---|---|---|
| Build dir | `~/.cache/talaria-target/<branch>` | Per-branch, persistent, outside the worktree — survives removal, so the next worktree on that branch is warm |
| rustc wrapper | `sccache`, if installed | Lets separate target dirs share compiled artifacts instead of each cold-building Servo |
| X display | `:1xxxx`, hashed from branch | Two concurrent e2e runs would otherwise both grab `:99` |
| `XDG_RUNTIME_DIR` | `/tmp/talaria-wt-<branch>`, mode 0700 | The control socket resolves under here; two shells would otherwise fight over one path |

All four are written to `.wt-env` in the worktree root. The primary checkout has
no generated `.cargo/config.toml` and no `.wt-env`, so it keeps using `./target`
and the default display — single-worktree workflow is unchanged.

## Driving it from a GSD orchestrator

GSD's own parallel dispatch uses Claude Code's `isolation="worktree"` primitive,
which knows nothing about these hooks — it would produce a bare checkout with no
`target/`. So when running plans in parallel here, drive worktrees explicitly and
leave `workflow.use_worktrees` set to `false`:

```bash
wt switch --create gsd/phase-3-plan-04 --yes     # hooks run; .wt-env appears
```

Then dispatch the executor **without** `isolation="worktree"`, telling it in the
prompt to work in that path and source the env first:

```
cd <worktree_path> && . ./.wt-env
```

Everything after that — `cargo build --release`, `python3 tests/e2e/run_all.py` —
picks up the right target dir, display, and socket automatically.

When the plan is done:

```bash
wt merge --yes          # rebases and merges the branch back
wt remove --yes         # branch deleted if merged; target dir is kept
```

## Caveat worth knowing

`wt merge` replaces GSD's `worktree.cleanup-wave` helper, which validates branch
identity, expected base, deletion diffs, and merge result before deleting
anything. Driving worktrees through `wt` means those guards are not running. Check
`git log --stat` on the merge before moving to the next wave, especially if two
plans in the same wave touched the same file.

## First run

Project hooks need approval once:

```bash
wt switch --create <branch>     # prompts to allow the three commands
```

Use `--yes` to skip the prompt in automation. Approvals live in
`~/.config/worktrunk/approvals.toml` and are re-requested if a command changes.

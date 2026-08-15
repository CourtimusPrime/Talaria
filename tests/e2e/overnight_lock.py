#!/usr/bin/env python3
"""Branch ownership lock for the overnight test-and-improve loop.

Two Claude sessions once ran the loop on the same branch and working tree at
the same time (2026-08-16): one froze rebuilds and soaked an old binary while
the other rebuilt the shell eight times and committed on top, so the soak's
results described a binary that no longer existed and one session's log edits
silently no-op'd against the other's. Neither session could tell, because
nothing on disk said "somebody already owns this branch".

This is that missing marker. A loop acquires the lock *before its first
commit* and releases it on a clean exit; a second loop that finds a live lock
refuses to start instead of interleaving.

    python3 tests/e2e/overnight_lock.py acquire <owner-id> [--force]
    python3 tests/e2e/overnight_lock.py status
    python3 tests/e2e/overnight_lock.py release <owner-id>

`acquire` exits 0 when the lock is taken (or already held by the same owner,
so a resumed session is not blocked by itself) and 1 when another owner holds
it. The lock is untracked (see .gitignore): it describes one machine's
working tree, not the branch's content, so it must never travel in a commit.
"""
import json
import os
import subprocess
import sys
import time

REPO = os.path.abspath(os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".."))
LOCK = os.path.join(REPO, ".overnight-lock")


def branch():
    out = subprocess.run(["git", "-C", REPO, "branch", "--show-current"],
                         capture_output=True, text=True)
    return out.stdout.strip()


def read():
    try:
        with open(LOCK) as f:
            return json.load(f)
    except (OSError, ValueError):
        return None


def acquire(owner, force=False):
    held = read()
    if held and held.get("owner") != owner and not force:
        print(f"REFUSED: {held.get('branch')} is owned by session {held.get('owner')} "
              f"since {held.get('started')} (pid {held.get('pid')}).")
        print(f"Take it over only if that session is confirmed stopped: "
              f"{sys.argv[0]} acquire {owner} --force")
        return 1
    if held and held.get("owner") == owner:
        print(f"HELD by this session since {held.get('started')} — continuing.")
        return 0
    with open(LOCK, "w") as f:
        json.dump({
            "owner": owner,
            "branch": branch(),
            "started": time.strftime("%Y-%m-%d %H:%M:%S %z"),
            "pid": os.getpid(),
        }, f, indent=2)
        f.write("\n")
    print(f"ACQUIRED {branch()} for session {owner}"
          + (" (forced over a stale lock)" if force and held else ""))
    return 0


def status():
    held = read()
    if not held:
        print(f"free — no lock at {LOCK}")
        return 0
    print(json.dumps(held, indent=2))
    return 0


def release(owner):
    held = read()
    if held and held.get("owner") != owner:
        print(f"REFUSED: lock belongs to {held.get('owner')}, not {owner}.")
        return 1
    try:
        os.remove(LOCK)
        print("RELEASED")
    except FileNotFoundError:
        print("already released")
    return 0


if __name__ == "__main__":
    action = sys.argv[1] if len(sys.argv) > 1 else "status"
    if action == "status":
        sys.exit(status())
    if len(sys.argv) < 3:
        sys.exit(f"usage: {sys.argv[0]} {action} <owner-id>")
    if action == "acquire":
        sys.exit(acquire(sys.argv[2], force="--force" in sys.argv))
    if action == "release":
        sys.exit(release(sys.argv[2]))
    sys.exit(f"unknown action: {action}")

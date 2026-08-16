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

    python3 tests/e2e/overnight_lock.py acquire <owner-id> [--pid <n>] [--force]
    python3 tests/e2e/overnight_lock.py check
    python3 tests/e2e/overnight_lock.py status
    python3 tests/e2e/overnight_lock.py release <owner-id>

`acquire` exits 0 when the lock is taken (or already held by the same owner,
so a resumed session is not blocked by itself) and 1 when another owner holds
it *and is still alive*. `check` is the read-only form the pre-commit hook
calls: it exits 1 when a live lock is held by someone other than the value of
the TALARIA_OVERNIGHT_OWNER environment variable, and 0 otherwise. An unset
TALARIA_OVERNIGHT_OWNER counts as not-the-owner, so an ordinary interactive
session is refused while a loop genuinely owns the branch — that is the whole
incident this file exists to prevent.

The recorded PID had to change before liveness could mean anything. The first
version wrote `os.getpid()`, the PID of the acquiring python process, which
exits the moment acquire returns — so the recorded PID was dead by design and
a `kill -0` bolted onto it would have cleared every lock, including live ones.
What is recorded now is the PID of something that *outlives* the acquiring
script: `os.getppid()` (the shell or loop driver that invoked it) by default,
or an explicit `--pid` the loop supplies for itself. The `pid_source` key says
which of the two it was, so a reader can tell a deliberate identity from a
fallback. Only then does "is the owner still there?" have a truthful answer.

The lock is untracked (see .gitignore): it describes one machine's working
tree, not the branch's content, so it must never travel in a commit. It is a
cooperative marker, not a security control — anyone who can write the repo
directory can edit or delete it, and could already edit the repo anyway.
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


def _pid_alive(pid):
    """True for a live process; False for none or a zombie (killed but not
    yet reaped by its parent — still listed in /proc)."""
    try:
        with open(f"/proc/{pid}/status") as f:
            for line in f:
                if line.startswith("State:"):
                    return "Z" not in line.split()[1]
    except OSError:
        return False
    return False


def held_is_live(held):
    """True when the lock's recorded owner process is still running. A missing
    or unparseable pid counts as dead: a lock we cannot attribute to a live
    process cannot be allowed to block a new session forever."""
    try:
        pid = int(held.get("pid"))
    except (TypeError, ValueError):
        return False
    return _pid_alive(pid)


def refusal(held, owner):
    print(f"REFUSED: {held.get('branch')} is owned by session {held.get('owner')} "
          f"since {held.get('started')} (pid {held.get('pid')}).")
    print(f"Take it over only if that session is confirmed stopped: "
          f"{sys.argv[0]} acquire {owner} --force")


def acquire(owner, force=False, pid=None):
    held = read()
    stale = held and held.get("owner") != owner and not held_is_live(held)
    if held and held.get("owner") != owner and not force and not stale:
        refusal(held, owner)
        return 1
    if held and held.get("owner") == owner:
        print(f"HELD by this session since {held.get('started')} — continuing.")
        return 0
    if stale:
        print(f"CLEARED stale lock: session {held.get('owner')} (pid {held.get('pid')}) "
              f"is no longer running.")
    with open(LOCK, "w") as f:
        json.dump({
            "owner": owner,
            "branch": branch(),
            "started": time.strftime("%Y-%m-%d %H:%M:%S %z"),
            "pid": os.getppid() if pid is None else pid,
            "pid_source": "ppid" if pid is None else "argv",
        }, f, indent=2)
        f.write("\n")
    print(f"ACQUIRED {branch()} for session {owner}"
          + (" (forced over a stale lock)" if force and held and not stale else ""))
    return 0


def check():
    """Read-only gate for the pre-commit hook: refuse when a live lock belongs
    to somebody else. Never writes the lock file."""
    held = read()
    if not held:
        return 0
    owner = os.environ.get("TALARIA_OVERNIGHT_OWNER", "")
    if held.get("owner") == owner and owner:
        return 0
    if not held_is_live(held):
        return 0
    refusal(held, owner or "<your-session-id>")
    return 1


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


def parse_pid(argv):
    """`--pid <n>`, or None to mean "record my parent"."""
    if "--pid" not in argv:
        return None
    try:
        return int(argv[argv.index("--pid") + 1])
    except (IndexError, ValueError):
        sys.exit("usage: --pid <integer>")


if __name__ == "__main__":
    action = sys.argv[1] if len(sys.argv) > 1 else "status"
    if action == "status":
        sys.exit(status())
    if action == "check":
        sys.exit(check())
    if len(sys.argv) < 3:
        sys.exit(f"usage: {sys.argv[0]} {action} <owner-id>")
    if action == "acquire":
        sys.exit(acquire(sys.argv[2], force="--force" in sys.argv,
                         pid=parse_pid(sys.argv)))
    if action == "release":
        sys.exit(release(sys.argv[2]))
    sys.exit(f"unknown action: {action}")

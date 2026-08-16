#!/usr/bin/env python3
"""E2E: the plaintext credential import and its cleanup (SEC-02).

Before this suite the vault read a hand-written ``vault.json``, encrypted it,
and then left the plaintext file sitting in the config directory with whatever
permissions it had — indefinitely, and with nothing telling the user their
passwords were still readable there.

What this pins down:

- a plaintext credential file present at first run is imported, is readable
  afterwards through ``cookies_read`` (the same path an agent uses), and is
  gone from the config directory;
- the encrypted file that replaced it carries neither password as a cleartext
  byte sequence;
- a restart with the same home directory still returns both entries — the
  second run finds no plaintext file, imports nothing, and disturbs nothing;
- a plaintext file that appears *after* the encrypted vault exists is neither
  imported nor deleted, which is deliberate (see the comment at that check);
- and the guard that makes the removal safe: when the encrypted write cannot
  be verified, the plaintext source survives, is chmodded owner-only, and an
  error naming it reaches the log.

That last case is the one that matters most. Removing the source before the
encrypted copy is proven durable would trade a data-loss bug for a security
fix, so this suite proves the ordering rather than assuming it.

Standalone (starts its own Xvfb + shell): it needs an isolated home, because
the vault, the key file and the engine profile all live under the platform
config directory and the developer's real ones must not be touched.

Passwords are generated per run rather than hardcoded — nothing
credential-shaped is committed, and a random token cannot coincidentally
appear inside the ciphertext the "no cleartext" check scans.
"""
import json
import os
import secrets
import shutil
import socket
import stat
import subprocess
import sys
import tempfile
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import harness

ONE = {"url": "https://vault-one.example/login", "username": "alice",
       "password": secrets.token_hex(16)}
TWO = {"url": "https://vault-two.example/signin", "username": "bob",
       "password": secrets.token_hex(16)}
THREE = {"url": "https://vault-three.example/", "username": "carol",
         "password": secrets.token_hex(16)}


def make_home(prefix):
    """A temp home with an empty talaria config directory inside it."""
    home = tempfile.mkdtemp(prefix=prefix)
    config = os.path.join(home, ".config")
    talaria = os.path.join(config, "talaria")
    os.makedirs(talaria)
    return home, config, talaria


def write_plaintext(talaria, entries):
    path = os.path.join(talaria, "vault.json")
    with open(path, "w") as f:
        json.dump(entries, f)
    return path


def start(home, config, log=subprocess.DEVNULL):
    return harness.start_shell("about:blank", log=log, rust_log="warn",
                               HOME=home, XDG_CONFIG_HOME=config)


def stop_shell(shell):
    """Stop the shell and wait for its socket to go, so the next start_shell
    does not return against the dying one (and does not get its URL forwarded
    to it by the single-instance path)."""
    shell.terminate()
    for _ in range(50):
        if shell.poll() is not None:
            break
        time.sleep(0.1)
    else:
        shell.kill()
    shell.wait()
    for _ in range(50):
        if not os.path.exists(harness.SOCK):
            return
        time.sleep(0.1)
    try:
        os.remove(harness.SOCK)
    except FileNotFoundError:
        pass


def rpc(command, **params):
    """One request on its own connection — the suite restarts the shell
    between phases, so a long-lived wire would not survive."""
    s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    s.connect(harness.SOCK)
    try:
        f = s.makefile("rw")
        f.write(json.dumps({"type": "hello", "client": "vault-e2e"}) + "\n")
        f.flush()
        assert json.loads(f.readline())["type"] == "hello_ack"
        f.write(json.dumps({"type": "request", "id": 1, "command": command,
                            **params}) + "\n")
        f.flush()
        while True:
            m = json.loads(f.readline())
            if m.get("type") == "reply" and m.get("id") == 1:
                return m
    finally:
        s.close()


def read(domain):
    r = rpc("cookies_read", domain=domain)
    assert r["outcome"] == "ok", r
    return r["result"]["entries"]


def host_of(entry):
    return entry["url"].split("/")[2]


home, config, talaria = make_home("talaria-vault-e2e-")
broken_home, broken_config, broken_talaria = make_home("talaria-vault-fail-e2e-")
plain = write_plaintext(talaria, [ONE, TWO])
enc = os.path.join(talaria, "vault.enc")

xvfb = harness.start_xvfb()
tal = None
try:
    # --- first run: import, then cleanup ---------------------------------
    tal = start(home, config)

    entries = read(host_of(ONE))
    assert len(entries) == 1, entries
    assert entries[0]["username"] == ONE["username"], entries
    assert entries[0]["password"] == ONE["password"], "the imported entry did not survive"
    print("IMPORT readable through cookies_read:", host_of(ONE), entries[0]["username"])

    assert not os.path.exists(plain), \
        "the plaintext credential file survived the import"
    assert os.path.exists(enc), "no encrypted vault replaced the plaintext one"
    print("PLAINTEXT removed:", plain)

    with open(enc, "rb") as f:
        ciphertext = f.read()
    assert ciphertext.startswith(b"TALARIA1"), "the encrypted vault has no magic header"
    for entry in (ONE, TWO):
        assert entry["password"].encode() not in ciphertext, \
            "a password is sitting in the encrypted vault as cleartext"
    print("CIPHERTEXT carries neither password in cleartext:", len(ciphertext), "bytes")

    # --- restart: the import is not redone, and nothing is disturbed ------
    stop_shell(tal)
    tal = start(home, config)

    for entry in (ONE, TWO):
        found = read(host_of(entry))
        assert len(found) == 1, (entry["url"], found)
        assert found[0]["password"] == entry["password"], entry["url"]
    print("RESTART both entries still readable")

    # --- a plaintext file appearing after the vault exists ----------------
    # Deliberate: the import arm runs only when the encrypted file is absent,
    # so a vault.json written later is ignored — and, just as deliberately, it
    # is left where it is rather than silently swallowed or silently deleted.
    stop_shell(tal)
    late = write_plaintext(talaria, [THREE])
    tal = start(home, config)

    for entry in (ONE, TWO):
        assert len(read(host_of(entry))) == 1, entry["url"]
    assert read(host_of(THREE)) == [], \
        "a plaintext file written after the vault exists was imported"
    assert os.path.exists(late), \
        "a plaintext file written after the vault exists was deleted"
    print("LATE plaintext neither imported nor deleted:", late)

    # --- a save that cannot land must not cost the user their only copy ---
    # vault.enc is pre-created as a *directory*, so the encrypted write fails
    # and re-reading it fails too. The source must survive that.
    stop_shell(tal)
    tal = None
    broken_plain = write_plaintext(broken_talaria, [ONE])
    os.mkdir(os.path.join(broken_talaria, "vault.enc"))
    log_path = os.path.join(broken_home, "shell.log")
    with open(log_path, "w") as log:
        broken = start(broken_home, broken_config, log=log)
        stop_shell(broken)

    assert os.path.exists(broken_plain), \
        "an unverifiable encrypted write still destroyed the plaintext source"
    mode = stat.S_IMODE(os.stat(broken_plain).st_mode)
    assert mode == 0o600, f"the surviving plaintext file is mode {mode:o}, not 0600"
    with open(log_path) as f:
        log_text = f.read()
    assert broken_plain in log_text, \
        "nothing in the log names the file still holding plaintext credentials"
    print("UNVERIFIED write kept the source, chmod 0600, and said so")

    print("VAULT CHECKS PASSED")
finally:
    if tal is not None:
        stop_shell(tal)
    xvfb.kill()
    xvfb.wait()
    shutil.rmtree(home, ignore_errors=True)
    shutil.rmtree(broken_home, ignore_errors=True)

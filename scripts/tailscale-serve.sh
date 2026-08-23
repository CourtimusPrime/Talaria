#!/usr/bin/env bash
#
# Stand Talaria's remote MCP transport up behind a Tailscale Serve proxy, and
# take it down again.
#
# ## Why this is a script and not a paragraph in a document
#
# The mapping this creates is machine-level state the Tailscale daemon holds.
# Nothing in this repository records it, nothing in a build reproduces it, and
# a `git checkout` neither creates nor removes it. A recipe in a document would
# therefore be a set of instructions nobody can run, nobody can verify, and
# nobody can undo the same way twice. So it is here, executable, with an `up`
# and a `down` and no third mode.
#
# ## What it implements
#
# D-05-03. Talaria keeps binding loopback — `http.rs`'s `BIND_HOST` is a module
# constant and this script does not and cannot change it. Transport security is
# terminated by the Tailscale daemon in front of that loopback listener, which
# is the one proxy target it supports and exactly what the browser already
# binds. The browser issues nothing, loads nothing and renews nothing; that
# lifecycle belongs to the daemon, permanently (threat T-05-16, transferred).
#
# ## What it refuses
#
# **The public-internet variant of this command is never invoked here, and this
# refusal is deliberate rather than an omission.** Serve publishes to your
# tailnet; its sibling publishes to the open internet. The two commands differ
# by a few characters, and the process on the other end of this mapping holds
# an encrypted credential vault and the human's logged-in browsing sessions.
# `05-RESEARCH.md` rejects Funnel outright and asks that the refusal be
# explicit, so that it reads as a named non-option rather than as something
# nobody thought of. Ask this script for it and it says no.
#
# It also refuses to touch a port that already carries a mapping. This node
# runs several unrelated services behind Serve, and clobbering one of them to
# stand up a browser feature is a trade nobody asked for.
#
# ## Usage
#
#   scripts/tailscale-serve.sh up      # create the mapping
#   scripts/tailscale-serve.sh down    # remove only the mapping it manages
#
# Environment:
#   TALARIA_SERVE_PORT   the tailnet HTTPS port to publish on (default 8449)
#   TALARIA_LOCAL_PORT   the loopback port Talaria bound   (default 8779)

set -euo pipefail

# The tailnet HTTPS port this script manages, named once. 8449 specifically:
# this node already serves unrelated services on 443, 8443, 9446 and 9447, and
# 8449 was free when `05-01-SPIKE.md` measured it. Overridable, but the script
# still refuses any port that already carries a mapping it did not create.
SERVE_PORT="${TALARIA_SERVE_PORT:-8449}"

# The loopback port Talaria's listener binds — `settings::DEFAULT_REMOTE_PORT`.
# This is the proxy *target*, and it is loopback because that is the only thing
# this browser ever binds.
LOCAL_PORT="${TALARIA_LOCAL_PORT:-8779}"

LOCAL_HOST="127.0.0.1"
TARGET="http://${LOCAL_HOST}:${LOCAL_PORT}"

say() { printf '%s\n' "$*"; }
fail() { printf 'refused: %s\n' "$*" >&2; exit 1; }

# The full mapping list, as the daemon reports it. Captured before and after
# every change so an operator can see that nothing else moved.
mappings() {
  tailscale serve status 2>/dev/null || true
}

# Whether the daemon already has a handler on SERVE_PORT, whoever created it.
port_is_claimed() {
  tailscale serve status --json 2>/dev/null \
    | python3 -c '
import json, sys
port = sys.argv[1]
try:
    status = json.load(sys.stdin)
except Exception:
    sys.exit(1)
tcp = (status or {}).get("TCP") or {}
sys.exit(0 if port in tcp else 1)
' "$SERVE_PORT"
}

# What that handler proxies to, for the refusal message and for the teardown
# guard. Empty when there is no handler.
claimed_target() {
  tailscale serve status --json 2>/dev/null \
    | python3 -c '
import json, sys
port = sys.argv[1]
try:
    status = json.load(sys.stdin)
except Exception:
    sys.exit(0)
for name, entry in ((status or {}).get("Web") or {}).items():
    if name.endswith(":" + port):
        for handler in (entry.get("Handlers") or {}).values():
            proxy = handler.get("Proxy")
            if proxy:
                print(proxy)
                sys.exit(0)
' "$SERVE_PORT"
}

# The node name a real, daemon-managed identity can be issued for. Also the
# host half of the origin the browser must be configured to advertise.
cert_domain() {
  tailscale status --json 2>/dev/null \
    | python3 -c '
import json, sys
try:
    status = json.load(sys.stdin)
except Exception:
    sys.exit(0)
for name in (status or {}).get("CertDomains") or []:
    print(name)
    break
'
}

backend_state() {
  tailscale status --json 2>/dev/null \
    | python3 -c '
import json, sys
try:
    print((json.load(sys.stdin) or {}).get("BackendState") or "")
except Exception:
    pass
'
}

magicdns_enabled() {
  tailscale status --json 2>/dev/null \
    | python3 -c '
import json, sys
try:
    status = json.load(sys.stdin) or {}
except Exception:
    sys.exit(1)
tailnet = status.get("CurrentTailnet") or {}
sys.exit(0 if tailnet.get("MagicDNSEnabled") else 1)
'
}

# Confirmed, not assumed. Each of these is an operator action in the Tailscale
# admin console, so a missing one is named rather than left to surface as an
# obscure failure three commands later.
check_prerequisites() {
  command -v tailscale >/dev/null 2>&1 \
    || fail "the tailscale CLI is not on PATH"
  command -v python3 >/dev/null 2>&1 \
    || fail "python3 is not on PATH (this script reads the daemon's JSON with it)"

  local state
  state="$(backend_state)"
  [ "$state" = "Running" ] \
    || fail "the Tailscale daemon is not running (state: ${state:-unknown}) — start it with 'tailscale up'"
  say "daemon:    Running"

  magicdns_enabled \
    || fail "MagicDNS is off for this tailnet — enable it in the admin console under DNS"
  say "MagicDNS:  enabled"

  local domain
  domain="$(cert_domain)"
  [ -n "$domain" ] \
    || fail "this node has no HTTPS domain — enable HTTPS Certificates in the admin console under DNS"
  say "node name: $domain"
}

up() {
  say "== prerequisites =="
  check_prerequisites
  local domain
  domain="$(cert_domain)"

  say ""
  say "== mappings before =="
  mappings

  if port_is_claimed; then
    local existing
    existing="$(claimed_target)"
    say ""
    fail "port ${SERVE_PORT} already proxies to ${existing:-something} — this script never takes a port it did not create. Pick another with TALARIA_SERVE_PORT, or remove that mapping deliberately."
  fi

  # A mapping pointing at nothing is worse than no mapping: it answers, and it
  # answers with an error the operator then has to trace back through a proxy.
  if ! (exec 3<>"/dev/tcp/${LOCAL_HOST}/${LOCAL_PORT}") 2>/dev/null; then
    fail "nothing is listening on ${LOCAL_HOST}:${LOCAL_PORT} — turn remote access on in Talaria's Access panel first (or set TALARIA_LOCAL_PORT)"
  fi
  exec 3<&- 2>/dev/null || true
  say ""
  say "listener:  ${TARGET} is up"

  tailscale serve --bg "--https=${SERVE_PORT}" "$TARGET"

  say ""
  say "== mappings after =="
  mappings

  # The one moment both halves of the identity are on screen together. They
  # have to agree byte for byte: the browser publishes what it is configured to
  # advertise, and a client that reached it at a different spelling would be
  # refused by the host allowlist and would reject the metadata document.
  say ""
  say "== the browser's side =="
  say "reachable at:  https://${domain}:${SERVE_PORT}/mcp"
  say ""
  say "Put this in config.json (the 'talaria' directory under your config dir)"
  say "so the advertised identity matches the mapping just created:"
  say ""
  say "  {\"remote_access\": {\"enabled\": true, \"port\": ${LOCAL_PORT},"
  say "                      \"advertised_url\": \"https://${domain}:${SERVE_PORT}\"}}"
  say ""
  say "Exactly that origin: no trailing slash, no path. Talaria refuses a"
  say "second spelling rather than normalising it, because the audience check"
  say "that reads it compares byte for byte."
}

down() {
  command -v tailscale >/dev/null 2>&1 || fail "the tailscale CLI is not on PATH"
  command -v python3 >/dev/null 2>&1 || fail "python3 is not on PATH"

  local before after
  before="$(mappings)"
  say "== mappings before =="
  printf '%s\n' "$before"

  if ! port_is_claimed; then
    say ""
    say "port ${SERVE_PORT} carries no mapping; nothing to remove."
    return 0
  fi

  # Only its own: a mapping on this port proxying somewhere else is somebody
  # else's, and this script does not remove what it did not create.
  local existing
  existing="$(claimed_target)"
  if [ -n "$existing" ] && [ "$existing" != "$TARGET" ]; then
    say ""
    fail "port ${SERVE_PORT} proxies to ${existing}, not ${TARGET} — that mapping is not this script's to remove"
  fi

  tailscale serve --bg "--https=${SERVE_PORT}" off

  after="$(mappings)"
  say ""
  say "== mappings after =="
  printf '%s\n' "$after"

  say ""
  say "== what changed =="
  diff <(printf '%s\n' "$before") <(printf '%s\n' "$after") \
    || true
}

case "${1:-}" in
  up) up ;;
  down) down ;;
  # Named and refused rather than merely absent. See the header: the
  # public-internet variant of Serve would expose a process holding a
  # credential vault and a logged-in browsing session to the open internet,
  # and the two commands differ by a few characters.
  funnel|--funnel|public)
    fail "Funnel is not an option this script offers. It publishes to the public internet, and the process behind this mapping holds an encrypted credential vault and the human's logged-in browsing sessions. Use 'up', which is tailnet-only."
    ;;
  *)
    say "usage: $(basename "$0") up|down"
    say ""
    say "  up    publish ${TARGET} at https://<node>:${SERVE_PORT} on this tailnet"
    say "  down  remove only the mapping on port ${SERVE_PORT}, if this script made it"
    exit 2
    ;;
esac

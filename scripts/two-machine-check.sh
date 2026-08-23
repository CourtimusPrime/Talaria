#!/usr/bin/env bash
#
# The one verification this repository's test harness cannot perform: a real
# remote view and takeover, from a second machine, over the tailnet.
#
# ## Why this exists
#
# `tests/e2e/run_all.py` runs both ends on **one host, under Xvfb, on software
# rendering**. That arrangement proves the protocol, the authorisation, the
# ordering rules, the ladder walk and the degrade-and-report behaviour — and it
# proves nothing whatsoever about the network, because loopback hides
# transmission. Transmission is the only variable Success Criterion 2 is about.
# `tests/e2e/remote_latency_test.py` says so in its own docstring and
# deliberately makes no claim that the target is met.
#
# Three further things are untested by the automated suite and exercised only
# here: TLS terminated by the Tailscale daemon, the tailnet `Host` a
# Serve-proxied request actually carries, and real link behaviour.
#
# ## Why it observes the path type instead of assuming it
#
# Success Criterion 2's ~30–60 ms target is explicitly conditioned on a
# **direct** path. A relayed path (DERP) goes through a relay server and has
# been measured on this tailnet at ~13 Mbit/s and ~40 ms one-way. Both are
# legitimate runs, but they support different claims:
#
#   * a **direct** path can confirm the latency target;
#   * a **relayed** path can only confirm the degrade-and-report behaviour —
#     that the client steps down the ladder, says which rung it is on, says the
#     path is relayed, and still lands a click.
#
# So the path type is read from the daemon's own status and printed
# prominently. Deciding afterwards which claim a run supports, from a number
# nobody wrote down, is how a loopback measurement ends up cited as a network
# one.
#
# ## Why it is a script and not a section in a document
#
# Same reason as `scripts/tailscale-serve.sh`: the global rule is that code
# does not go inline into documentation. A recipe in a document is a set of
# instructions nobody can run and nobody can verify. This one checks what it
# can, prints what it observed, and exits non-zero when a prerequisite is
# missing — so a run that could not have proved anything does not read as a
# pass.
#
# ## What it deliberately does not do
#
# It does not drive the second machine. The questions left after the automated
# suite are "did takeover feel immediate" and "did the click land where I
# aimed", and a script that pretended to automate a human judgement would
# produce a green result for a question only a human can answer. That is worse
# than an honest manual step.
#
# ## Usage
#
#   scripts/two-machine-check.sh [peer-hostname]
#
# Environment:
#   TALARIA_PEER         the client machine on this tailnet (default courts-macbook-air)
#   TALARIA_SERVE_PORT   the tailnet HTTPS port Serve publishes on (default 8449)
#   TALARIA_LOCAL_PORT   the loopback port Talaria bound        (default 8779)

set -euo pipefail

PEER="${1:-${TALARIA_PEER:-courts-macbook-air}}"
SERVE_PORT="${TALARIA_SERVE_PORT:-8449}"
LOCAL_PORT="${TALARIA_LOCAL_PORT:-8779}"

say() { printf '%s\n' "$*"; }
head2() { printf '\n%s\n%s\n' "$*" "$(printf '%*s' "${#1}" '' | tr ' ' '-')"; }
miss() { printf 'missing prerequisite: %s\n' "$*" >&2; MISSING=$((MISSING + 1)); }

MISSING=0

say "Talaria two-machine check"
say "server: this machine   client: ${PEER}"
say "serve port: ${SERVE_PORT}   loopback target: 127.0.0.1:${LOCAL_PORT}"

# ---------------------------------------------------------------------------
# 1. Prerequisites. Checked where checkable, named where not.
# ---------------------------------------------------------------------------

head2 "1. Prerequisites"

if ! command -v tailscale >/dev/null 2>&1; then
  miss "the tailscale CLI is not on PATH — the overlay network is how the client reaches this machine"
else
  say "found: tailscale CLI"
  if ! tailscale status >/dev/null 2>&1; then
    miss "tailscaled is not running or this node is logged out (\`tailscale status\` failed)"
  else
    say "found: tailscaled running"
  fi
fi

if [ ! -x scripts/tailscale-serve.sh ]; then
  miss "scripts/tailscale-serve.sh is not executable — it owns the proxy mapping and this script defers to it"
else
  say "found: scripts/tailscale-serve.sh"
fi

# The proxy mapping. This script does not create it and does not duplicate the
# logic that does: `scripts/tailscale-serve.sh up` is the one place that
# mapping is made, refuses a port carrying somebody else's mapping, and prints
# the `config.json` key it implies.
if command -v tailscale >/dev/null 2>&1 \
  && tailscale serve status 2>/dev/null | grep -q ":${SERVE_PORT}"; then
  say "found: a Serve mapping on port ${SERVE_PORT}"
else
  miss "no Serve mapping on port ${SERVE_PORT} — run: scripts/tailscale-serve.sh up"
fi

# The peer. Reachability is the daemon's business, not ping's: a node can be
# online and still not admitted by the tailnet's access rules.
if command -v tailscale >/dev/null 2>&1; then
  if tailscale status 2>/dev/null | grep -q "[[:space:]]${PEER}[[:space:]]"; then
    say "found: peer ${PEER} in this tailnet"
  else
    miss "peer ${PEER} is not in \`tailscale status\` — name it with TALARIA_PEER or as \$1"
  fi
fi

# Named rather than checked: this script runs on the server and cannot see the
# client machine's filesystem.
say ""
say "not checkable from here, confirm yourself:"
say "  * ${PEER} has a talaria-client binary (cargo build --release -p talaria-client;"
say "    it links no web engine, so it builds in seconds and needs none of the"
say "    engine's native prerequisites)"
say "  * this machine's config.json has remote_access.enabled true, its port set to"
say "    ${LOCAL_PORT}, and advertised_url set to the https:// origin the mapping publishes"

# ---------------------------------------------------------------------------
# 2. The path type. The single most important line this script prints.
# ---------------------------------------------------------------------------

head2 "2. Path type to ${PEER}"

PATH_TYPE="unknown"
if command -v tailscale >/dev/null 2>&1; then
  PEER_LINE="$(tailscale status 2>/dev/null | grep "[[:space:]]${PEER}[[:space:]]" || true)"
  if [ -n "${PEER_LINE}" ]; then
    say "${PEER_LINE}"
    case "${PEER_LINE}" in
      *direct*) PATH_TYPE="direct" ;;
      *relay*)  PATH_TYPE="relayed" ;;
      *)        PATH_TYPE="unknown" ;;
    esac
  fi
fi

say ""
case "${PATH_TYPE}" in
  direct)
    say ">>> PATH: DIRECT"
    say "    This run CAN confirm Success Criterion 2's ~30-60 ms target."
    ;;
  relayed)
    say ">>> PATH: RELAYED (DERP)"
    say "    This run CANNOT confirm the latency target — a relay is in the way."
    say "    It CAN confirm the degrade-and-report behaviour: the client should step"
    say "    down the ladder, name the rung, say the path is relayed, and still land"
    say "    a click. Record it as that, not as a latency result."
    ;;
  *)
    say ">>> PATH: UNKNOWN"
    say "    The daemon did not report direct or relay for this peer. It is often idle"
    say "    until traffic flows: start the client, then re-run this script. Do not"
    say "    record a latency result against an unknown path."
    ;;
esac

if [ "${MISSING}" -gt 0 ]; then
  say ""
  say "${MISSING} prerequisite(s) missing. Nothing below can be proved until they are met."
  exit 1
fi

# ---------------------------------------------------------------------------
# 3. The steps a human performs.
# ---------------------------------------------------------------------------

head2 "3. Steps (perform these yourself)"

SERVE_URL="$(tailscale serve status 2>/dev/null | grep -o "https://[^ ]*:${SERVE_PORT}" | head -1 || true)"
[ -n "${SERVE_URL}" ] || SERVE_URL="https://<this-node>.<tailnet>.ts.net:${SERVE_PORT}"

cat <<STEPS
 1. On THIS machine (the server), start the browser with remote access on:
        cargo run --release -p talaria-shell
    Observe: the Access panel is reachable and the listener is up on
    127.0.0.1:${LOCAL_PORT}. If remote access is off, nothing is listening at all —
    that is the default, not a fault.

 2. Have an agent open at least one tab, so there is something to attach to.
    A viewer can only ever name an agent's tab; your own tabs are not in the
    snapshot and the client cannot ask for one.

 3. Authorise the client. THE FIRST AUTHORISATION HAPPENS IN THIS SERVER
    MACHINE'S OWN CHROME — the consent panel is part of the browser, not a web
    page, so somebody has to be at this keyboard for the first pairing. That is
    a known open constraint, recorded in the phase's deferred register rather
    than worked around. Put the resulting token on ${PEER} in
    TALARIA_CLIENT_TOKEN or its owner-only token file; never on a command line.

 4. On ${PEER}, start the client against the served address:
        talaria-client ${SERVE_URL}
    Observe: it connects over TLS the Tailscale daemon terminated, and lists
    this server's agent tabs. If it reports an untrusted certificate, stop —
    that is the client refusing to downgrade, not a client bug.

 5. Attach to an agent tab (Watch).
    Observe: a keyframe arrives and the page appears. A settled page then sends
    nothing at all; that is correct, not a stall.

 6. Click a link through the client.
    Observe BOTH: (a) the tab navigated, confirmed on this server's screen, and
    (b) the picture in the client updated to the new page. One without the
    other is the interesting failure.

 7. Scroll and type into a field. Observe the aim: the click and the caret land
    where you pointed, not one toolbar-height off.
STEPS

# ---------------------------------------------------------------------------
# 4. What to record. Two different kinds of thing, asked for separately.
# ---------------------------------------------------------------------------

head2 "4. Record these four, in VERIFICATION.md or the phase's summary"

cat <<RECORD
 (a) THE OBSERVED PATH TYPE: ${PATH_TYPE}
     Direct or relayed decides which claim this run supports at all. Copy the
     value above rather than assuming the usual one.

 (b) THE CLIENT'S INPUT-TO-PHOTON ESTIMATE, IN MILLISECONDS.
     The client prints it as "Input to picture: about N ms." (recorded control
     link.measurement; reading.input_to_photon_ms). Write down N.

     >>> THIS NUMBER IS THE EVIDENCE for Success Criterion 2's ~30-60 ms on a
     direct path. It is computed from the frame header's last_delivered_input
     echo, so it needs no clock shared between the two machines and it contains
     the whole chain: transmission, hit test, paint, readback, encode, and the
     client's own presentation. A run that records only a rung and an
     impression has discarded the one figure the claim rests on.

     It counts only inputs that actually reached the page. The server keeps a
     separate ordering mark that advances on inputs it refused -- a click in
     the letterboxed margin, on a crashed tab, or on a tab the viewer never
     attached to -- and this figure is deliberately not computed from that one,
     because those round trips never included a hit test or a repaint and
     would bias the number LOW.

 (c) THE RUNG THE CLIENT REPORTED (reading.rung, and reading.rung_changes if it
     moved). Rung plus denominator is what the human was actually served.

 (d) WHETHER TAKEOVER FELT IMMEDIATE, or felt like operating a remote desktop.

     >>> THIS IS THE PRODUCT CLAIM, AND IT IS NOT THE EVIDENCE. Record it,
     because it is what the feature is for and a number can be met while the
     thing still feels wrong. But do not let it stand in for (b), and do not
     let (b) stand in for it. They answer different questions.
RECORD

say ""
say "Done. Nothing above was proved by this script; it checked what it could,"
say "observed the path type, and told you what to look at."

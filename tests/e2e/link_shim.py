#!/usr/bin/env python3
"""An unprivileged constrained link, for suites that need a slow one.

A TCP forwarder that sits between a client and a server on loopback and makes
the bytes take longer to get there.  Three knobs, and no more:

  * **delay_ms** -- an added one-way delay, applied in both directions.
  * **rate_bps** -- a rate cap in bits per second, applied per direction, by
    holding each chunk until the link would have finished carrying everything
    queued in front of it.
  * **kill()** -- close both halves of every live pair, at once, without
    closing the listener.

The default is **this tailnet's own measured relayed profile**: about 13
Mbit/s and about 40 ms, taken from `05-CONTEXT.md`'s measured facts, where a
MacBook Air tethered through a phone's symmetric NAT fell back to a DERP relay
at that rate while the same machine on the LAN measured 342-447 Mbit/s.  It is
worth attributing rather than leaving as a magic constant: a number that turns
out to be somebody's real measurement of their real network is a different kind
of number from one that was picked to make a test fail.

**Why a Python forwarder and not the kernel.**  `tc netem` and network
namespaces both need root, and the continuous-integration machine here is a
single unprivileged host.  This needs nothing but the standard library, which
is also what the rest of `tests/e2e` is written in.

**The kill knob is not used by the suite that first shipped this**, and it is
here anyway.  Phase 5.1's reconnect and resync work needs exactly it -- a link
that goes away mid-stream, with both ends still running -- and writing it a
second time later is how two versions of one thing come to disagree.

**Its honest limits, stated rather than discovered.**  It shapes a loopback
stream, so it reproduces *bandwidth* and *latency* and nothing else.  There is
no jitter, no packet loss, no reordering, no congestion response, and none of
the path changes a real overlay network performs while you are using it -- a
relay that moves, a direct path that opens mid-session.  It is also a
byte-stream shim rather than a packet shim, so it says nothing about MTU or
about how a real stack would fragment any of this.  **It is a regression
harness for degrade behaviour, not a network simulator**, and a suite that
reads a wall-clock latency off it and calls that a product claim is making the
mistake this file exists to avoid.

Used either as a context manager or by hand::

    with LinkShim(("127.0.0.1", server_port)) as shim:
        client = start_client(f"http://127.0.0.1:{shim.port}")

Run directly, it forwards on a chosen port until interrupted, which is how to
poke at it by hand::

    python3 link_shim.py 41999 127.0.0.1 8779 --delay-ms 40 --rate-bps 13000000
"""
import socket
import sys
import threading
import time

# This tailnet's own measured relayed profile (05-CONTEXT.md, 2026-08-21):
# ~13 Mbit/s and ~40 ms one way, against 342-447 Mbit/s on a direct path.
RELAYED_RATE_BPS = 13_000_000
RELAYED_DELAY_MS = 40

# How much is read from a socket at once. Small enough that the pacing below
# has something to pace -- one 4 MB read would be one 2.5-second stall at the
# relayed rate rather than a stream arriving slowly.
CHUNK = 16384


class _Pipe:
    """One direction of one pair: read here, pace, write there.

    Two threads, deliberately.  A single thread that slept between a read and
    the matching write would cap throughput at one chunk per delay -- which is
    a *bandwidth* limit wearing a latency limit's clothes, and would make every
    measurement taken through it meaningless.  The reader keeps reading and
    stamps each chunk with when it should be delivered; the writer pays the
    delay and the rate."""

    def __init__(self, source, sink, delay_ms, rate_bps):
        self.source = source
        self.sink = sink
        self.delay = delay_ms / 1000.0
        self.rate = rate_bps
        self.queue = []
        self.wake = threading.Condition()
        self.done = False

    def start(self):
        for target in (self._read, self._write):
            threading.Thread(target=target, daemon=True).start()

    def _read(self):
        try:
            while True:
                chunk = self.source.recv(CHUNK)
                if not chunk:
                    break
                with self.wake:
                    # Stamped on arrival: the delay is measured from when the
                    # byte reached the link, not from when the link got round
                    # to it.
                    self.queue.append((time.monotonic() + self.delay, chunk))
                    self.wake.notify()
        except OSError:
            pass
        self.finish()

    def _write(self):
        # When the link next has capacity. A chunk is carried no earlier than
        # this, so a burst is serialised at the cap rather than delivered at
        # once with each piece merely delayed.
        free_at = 0.0
        while True:
            with self.wake:
                while not self.queue and not self.done:
                    self.wake.wait(0.1)
                if not self.queue:
                    break
                deliver_at, chunk = self.queue.pop(0)
            carry = len(chunk) * 8 / self.rate if self.rate else 0.0
            send_at = max(time.monotonic(), deliver_at, free_at)
            pause = send_at - time.monotonic()
            if pause > 0:
                time.sleep(pause)
            free_at = send_at + carry
            try:
                self.sink.sendall(chunk)
            except OSError:
                break
        self.finish()

    def finish(self):
        with self.wake:
            self.done = True
            self.wake.notify_all()
        for end in (self.source, self.sink):
            try:
                end.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass


class LinkShim:
    """A constrained loopback link in front of ``upstream``."""

    def __init__(self, upstream, delay_ms=RELAYED_DELAY_MS,
                 rate_bps=RELAYED_RATE_BPS, host="127.0.0.1", port=0):
        self.upstream = upstream
        self.delay_ms = delay_ms
        self.rate_bps = rate_bps
        self.listener = socket.socket()
        self.listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        self.listener.bind((host, port))
        self.listener.listen(8)
        # Read back rather than assumed, so port 0 is usable and the caller
        # always learns the number actually bound.
        self.host, self.port = self.listener.getsockname()
        self.pairs = []
        self.running = True
        threading.Thread(target=self._accept, daemon=True).start()

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        self.stop()
        return False

    def _accept(self):
        while self.running:
            try:
                near, _ = self.listener.accept()
            except OSError:
                return
            try:
                far = socket.create_connection(self.upstream, timeout=5)
            except OSError:
                near.close()
                continue
            near.settimeout(None)
            far.settimeout(None)
            self.pairs.append((near, far))
            for source, sink in ((near, far), (far, near)):
                _Pipe(source, sink, self.delay_ms, self.rate_bps).start()

    def kill(self):
        """Close both halves of every live pair, leaving the listener up.

        The reconnect case: from either end this is the link going away, not
        the far end going away, which is the distinction a resync has to be
        able to make. Unused by the latency suite; see the module docstring."""
        pairs, self.pairs = self.pairs, []
        for pair in pairs:
            for end in pair:
                try:
                    end.shutdown(socket.SHUT_RDWR)
                except OSError:
                    pass
                try:
                    end.close()
                except OSError:
                    pass
        return len(pairs)

    def stop(self):
        self.running = False
        self.kill()
        try:
            self.listener.close()
        except OSError:
            pass


def _main(argv):
    if len(argv) < 4:
        print(__doc__)
        return 2
    port, host, upstream_port = int(argv[1]), argv[2], int(argv[3])
    delay = RELAYED_DELAY_MS
    rate = RELAYED_RATE_BPS
    for flag, value in zip(argv[4::2], argv[5::2]):
        if flag == "--delay-ms":
            delay = int(value)
        elif flag == "--rate-bps":
            rate = int(value)
    shim = LinkShim((host, upstream_port), delay_ms=delay, rate_bps=rate, port=port)
    print(f"link_shim: 127.0.0.1:{shim.port} -> {host}:{upstream_port} "
          f"at {rate} bit/s with {delay} ms each way")
    try:
        while True:
            time.sleep(1)
    except KeyboardInterrupt:
        shim.stop()
    return 0


if __name__ == "__main__":
    sys.exit(_main(sys.argv))

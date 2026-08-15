"""Shared launcher for the e2e suites that start their own shell.

Isolation knobs (so a suite can run next to a long soak on the default
display without killing it):

- ``TALARIA_E2E_DISPLAY``  X display to create with Xvfb (default ``:99``).
- ``XDG_RUNTIME_DIR``      inherited by the shell — point it at a private
  directory to get a private control socket (``$XDG_RUNTIME_DIR/talaria.sock``).

Only Talaria processes attached to *this* display are killed before launch;
shells on other displays are left alone.
"""
import os
import signal
import subprocess
import time

REPO = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..")
DISPLAY = os.environ.get("TALARIA_E2E_DISPLAY", ":99")
BINARY = os.path.join(REPO, "target", "release", "talaria")
SOCK = os.environ.get("XDG_RUNTIME_DIR", "/tmp") + "/talaria.sock"


def x_env(**extra):
    env = dict(os.environ, DISPLAY=DISPLAY)
    env.update(extra)
    return env


def kill_shells_on_display(display=DISPLAY):
    """SIGKILL talaria processes whose environment carries DISPLAY=display."""
    needle = f"DISPLAY={display}".encode()
    for pid in os.listdir("/proc"):
        if not pid.isdigit():
            continue
        try:
            with open(f"/proc/{pid}/comm", "rb") as f:
                if f.read().strip() != b"talaria":
                    continue
            with open(f"/proc/{pid}/environ", "rb") as f:
                if needle in f.read().split(b"\0"):
                    os.kill(int(pid), signal.SIGKILL)
        except (OSError, ProcessLookupError):
            continue


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


def _clear_stale_x_lock(display):
    """A SIGKILLed Xvfb leaves /tmp/.X<n>-lock behind; a new server refuses
    to start while it exists. Remove it once its owning pid is gone."""
    number = display.lstrip(":").split(".")[0]
    lock = f"/tmp/.X{number}-lock"
    for _ in range(50):
        try:
            with open(lock) as f:
                pid = int(f.read().strip() or 0)
        except FileNotFoundError:
            return
        except ValueError:
            pid = 0
        if pid and _pid_alive(pid):
            time.sleep(0.1)
            continue
        for path in (lock, f"/tmp/.X11-unix/X{number}"):
            try:
                os.remove(path)
            except FileNotFoundError:
                pass
        return


def start_xvfb(display=DISPLAY, settle=2.0):
    subprocess.run(["pkill", "-f", f"[X]vfb {display}( |$)"], check=False)
    kill_shells_on_display(display)
    time.sleep(settle)  # let a previous Xvfb fully release the display
    for attempt in range(5):
        _clear_stale_x_lock(display)
        xvfb = subprocess.Popen(["Xvfb", display, "-screen", "0", "1280x800x24"],
                                stderr=subprocess.DEVNULL)
        for _ in range(20):  # wait until the display actually accepts connections
            if subprocess.run(["xdpyinfo", "-display", display], stdout=subprocess.DEVNULL,
                              stderr=subprocess.DEVNULL).returncode == 0:
                return xvfb
            if xvfb.poll() is not None:
                break  # refused to start (stale lock / race) — retry
            time.sleep(0.5)
        xvfb.kill()
        time.sleep(1.0)
    raise RuntimeError(f"Xvfb {display} never came up")


def start_shell(url, log=subprocess.DEVNULL, rust_log="error", wait=8.0, **env_extra):
    env = x_env(RUST_LOG=rust_log, **env_extra)
    shell = subprocess.Popen([BINARY, url], env=env, stdout=log, stderr=log)
    # Ready when the control socket answers a hello (or after `wait` seconds).
    deadline = time.monotonic() + wait
    while time.monotonic() < deadline:
        if os.path.exists(SOCK):
            break
        time.sleep(0.1)
    time.sleep(max(0.0, deadline - time.monotonic()))  # first paint settle
    return shell


def stop(shell, xvfb):
    shell.terminate()
    time.sleep(1)
    shell.kill()
    shell.wait()
    xvfb.kill()
    xvfb.wait()  # reap, so the X lock's pid isn't left as a zombie

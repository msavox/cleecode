#!/usr/bin/env python3
"""Run A: a film in a pane, in a pty, with the graphics profile switched on.

    python3 scripts/profile_video.py [--shm yes|no] [--seconds 12] [--runs 3]

Not a driver, though it lives among them. The drivers here are checks: they assert, and they
fail. This asserts nothing — it starts a film, waits, and writes down what the counters said,
and reading those numbers is somebody's job afterwards. It is in the tree all the same, because
the pipeline it measures is about to be changed more than once, and a before and an after taken
with two different instruments are not a comparison.

Two things about this harness are not the real world, and both are stated in the report rather
than papered over here. The host is a python process that reads the pty and throws the bytes
away, so it drains as fast as the kernel will hand them over and never decodes a single base64
character — a real terminal both waits and works. And the pty answers no graphics query, so the
protocol is forced with CLEE_GRAPHICS_PROTOCOL=kitty, which is exactly what that hook is for.
"""

import argparse
import os
import select
import shutil
import subprocess
import sys
import tempfile
import time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
sys.path.insert(0, HERE)
from pty_drive import Session  # noqa: E402

# Everything this writes — the throwaway settings, the clip, the profiles — goes in one place
# outside the repository. The root handed to a `Session` is also where the editor looks for its
# configuration, so pointing it at `scripts/` would have a measurement quietly rewriting the
# tree it is measuring.
WORK = os.path.join(tempfile.gettempdir(), "clee-video-profile")


def tool(name):
    """A program this needs, or a clean exit naming the one that is missing.

    Looked up rather than spelled out: a path under `/opt/homebrew` is true on the machine this
    was written on and nowhere else, and a measurement that dies with `No such file` has wasted
    the run it was in the middle of.
    """
    found = shutil.which(name)
    if not found:
        sys.exit(f"{name} is not installed, and this measurement cannot be made without it")
    return found


def clip(work):
    """The film, made once and kept.

    Generated rather than chosen, so that two runs a month apart are two measurements of the
    same thing rather than of two clips. Four by three on purpose: that is the shape that lands
    short of the cell grid — the picture comes back 580x435 for a 580x440 hole — and the cost of
    those few pixels is a large part of what there is to measure.
    """
    path = os.path.join(work, "clip.mp4")
    if not os.path.exists(path):
        subprocess.run([tool("ffmpeg"), "-y", "-loglevel", "error", "-f", "lavfi", "-i",
                        "testsrc2=size=640x480:rate=30", "-t", "20", "-pix_fmt", "yuv420p",
                        path], check=True)
    return path
# Pinned so that two runs are two measurements of the same thing. Wide and tall enough that the
# pane under the editor's chrome is a realistic size for watching something.
COLS, ROWS = 120, 40


def cpu_seconds(pid):
    """Cumulative CPU of a process, as `ps` reports it: [DD-]HH:MM:SS.ss or MM:SS.ss."""
    try:
        out = subprocess.run(["ps", "-o", "time=", "-p", str(pid)],
                             capture_output=True, text=True).stdout.strip()
    except OSError:
        return None
    if not out:
        return None
    parts = out.replace("-", ":").split(":")
    try:
        seconds = 0.0
        for part in parts:
            seconds = seconds * 60 + float(part)
        return seconds
    except ValueError:
        return None


def write_settings(root):
    """A window that is mostly terminal, so the film is a realistic size rather than a stamp.

    The editor's own default gives the pane 35% of the width and the picture that follows is
    300x220 — small enough that every per-pixel stage looks free. The profile is about what a
    film costs, so the pane is made the size somebody watching one would make it.

    `CLEE_PROFILE_SETTINGS` is appended to the file as it stands, for measuring a setting that
    changes what the pipeline is handed rather than what it does with it — `pane_pixel_pct` is
    the one it was added for. Empty unless somebody set it, so a run taken without it is the same
    run it always was: the instrument has to stay the same instrument between two measurements."""
    config = os.path.join(root, ".config", "cleecode")
    os.makedirs(config, exist_ok=True)
    with open(os.path.join(config, "settings.toml"), "w") as handle:
        handle.write(
            "show_sidebar = false\n"
            "show_terminal = true\n"
            "terminal_pct = 80\n"
            "terminal_on_right = false\n"
            "language_server = false\n"
        )
        extra = os.environ.get("CLEE_PROFILE_SETTINGS", "").strip()
        if extra:
            handle.write(extra + "\n")


def focus_terminal(session):
    """Ctrl+P, then the palette entry that moves the keyboard into a shell."""
    session.send("\x10")
    session.wait(lambda s: "matches" in s.text(), 6)
    session.send("focus term")
    session.wait(lambda s: True, 0.5)
    session.send("\r")
    return session.wait(lambda s: "sh-" in s.text() or "$" in s.text(), 8)


def discard(fd, seconds, sample=None):
    """Read the pty and throw it away, waking on the data rather than on a timer.

    The timer is the whole difference between a measurement and a fiction. A pty master hands
    over about a kilobyte per read, so a megabyte-a-frame film is a thousand reads a frame; a
    loop that slept a millisecond whenever it found nothing waiting drained 0.6 MB/s, the pane
    got half a frame a second, and the editor spent its entire life blocked in `write`. Waiting
    on the descriptor instead drains 40 MB/s, which is a film at thirty-seven frames a second
    and an editor doing the work this profile exists to measure."""
    end = time.time() + seconds
    next_sample = time.time()
    while True:
        left = end - time.time()
        if left <= 0:
            return
        ready, _, _ = select.select([fd], [], [], min(left, 0.05))
        if ready:
            try:
                if not os.read(fd, 1 << 20):
                    return
            except BlockingIOError:
                pass
            except OSError:
                return
        if sample and time.time() >= next_sample:
            sample()
            next_sample = time.time() + 1.0


def mpv_pid():
    out = subprocess.run(["pgrep", "-f", "vo=kitty"], capture_output=True, text=True).stdout
    pids = [int(line) for line in out.split()]
    return pids[0] if pids else None


def one_run(binary, profile_path, shm, seconds, sampler=None):
    env = {
        # An empty value is read as "not asked for", which is how the same harness measures the
        # editor with the instrument switched off — the control the report needs to claim that
        # measuring the pipeline did not change it.
        "CLEE_GRAPHICS_PROFILE": profile_path or "",
        # The pty answers no graphics query, so without this the picker falls back to
        # half-blocks and the kitty path — the one the decision is about — never runs.
        "CLEE_GRAPHICS_PROTOCOL": "kitty",
    }
    write_settings(WORK)
    session = Session(binary, WORK, env=env, cols=COLS, rows=ROWS)
    samples = {"clee": [], "mpv": []}
    try:
        # Not "Project Files": the sidebar is off in this layout. A screen with several rows of
        # anything on it is the editor having drawn its first frame.
        ready = lambda s: sum(1 for line in s.lines() if line.strip()) > 3
        if not session.wait(ready, 30):
            return {"error": "the editor never drew its first frame"}
        if not focus_terminal(session):
            return {"error": "the keyboard never reached a shell"}
        # Idle for a moment with everything up: the CPU of a still editor, subtracted later.
        os.set_blocking(session.fd, False)
        idle_before = cpu_seconds(session.pid)
        discard(session.fd, 2.0)
        idle_after = cpu_seconds(session.pid)

        line = (
            f"{tool('mpv')} --vo=kitty --vo-kitty-use-shm={shm} --really-quiet "
            f"--no-audio --untimed=no --keep-open=no {clip(WORK)}\r"
        )
        session.send(line)
        # From here to the end of the session the pty is read and thrown away, and `pyte` never
        # sees another byte. That is not a detail: rendering a megabyte of base64 into a cell
        # grid in pure python takes seconds, the pty fills while it does, and the editor sits
        # blocked in `write` — which showed up in the first runs as fifteen seconds of blocked
        # writing in a nineteen-second session and a film that reached the screen twice a
        # second. The harness was the bottleneck being measured.
        discard(session.fd, 2.0)
        player = mpv_pid()
        # The sampler runs beside the counters rather than instead of them: counters can only
        # report the stages somebody thought to instrument, and the gap between their sum and
        # `getrusage` is exactly the work nobody thought of. A call-stack sampler finds that.
        watcher = None
        if sampler:
            watcher = subprocess.Popen(
                ["/usr/bin/sample", str(session.pid), str(int(seconds)), "-file", sampler],
                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        start = (cpu_seconds(session.pid), cpu_seconds(player) if player else None, time.time())
        discard(session.fd, seconds,
                sample=lambda: (samples["clee"].append(cpu_seconds(session.pid)),
                                samples["mpv"].append(cpu_seconds(player) if player else None)))
        end = (cpu_seconds(session.pid), cpu_seconds(player) if player else None, time.time())
        if watcher:
            watcher.wait(timeout=30)
        # `q` quits mpv; then the editor is asked to quit the ordinary way, because the report is
        # written on the way out of `main` and a killed editor writes nothing.
        session.send("q")
        discard(session.fd, 1.5)
        # Ctrl+Tab first. A focused pane keeps every chord the editor has not claimed, and
        # Ctrl+Q is one of the ones it leaves alone — sent into the pane it reaches the shell as
        # XON and the editor never hears it. Ctrl+Tab is the menu's own "Focus editor".
        session.send("\x1b[9;5u")
        discard(session.fd, 0.7)
        session.send("\x11")
        status = None
        deadline = time.time() + 20
        while time.time() < deadline:
            discard(session.fd, 0.05)
            try:
                pid, raw = os.waitpid(session.pid, os.WNOHANG)
            except OSError:
                break
            if pid:
                status = raw
                break
    finally:
        try:
            os.close(session.fd)
        except OSError:
            pass
        try:
            os.kill(session.pid, 9)
        except OSError:
            pass
    window = end[2] - start[2]
    return {
        "status": Session.describe_status(status),
        "window_seconds": round(window, 2),
        "clee_cpu_seconds": None if None in (start[0], end[0]) else round(end[0] - start[0], 2),
        "mpv_cpu_seconds": None if None in (start[1], end[1]) else round(end[1] - start[1], 2),
        "clee_percent": None if None in (start[0], end[0]) else round((end[0] - start[0]) / window * 100, 1),
        "mpv_percent": None if None in (start[1], end[1]) else round((end[1] - start[1]) / window * 100, 1),
        "idle_percent": None if None in (idle_before, idle_after) else round((idle_after - idle_before) / 2.0 * 100, 1),
    }


def main(argv):
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", default=os.path.join(ROOT, "target", "release", "clee"))
    parser.add_argument("--shm", default="yes", choices=["yes", "no"])
    parser.add_argument("--seconds", type=float, default=12.0)
    parser.add_argument("--runs", type=int, default=3)
    parser.add_argument("--tag", default="A")
    parser.add_argument("--sample", default=None, help="also run /usr/bin/sample into this file")
    parser.add_argument("--off", action="store_true", help="run with the profile switched off")
    args = parser.parse_args(argv[1:])

    os.makedirs(WORK, exist_ok=True)
    print(f"measuring {args.binary}\nworking in {WORK}", flush=True)
    for run in range(1, args.runs + 1):
        path = None if args.off else os.path.join(WORK, f"profile-{args.tag}-shm{args.shm}-{run}.json")
        result = one_run(args.binary, path, args.shm, args.seconds, args.sample)
        result["profile"] = path
        result["exists"] = bool(path) and os.path.exists(path)
        print(f"run {run}: {result}", flush=True)


if __name__ == "__main__":
    main(sys.argv)

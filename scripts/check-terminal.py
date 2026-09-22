#!/usr/bin/env python3
"""Exercise the compiled Unix TUI and recorder without PostgreSQL or third-party packages."""
import fcntl
import json
import os
from pathlib import Path
import pty
import select
import signal
import socket
import sqlite3
import struct
import subprocess
import sys
import tempfile
import termios
import threading
import time

BINARY = str(Path(sys.argv[1] if len(sys.argv) > 1 else "target/debug/pgtrail").resolve())


def environment():
    env = dict(os.environ, TERM="xterm-256color")
    for name in ("PGTRAIL_DATABASE_URL", "PGTRAIL_STORE", "PGTRAIL_PROFILES"):
        env.pop(name, None)
    return env


def terminal(store, arguments, actions, database_url=None):
    master, slave = pty.openpty()
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 36, 140, 0, 0))
    original = termios.tcgetattr(slave)
    env = environment()
    if database_url:
        env["PGTRAIL_DATABASE_URL"] = database_url
    process = subprocess.Popen([BINARY, "--store", str(store), *arguments], stdin=slave, stdout=slave, stderr=slave, env=env)
    output = bytearray()

    def pump(seconds=0.15):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            if select.select([master], [], [], min(0.03, max(0, deadline - time.monotonic())))[0]:
                try:
                    output.extend(os.read(master, 65536))
                except OSError:
                    break

    def key(value):
        os.write(master, value)
        pump()

    def until(predicate, label):
        deadline = time.monotonic() + 8
        while not predicate() and time.monotonic() < deadline:
            pump()
        assert predicate(), f"{label}\n{bytes(output[-1600:])!r}"

    try:
        until(lambda: b"pgtrail" in output, "TUI did not start")
        actions(process, slave, key, pump, until, output)
        process.wait(timeout=3)
        pump()
        assert process.returncode == 0, bytes(output[-1600:])
        assert termios.tcgetattr(slave) == original, "Terminal settings were not restored"
        assert b"\x1b[?1049h" in output and b"\x1b[?1049l" in output, "Alternate screen was not restored"
    finally:
        if process.poll() is None:
            process.kill()
            process.wait()
        os.close(master)
        os.close(slave)


def scalar(store, sql):
    if not store.exists():
        return None
    try:
        with sqlite3.connect(store, timeout=0.1) as connection:
            return connection.execute(sql).fetchone()[0]
    except sqlite3.OperationalError:
        return None


def exercise(store):
    def demo(process, slave, key, pump, until, output):
        until(lambda: b"SYNTHETIC DATA" in output, "Demo provenance missing")
        for view in (b"2", b"3", b"4", b"v", b"6", b"7", b"v", b"s", b"8", b"9", b"0", b"1"):
            key(view)
        key(b"\r")
        until(lambda: b"Finding details" in output, "Finding drilldown missing")
        key(b"\x1b")
        key(b"iQuery queue")
        assert process.poll() is None, "q in incident title quit the app"
        key(b"\r")
        until(lambda: scalar(store, "SELECT count(*) FROM incidents") == 1, "Incident was not created")
        until(lambda: b"activated incident #1" in output, "New incident was not activated")
        key(b"nObserved a blocked checkout")
        key(b"\r")
        until(lambda: scalar(store, "SELECT count(*) FROM incident_notes") == 1, "Incident note was not saved")
        key(b"c")
        until(lambda: scalar(store, "SELECT count(*) FROM snapshots") == 1, "First capture missing")
        # Wait for the save result before requesting the next capture.
        until(lambda: b"Saved capture #1" in output, "First save acknowledgement missing")
        key(b"c")
        until(lambda: scalar(store, "SELECT count(*) FROM snapshots") == 2, "Second capture missing")
        assert scalar(store, "SELECT count(*) FROM incident_snapshots") == 2
        key(b"5")
        key(b"g")
        key(b"b")
        key(b"j")
        key(b"a")
        key(b"d")
        until(lambda: b"OFFLINE COMPARISON" in output, "Offline comparison missing")
        key(b"\x1b")
        key(b"?")
        key(b"\x1b")
        key(b"/q")
        assert process.poll() is None, "q in filter quit the app"
        key(b"\r")
        key(b"\x1b")
        for width, height in ((30, 10), (1, 1), (140, 36)):
            fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", height, width, 0, 0))
            os.kill(process.pid, signal.SIGWINCH)
            pump()
            assert process.poll() is None, "Resize terminated the app"
        key(b"q")

    terminal(store, ["--demo", "--refresh", "60"], demo)

    def offline(process, slave, key, pump, until, output):
        until(lambda: b"NO LIVE CONNECTION" in output, "Disconnected state missing")
        key(b"5")
        until(lambda: b"Manual TUI capture" in output, "Offline history unavailable")
        key(b"\r")
        until(lambda: b"OFFLINE  Esc: return" in output, "Offline inspection missing")
        for view in (b"6", b"7", b"8", b"9"):
            key(view)
        key(b"\x1b")
        key(b"1")
        key(b"\x03")

    terminal(store, [], offline)

    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        listener.listen()
        listener.settimeout(5)
        accepted = []

        def accept():
            try:
                accepted.append(listener.accept()[0])
            except OSError:
                pass

        worker = threading.Thread(target=accept, daemon=True)
        worker.start()
        try:
            def stalled(process, slave, key, pump, until, output):
                until(lambda: bool(accepted), "Collector never connected to the test server")
                started = time.monotonic()
                key(b"\x03")
                process.wait(timeout=2)
                assert time.monotonic() - started < 2, "Ctrl+C blocked on collection"

            port = listener.getsockname()[1]
            terminal(store, ["--timeout", "10"], stalled, f"postgresql://monitor@127.0.0.1:{port}/test?sslmode=disable")
        finally:
            for connection in accepted:
                connection.close()
            worker.join(timeout=0.1)


def interrupt_recording(store):
    process = subprocess.Popen([BINARY, "--demo", "--store", str(store), "record", "--count", "10", "--interval", "1"], env=environment(), stdout=subprocess.PIPE, stderr=subprocess.PIPE)
    writer = None
    try:
        assert select.select([process.stdout], [], [], 8)[0], "Recorder produced no first capture"
        assert b"Saved capture #1" in process.stdout.readline()
        writer = sqlite3.connect(store)
        writer.execute("BEGIN IMMEDIATE")
        # The second sample reaches a blocked SQLite save. Ctrl+C must remain
        # observed throughout writes, not only during collection or interval waits.
        time.sleep(1.4)
        os.kill(process.pid, signal.SIGINT)
        stdout, stderr = process.communicate(timeout=2)
        assert process.returncode == 0, stderr
        assert b"Recording interrupted" in stdout
        writer.rollback()
        assert scalar(store, "SELECT count(*) FROM snapshots") == 1
    finally:
        if writer:
            writer.close()
        if process.poll() is None:
            process.kill()
            process.wait()


with tempfile.TemporaryDirectory(prefix="pgtrail-terminal-check-") as directory:
    folder = Path(directory)
    exercise(folder / "history.sqlite3")
    interrupt_recording(folder / "recorder.sqlite3")
print(json.dumps({"result": "passed", "checks": ["ten views and finding drilldown", "incident create/note/capture", "offline compare and restart", "filter/editor shortcuts", "resize and terminal restoration", "Ctrl+C during stalled PostgreSQL", "Ctrl+C during blocked SQLite recording"]}))

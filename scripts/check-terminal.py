#!/usr/bin/env python3
"""Exercise the compiled Unix TUI and recorder without PostgreSQL or third-party packages."""
import fcntl
import codecs
import json
import os
from pathlib import Path
import pty
import re
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
import unicodedata

BINARY = str(Path(sys.argv[1] if len(sys.argv) > 1 else "target/debug/pgtrail").resolve())


class TerminalOutput(bytearray):
    """Retain emitted bytes and reconstruct cursor-addressed text for assertions.

    Ratatui writes differences, so a visible title need not occur contiguously in
    raw output. This limited CSI decoder covers the sequences used by crossterm;
    rendering geometry and color assertions also run against Rust TestBackend.
    """

    def __init__(self, size):
        super().__init__()
        self.width, self.height = size
        self.decoder = codecs.getincrementaldecoder("utf-8")("replace")
        self.pending = ""
        self.cells = {}
        self.row = self.column = 1
        self.entered = self.exited = False

    def extend(self, data):
        super().extend(data)
        text = self.pending + self.decoder.decode(data)
        self.pending = ""
        index = 0
        while index < len(text):
            char = text[index]
            if char == "\x1b":
                match = re.match(r"\x1b\[([0-?]*)([ -/]*)([@-~])", text[index:])
                if not match:
                    self.pending = text[index:]
                    break
                params, _, command = match.groups()
                numbers = [int(value) if value else 0 for value in params.lstrip("?").split(";")]
                first = numbers[0] or 1
                if params == "?1049" and command == "h":
                    self.entered = True
                elif params == "?1049" and command == "l":
                    self.exited = True
                elif command in ("H", "f"):
                    self.row = first
                    self.column = (numbers[1] if len(numbers) > 1 else 1) or 1
                elif command == "G":
                    self.column = first
                elif command == "d":
                    self.row = first
                elif command in "ABCD":
                    self.row += first * ((command == "B") - (command == "A"))
                    self.column += first * ((command == "C") - (command == "D"))
                elif command == "J" and numbers[0] in (2, 3):
                    self.cells.clear()
                elif command == "K":
                    for position in list(self.cells):
                        if position[0] == self.row and (numbers[0] == 2 or (numbers[0] == 0 and position[1] >= self.column) or (numbers[0] == 1 and position[1] <= self.column)):
                            self.cells.pop(position)
                index += len(match.group())
                continue
            if char == "\r":
                self.column = 1
            elif char == "\n":
                self.row += 1
            elif not unicodedata.category(char).startswith("C"):
                self.cells[(self.row, self.column)] = char
                self.column += 0 if unicodedata.combining(char) else (2 if unicodedata.east_asian_width(char) in "WF" else 1)
            index += 1

    def visible(self):
        return "\n".join("".join(self.cells.get((row, column), " ") for column in range(1, self.width + 1)) for row in range(1, self.height + 1))

    def __contains__(self, value):
        return super().__contains__(value) or value.decode("utf-8") in self.visible()


def environment():
    env = dict(os.environ, TERM="xterm-256color")
    for name in ("PGTRAIL_DATABASE_URL", "PGTRAIL_STORE", "PGTRAIL_PROFILES"):
        env.pop(name, None)
    return env


def terminal(store, arguments, actions, database_url=None, size=(120, 36)):
    master, slave = pty.openpty()
    width, height = size
    fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", height, width, 0, 0))
    original = termios.tcgetattr(slave)
    env = environment()
    if database_url:
        env["PGTRAIL_DATABASE_URL"] = database_url
    process = subprocess.Popen([BINARY, "--store", str(store), *arguments], stdin=slave, stdout=slave, stderr=slave, env=env)
    output = TerminalOutput(size)

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
        assert output.entered and output.exited, "Alternate screen was not restored"
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


def exercise(store, size):
    def demo(process, slave, key, pump, until, output):
        until(lambda: b"SYNTHETIC DATA" in output, "Demo provenance missing")
        for view in (b"2", b"3", b"4", b"v", b"6", b"7", b"v", b"s", b"8", b"9", b"0", b"1"):
            key(view)
        key(b"\r")
        until(lambda: b"Finding details" in output, "Finding drilldown missing")
        key(b"\x1b")
        key(b"/blocking\r")
        key(b"e")
        until(lambda: b"FROZEN EVIDENCE" in output, "Finding evidence navigation missing")
        key(b"w")
        key(b"b")
        key(b"\x7f")
        key(b"\x7f")
        key(b"\x7f")
        key(b"\x1b")
        key(b"h")
        until(lambda: b"Coverage" in output, "Coverage details missing")
        key(b"\x1b")
        for view, title in ((b"2", b"Session details"), (b"3", b"Blocking details"), (b"4", b"Statement details"), (b"7", b"Relation details")):
            key(view)
            if view in (b"4", b"7"):
                key(b"v")
            key(b"\r")
            until(lambda title=title: title in output, f"Full detail view missing: {title!r}")
            key(b"]")
            key(b"\x1b[6~")
            key(b"G")
            key(b"g")
            key(b"\x1b")
        key(b"1")
        key(b"iQuery queue")
        assert process.poll() is None, "q in incident title quit the app"
        key(b"\r")
        until(lambda: scalar(store, "SELECT count(*) FROM incidents") == 1, "Incident was not created")
        until(lambda: b"activated incident #1" in output, "New incident was not activated")
        key(b"nObserved a blocked checkout")
        key(b"\r")
        until(lambda: scalar(store, "SELECT count(*) FROM incident_notes") == 1, "Incident note was not saved")
        for index in range(2, 9):
            key(f"nChronology note {index}".encode())
            key(b"\r")
            until(lambda index=index: scalar(store, "SELECT count(*) FROM incident_notes") == index, "Incident chronology note missing")
        key(b"c")
        until(lambda: scalar(store, "SELECT count(*) FROM snapshots") == 1, "First capture missing")
        # Wait for the save result before requesting the next capture.
        until(lambda: b"Saved capture #1" in output, "First save acknowledgement missing")
        key(b"c")
        until(lambda: scalar(store, "SELECT count(*) FROM snapshots") == 2, "Second capture missing")
        until(lambda: b"Saved capture #2" in output, "Second save acknowledgement missing")
        for count in range(3, 7):
            key(b"c")
            until(lambda count=count: scalar(store, "SELECT count(*) FROM snapshots") == count, "Additional incident capture missing")
            until(lambda count=count: f"Saved capture #{count}".encode() in output, "Capture acknowledgement missing")
        assert scalar(store, "SELECT count(*) FROM incident_snapshots") == 6
        key(b"5")
        key(b"g")
        key(b"b")
        key(b"j")
        key(b"a")
        key(b"d")
        until(lambda: b"OFFLINE COMPARISON" in output, "Offline comparison missing")
        key(b"\x1b")
        key(b"?")
        key(b"\x1b[6~")
        key(b"G")
        until(lambda: "Quit and restore the terminal" in output.visible(), "Last help actions unreachable")
        key(b"\x1b")
        key(b"/q")
        assert process.poll() is None, "q in filter quit the app"
        key(b"\r")
        key(b"\x1b")
        # Extend one disposable note directly, avoiding thousands of individual
        # key-render cycles while checking the real report's full-text access.
        with sqlite3.connect(store) as connection:
            note = "Observed a blocked checkout " + "timeline evidence " * 200 + "END_OF_LONG_NOTE"
            connection.execute("UPDATE incident_notes SET text=? WHERE id=(SELECT min(id) FROM incident_notes)", (note,))
        key(b"0")
        key(b"\r")
        until(lambda: b"chronology" in output.lower(), "Incident chronology missing")
        key(b"g")
        key(b"\r")
        until(lambda: b"Incident note" in output, "Oldest incident note not accessible")
        key(b"G")
        until(lambda: "END_OF_LONG_NOTE" in output.visible(), "End of long incident note is unreachable")
        key(b"\x1b")
        key(b"G")
        key(b"\r")
        until(lambda: b"OFFLINE CAPTURE" in output, "Timeline capture did not open")
        key(b"\x1b")
        markdown = store.with_suffix(".md")
        json_export = store.with_suffix(".json")
        key(b"e" + str(markdown).encode())
        key(b"\r")
        until(markdown.exists, "Markdown export was not written")
        assert "Chronology note 6" in markdown.read_text()
        assert markdown.stat().st_mode & 0o777 == 0o600, "Export is not private"
        key(b"E" + str(json_export).encode())
        key(b"\r")
        until(json_export.exists, "JSON export was not written")
        assert "Chronology note 6" in json_export.read_text()
        original_export = markdown.read_bytes()
        key(b"e" + str(markdown).encode())
        key(b"\r")
        until(lambda: b"Export failed" in output, "Existing export was not rejected")
        assert markdown.read_bytes() == original_export
        for width, height in ((30, 10), (1, 1), size):
            output.width, output.height = width, height
            fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", height, width, 0, 0))
            os.kill(process.pid, signal.SIGWINCH)
            pump()
            assert process.poll() is None, "Resize terminated the app"
        key(b"q")

    terminal(store, ["--demo", "--include-query-text", "--refresh", "60"], demo, size=size)

    # Mutate only this disposable synthetic fixture to exercise SQL much longer
    # than a viewport. No network source or operational capture is used.
    query = "SELECT " + "fixture_column, " * 400 + "\nFROM local_fixture; END_OF_LONG_SQL"
    with sqlite3.connect(store) as connection:
        for capture_id, payload in connection.execute("SELECT id, payload FROM snapshots").fetchall():
            snapshot = json.loads(payload)
            for session in snapshot["activity"]["data"]:
                session["query"] = query
            for statement in snapshot["statements"]["data"]["entries"]:
                statement["query"] = query
            connection.execute("UPDATE snapshots SET payload=? WHERE id=?", (json.dumps(snapshot), capture_id))

    def offline(process, slave, key, pump, until, output):
        until(lambda: b"NO LIVE CONNECTION" in output, "Disconnected state missing")
        key(b"5")
        until(lambda: b"Manual TUI capture" in output, "Offline history unavailable")
        key(b"G")
        key(b"\r")
        until(lambda: b"OFFLINE CAPTURE" in output, "Offline inspection missing")
        until(lambda: b"Capture #1" in output, "Offline capture identity missing")
        for view in (b"2", b"3", b"4"):
            key(view)
            key(b"\r")
            output.clear()
            key(b"G")
            until(lambda: "END_OF_LONG_SQL" in output.visible(), "End of long captured SQL is unreachable")
            last_row = int(re.search(r"rows (\d+)", output.visible()).group(1))
            key(b"k")
            previous_row = int(re.search(r"rows (\d+)", output.visible()).group(1))
            assert previous_row == last_row - 1, "Up after End did not move one displayed row"
            key(b"\x1b")
        key(b"2/checkout\r")
        key(b"4")
        key(b"/fixture_column\r")
        key(b"v")
        key(b"v")
        key(b"\x1b")
        for view in (b"6", b"7", b"8", b"9"):
            key(view)
        key(b"\x1b")
        key(b"1")
        key(b"\x03")

    terminal(store, [], offline, size=size)

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
            terminal(store, ["--timeout", "10"], stalled, f"postgresql://monitor@127.0.0.1:{port}/test?sslmode=disable", size=size)
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
    for size in ((120, 36), (80, 24)):
        exercise(folder / f"history-{size[0]}x{size[1]}.sqlite3", size)
    interrupt_recording(folder / "recorder.sqlite3")
print(json.dumps({"result": "passed", "sizes": ["120x36", "80x24"], "checks": ["ten views, full details and section navigation", "finding evidence and return", "scrollable help and long captured SQL", "incident chronology, note and attached capture", "Markdown/JSON export and overwrite protection", "offline compare and restart", "filter/editor shortcuts", "resize and terminal restoration", "Ctrl+C during stalled PostgreSQL", "Ctrl+C during blocked SQLite recording"]}))

#!/usr/bin/env python3
"""Read-only screen probe for a DETACHED a2amx session.

Attaches in a pty at the session's own size, sends no input, renders the
snapshot into a small character grid and prints the last non-blank rows,
then drops the connection. Plain `attach` (no --force) refuses a session that
someone is attached to, so a live client is never displaced; still, only run
it against sessions you know are detached.

    scripts/screen-probe.py s2            # last 8 non-blank rows of session s2
    scripts/screen-probe.py s2 --rows 20  # more rows
    scripts/screen-probe.py s2 --idle 14  # count bytes sent after the first screen

Side effect: the daemon keeps the attach client's size (one row less, for the
status line) until the next attach. Set A2AMX_BIN to use another binary.
"""
import argparse, fcntl, os, pty, re, select, signal, struct, subprocess, sys, termios, time

BIN = os.environ.get("A2AMX_BIN", os.path.expanduser("~/.a2amx/bin/a2amx"))
CSI = re.compile(r"\x1b\[([0-9;?]*)([A-Za-z@`])")
OTHER_ESC = re.compile(r"\x1b\][^\x07\x1b]*(\x07|\x1b\\)|\x1b[()][0-9A-B]|\x1b.")


def session_size(sid):
    listing = subprocess.run([BIN, "list"], capture_output=True, text=True, check=True).stdout
    for line in listing.splitlines()[1:]:
        fields = line.split()
        if fields and fields[0] == sid:
            for field in fields:
                if re.fullmatch(r"\d+x\d+", field):
                    cols, rows = field.split("x")
                    return int(cols), int(rows)
    sys.exit(f"session {sid} not found in `a2amx list`")


def capture(sid, cols, rows, secs):
    pid, fd = pty.fork()
    if pid == 0:
        os.execv(BIN, [BIN, "attach", sid])
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))
    start = time.time()
    events = []
    while time.time() - start < secs:
        ready, _, _ = select.select([fd], [], [], 0.2)
        if ready:
            try:
                data = os.read(fd, 65536)
            except OSError:
                break
            if not data:
                break
            events.append((time.time() - start, data))
    os.kill(pid, signal.SIGTERM)
    try:
        os.waitpid(pid, 0)
    except ChildProcessError:
        pass
    return events


def render(raw, cols, rows):
    """Minimal grid: cursor moves, line erase, clear screen; styles are ignored."""
    grid = [[" "] * cols for _ in range(rows)]
    r = c = 0
    text = raw.decode("utf-8", "replace")
    i = 0
    while i < len(text):
        ch = text[i]
        if ch == "\x1b":
            m = CSI.match(text, i)
            if m:
                args, cmd = m.group(1), m.group(2)
                nums = [int(x) if x.isdigit() else 0 for x in args.replace("?", "").split(";")] if args else []
                first = nums[0] if nums and nums[0] else 1
                if cmd in "Hf":
                    r = max(0, first - 1)
                    c = max(0, (nums[1] if len(nums) > 1 and nums[1] else 1) - 1)
                elif cmd == "J" and nums[:1] in ([2], [3]):
                    grid = [[" "] * cols for _ in range(rows)]
                elif cmd == "K":
                    for x in range(c, cols):
                        grid[min(r, rows - 1)][x] = " "
                elif cmd == "C":
                    c += first
                elif cmd == "G":
                    c = max(0, first - 1)
                i = m.end()
                continue
            m = OTHER_ESC.match(text, i)
            i = m.end() if m else i + 1
            continue
        if ch == "\r":
            c = 0
        elif ch == "\n":
            r = min(r + 1, rows - 1)
        elif ch >= " ":
            if r < rows and c < cols:
                grid[r][c] = ch
            c += 1
        i += 1
    return ["".join(row).rstrip() for row in grid]


def main():
    parser = argparse.ArgumentParser(description="Read-only screen probe for a detached a2amx session")
    parser.add_argument("session", help="session id from `a2amx list`, e.g. s2")
    parser.add_argument("--rows", type=int, default=8, help="how many non-blank rows to print")
    parser.add_argument("--idle", type=float, help="seconds to watch; report bytes after the first 3s")
    args = parser.parse_args()

    cols, rows = session_size(args.session)
    events = capture(args.session, cols, rows, args.idle if args.idle else 4)
    if args.idle:
        settled = [d for t, d in events if t > 3.0]
        print(f"[{args.session}] chunks={len(events)} bytes={sum(len(d) for _, d in events)}; "
              f"after 3s: chunks={len(settled)} bytes={sum(len(d) for d in settled)}")
        return
    lines = render(b"".join(d for _, d in events), cols, rows)
    shown = [(n, line) for n, line in enumerate(lines) if line.strip()][-args.rows:]
    print(f"[{args.session}] {cols}x{rows}, last {len(shown)} non-blank rows")
    for n, line in shown:
        print(f"{n:02d}| {line}")


if __name__ == "__main__":
    main()

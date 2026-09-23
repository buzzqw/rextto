#!/usr/bin/env python3
"""Rextto TUI in Python — terminal client for a running Rextto daemon.

Uses only the standard library (curses + urllib), so there is nothing to
install:

    python3 scripts/rextto_tui.py
    REXTTO_URL=http://127.0.0.1:5000 REXTTO_API_TOKEN=... python3 scripts/rextto_tui.py

Keys: 1-5/Tab switch tabs · r refresh · c run cycle · p pause/resume selected
torrent · ↑↓ select · q quit.

It only talks to the daemon's HTTP API and never touches the databases.
"""

from __future__ import annotations

import curses
import json
import os
import sys
import time
import urllib.error
import urllib.request

BASE = os.environ.get("REXTTO_URL", "http://127.0.0.1:5000").rstrip("/")
TOKEN = os.environ.get("REXTTO_API_TOKEN", "").strip()
REFRESH_SECS = 2.0
TABS = ["Status", "Torrents", "Activity", "Logs", "Health"]


def api(path: str, method: str = "GET"):
    """Calls the daemon and returns parsed JSON, or raises RuntimeError."""
    url = f"{BASE}{path}"
    headers = {"Accept": "application/json"}
    if TOKEN:
        headers["x-rextto-token"] = TOKEN
    data = None
    if method == "POST":
        data = b"{}"
        headers["Content-Type"] = "application/json"
    request = urllib.request.Request(url, data=data, headers=headers, method=method)
    try:
        with urllib.request.urlopen(request, timeout=15) as response:
            return json.loads(response.read().decode("utf-8", "replace"))
    except urllib.error.HTTPError as error:
        detail = error.read().decode("utf-8", "replace")
        raise RuntimeError(f"HTTP {error.code}: {detail[:200]}") from error
    except (urllib.error.URLError, TimeoutError, OSError) as error:
        raise RuntimeError(str(error)) from error


def human_bytes(value: float) -> str:
    units = ["B", "KB", "MB", "GB", "TB"]
    value = max(0.0, float(value))
    unit = 0
    while value >= 1024.0 and unit < len(units) - 1:
        value /= 1024.0
        unit += 1
    return f"{value:.0f} {units[unit]}" if unit == 0 else f"{value:.1f} {units[unit]}"


def text(item, key: str, default: str = "") -> str:
    if not isinstance(item, dict):
        return default
    value = item.get(key)
    if value is None:
        return default
    if isinstance(value, str):
        return value
    if isinstance(value, bool):
        return "true" if value else "false"
    return str(value)


def shorten(value: str, width: int) -> str:
    if width <= 1:
        return value[:width]
    return value if len(value) <= width else value[: width - 1] + "…"


class App:
    def __init__(self) -> None:
        self.tab = 0
        self.selected = 0
        self.message = ""
        self.last_refresh = 0.0
        self.error = ""
        self.status = {}
        self.torrents = []
        self.events = []
        self.logs = []
        self.health = {}

    def refresh(self) -> None:
        try:
            self.status = api("/api/status")
            self.torrents = api("/api/torrents") or []
            self.events = api("/api/torrent-events") or []
            self.logs = (api("/api/logs?limit=200") or {}).get("items", [])
            self.health = api("/api/health") or {}
            self.error = ""
        except RuntimeError as error:
            self.error = str(error)
        self.last_refresh = time.time()
        count = len(self.torrents)
        if count == 0:
            self.selected = 0
        else:
            self.selected = min(self.selected, count - 1)

    def run_cycle(self) -> None:
        try:
            api("/api/run-now", "POST")
            self.message = "cycle requested"
        except RuntimeError as error:
            self.message = f"cycle failed: {error}"

    def toggle_selected(self) -> None:
        if not self.torrents:
            self.message = "no torrent selected"
            return
        torrent = self.torrents[self.selected]
        hashes = text(torrent, "hash")
        if not hashes:
            return
        action = "resume" if text(torrent, "state") == "paused" else "pause"
        try:
            api(f"/api/torrents/{hashes}/{action}", "POST")
            self.message = f"{action} {hashes[:8]}"
        except RuntimeError as error:
            self.message = f"{action} failed: {error}"


def add(win, y: int, x: int, value: str, attr: int = 0) -> None:
    height, width = win.getmaxyx()
    if y < 0 or y >= height or x >= width:
        return
    value = value[: max(0, width - x - 1)]
    try:
        win.addstr(y, x, value, attr)
    except curses.error:
        pass


def draw(app: "App", win, colors: dict) -> None:
    win.erase()
    height, width = win.getmaxyx()
    if height < 6 or width < 40:
        add(win, 0, 0, "terminal too small", curses.A_BOLD)
        win.refresh()
        return

    # Header.
    name = text(app.status, "name", "rextto")
    version = text(app.status, "version")
    active = bool(app.status.get("active"))
    add(win, 0, 1, f"rextto v{version}  ", colors["header"] | curses.A_BOLD)
    add(win, 0, 12, "ACTIVE" if active else "PAUSED",
        colors["ok"] if active else colors["warn"])

    # Tabs.
    x = 2
    for index, label in enumerate(TABS):
        attr = colors["tab_active"] | curses.A_BOLD if index == app.tab else colors["muted"]
        add(win, 1, x, f" {index + 1}:{label} ", attr)
        x += len(label) + 6
    add(win, 1, x, "  (Tab/1-5 to switch)", colors["muted"])

    # Body.
    top = 3
    bottom = height - 2
    if app.error:
        add(win, top, 2, f"cannot reach daemon: {app.error}", colors["err"])
        add(win, top + 2, 2, "set REXTTO_URL / REXTTO_API_TOKEN", colors["muted"])
    elif app.tab == 0:
        draw_status(app, win, top, bottom, width, colors)
    elif app.tab == 1:
        draw_torrents(app, win, top, bottom, width, colors)
    elif app.tab == 2:
        draw_events(app, win, top, bottom, width, colors)
    elif app.tab == 3:
        draw_lines(app.logs[-max(1, bottom - top):], win, top, bottom, width, colors)
    else:
        body = json.dumps(app.health, indent=2).splitlines()
        draw_lines(body[: max(1, bottom - top)], win, top, bottom, width, colors)

    # Footer.
    help_text = " q quit · r refresh · c cycle · p pause/resume · ↑↓ select"
    add(win, height - 1, 1, help_text, colors["muted"])
    if app.message:
        add(win, height - 1, len(help_text) + 3, app.message, colors["ok"])
    win.refresh()


def draw_lines(lines, win, top, bottom, width, colors) -> None:
    for offset, line in enumerate(lines):
        y = top + offset
        if y >= bottom:
            break
        attr = colors["err"] if " ERROR " in line else (
            colors["warn"] if " WARN " in line else colors["normal"])
        add(win, y, 2, line, attr)


def draw_status(app, win, top, bottom, width, colors) -> None:
    status = app.status
    torrents = status.get("torrent_stats", {})
    cycle = status.get("last_cycle", {})
    seen = status.get("seen", {})
    lines = [
        f"Torrents: {text(torrents,'count')} ({text(torrents,'downloading')} downloading, "
        f"{text(torrents,'queued')} queued, {text(torrents,'seeding')} seeding)",
        f"Last cycle: scraped {text(cycle,'scraped')} | candidates {text(cycle,'candidates')} "
        f"| downloads {text(cycle,'downloads_started')} | errors {text(cycle,'errors')}",
        f"Seen in feeds: groups {text(seen,'groups')} · movies {text(seen,'movies')} "
        f"· series {text(seen,'series')}",
    ]
    for offset, line in enumerate(lines):
        add(win, top + offset, 2, line, colors["normal"])


def draw_torrents(app, win, top, bottom, width, colors) -> None:
    add(win, top, 2, f"{'HASH':<9} {'STATE':<12} {'PROG':>6} {'DONE':>10} "
                     f"{'DOWN':>10} {'UP':>10}  NAME", colors["header"])
    name_width = max(10, width - 64)
    visible = app.torrents[: max(1, bottom - top - 1)]
    for index, torrent in enumerate(visible):
        y = top + 1 + index
        if y >= bottom:
            break
        attr = colors["tab_active"] if index == app.selected else colors["normal"]
        progress = float(torrent.get("progress") or 0.0)
        done = human_bytes(torrent.get("total_done") or 0)
        down = human_bytes(torrent.get("download_rate") or 0)
        up = human_bytes(torrent.get("upload_rate") or 0)
        line = (f"{shorten(text(torrent,'hash'),9):<9} {shorten(text(torrent,'state'),12):<12} "
                f"{progress:>5.1f}% {done:>10} {down + '/s':>10} {up + '/s':>10}  "
                f"{shorten(text(torrent,'name'), name_width)}")
        add(win, y, 2, line, attr)


def draw_events(app, win, top, bottom, width, colors) -> None:
    events = list(reversed(app.events))[: max(1, bottom - top)]
    for index, event in enumerate(events):
        y = top + index
        if y >= bottom:
            break
        line = f"{shorten(text(event,'kind'),18):<18} {shorten(text(event,'hash'),9):<9} {text(event,'name')}"
        add(win, y, 2, line, colors["normal"])


def main(stdscr) -> None:
    curses.curs_set(0)
    stdscr.keypad(True)
    stdscr.timeout(200)
    colors = build_colors()

    app = App()
    app.refresh()

    while True:
        draw(app, stdscr, colors)
        key = stdscr.getch()
        if key in (ord("q"), 27):
            break
        if key in (9, curses.KEY_RIGHT):
            app.tab = (app.tab + 1) % len(TABS)
        elif key in (getattr(curses, "KEY_BTAB", 353), curses.KEY_LEFT):
            app.tab = (app.tab - 1) % len(TABS)
        elif ord("1") <= key <= ord("5"):
            app.tab = key - ord("1")
        elif key == ord("r"):
            app.refresh()
            app.message = "refreshed"
        elif key == ord("c"):
            app.run_cycle()
        elif key == ord("p"):
            app.toggle_selected()
        elif key == curses.KEY_DOWN:
            if app.torrents:
                app.selected = min(app.selected + 1, len(app.torrents) - 1)
        elif key == curses.KEY_UP:
            app.selected = max(0, app.selected - 1)

        if time.time() - app.last_refresh >= REFRESH_SECS:
            app.refresh()


def build_colors() -> dict:
    colors = {
        "normal": curses.A_NORMAL,
        "muted": curses.A_DIM,
        "header": curses.A_NORMAL,
        "tab_active": curses.A_NORMAL,
        "ok": curses.A_NORMAL,
        "warn": curses.A_NORMAL,
        "err": curses.A_NORMAL,
    }
    if not curses.has_colors():
        return colors
    try:
        curses.start_color()
        curses.use_default_colors()
        curses.init_pair(1, curses.COLOR_CYAN, -1)
        curses.init_pair(2, curses.COLOR_GREEN, -1)
        curses.init_pair(3, curses.COLOR_YELLOW, -1)
        curses.init_pair(4, curses.COLOR_RED, -1)
        colors["header"] = curses.color_pair(1) | curses.A_BOLD
        colors["tab_active"] = curses.color_pair(2) | curses.A_BOLD
        colors["ok"] = curses.color_pair(2)
        colors["warn"] = curses.color_pair(3)
        colors["err"] = curses.color_pair(4)
    except curses.error:
        pass
    return colors


if __name__ == "__main__":
    if "-h" in sys.argv or "--help" in sys.argv:
        print(__doc__)
        raise SystemExit(0)
    if not sys.stdout.isatty():
        print("rextto_tui.py needs a terminal; use `rextto-tui status` for text output",
              file=sys.stderr)
        raise SystemExit(2)
    try:
        curses.wrapper(main)
    except KeyboardInterrupt:
        pass

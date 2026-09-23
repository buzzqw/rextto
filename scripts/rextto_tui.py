#!/usr/bin/env python3
"""Rextto TUI in Python — terminal client for a running Rextto daemon.

Uses only the standard library (curses + urllib), so there is nothing to
install:

    python3 scripts/rextto_tui.py
    REXTTO_URL=http://127.0.0.1:5000 REXTTO_API_TOKEN=... python3 scripts/rextto_tui.py

Tabs: Status · Torrents · Activity · Logs · Health.
Keys: 1-5/Tab switch · ↑↓ select · r refresh · q quit.
Global: a add magnet · t add .torrent file · c run cycle.
Torrents tab: p pause/resume · d remove · k recheck · R reannounce · n no-rename.

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


def api(path, method="GET", body=None, raw=None, content_type="application/json"):
    """Calls the daemon and returns parsed JSON, or raises RuntimeError."""
    url = f"{BASE}{path}"
    headers = {"Accept": "application/json"}
    if TOKEN:
        headers["x-rextto-token"] = TOKEN
    data = None
    if raw is not None:
        data = raw
        headers["Content-Type"] = content_type
    elif method == "POST":
        data = json.dumps(body if body is not None else {}).encode()
        headers["Content-Type"] = content_type
    request = urllib.request.Request(url, data=data, headers=headers, method=method)
    try:
        with urllib.request.urlopen(request, timeout=20) as response:
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

    def selected_hash(self):
        if not self.torrents:
            return None
        return text(self.torrents[self.selected], "hash") or None

    # --- actions ---------------------------------------------------------

    def run_cycle(self) -> None:
        try:
            api("/api/run-now", "POST")
            self.message = "cycle requested"
        except RuntimeError as error:
            self.message = f"cycle failed: {error}"

    def add_magnet(self, magnet: str) -> None:
        if not magnet:
            return
        if not magnet.startswith("magnet:"):
            self.message = "not a magnet link"
            return
        try:
            api("/api/send-magnet", "POST", {"magnet": magnet})
            self.message = "magnet added"
        except RuntimeError as error:
            self.message = f"add failed: {error}"

    def add_torrent_file(self, path: str) -> None:
        if not path:
            return
        try:
            with open(path, "rb") as handle:
                data = handle.read()
        except OSError as error:
            self.message = f"file error: {error}"
            return
        if not data:
            self.message = "empty file"
            return
        try:
            api("/api/upload-torrent", "POST", raw=data,
                content_type="application/x-bittorrent")
            self.message = "torrent added"
        except RuntimeError as error:
            self.message = f"add failed: {error}"

    def toggle_selected(self) -> None:
        hash_value = self.selected_hash()
        if not hash_value:
            self.message = "no torrent selected"
            return
        # Read the current state from the daemon: the cached list may be up to
        # REFRESH_SECS old, so a second `p` would otherwise repeat the same
        # action instead of toggling. The detail endpoint nests the fields
        # under "torrent".
        try:
            details = api(f"/api/torrents/{hash_value}")
            current = details.get("torrent") if isinstance(details, dict) else None
            if not isinstance(current, dict):
                current = details
        except RuntimeError:
            current = self.torrents[self.selected]
        paused = text(current, "state") == "paused"
        action = "resume" if paused else "pause"
        self._act(f"/api/torrents/{hash_value}/{action}", action)

    def remove_selected(self, delete_files: bool) -> None:
        hash_value = self.selected_hash()
        if not hash_value:
            self.message = "no torrent selected"
            return
        self._act(f"/api/torrents/{hash_value}/remove",
                  "removed" if not delete_files else "removed with files",
                  {"delete_files": delete_files, "blocklist": False})

    def recheck_selected(self) -> None:
        hash_value = self.selected_hash()
        if hash_value:
            self._act(f"/api/torrents/{hash_value}/recheck", "recheck requested")

    def reannounce_selected(self) -> None:
        hash_value = self.selected_hash()
        if hash_value:
            self._act(f"/api/torrents/{hash_value}/reannounce", "reannounce requested")

    def toggle_no_rename(self) -> None:
        hash_value = self.selected_hash()
        if not hash_value:
            self.message = "no torrent selected"
            return
        try:
            details = api(f"/api/torrents/{hash_value}")
            current = bool(details.get("no_rename"))
            api(f"/api/torrents/{hash_value}/no_rename", "POST", {"value": not current})
            self.message = f"no-rename {'off' if current else 'on'}"
        except RuntimeError as error:
            self.message = f"no-rename failed: {error}"

    def _act(self, path: str, label: str, body=None) -> None:
        try:
            api(path, "POST", body)
            self.message = label
        except RuntimeError as error:
            self.message = f"{label} failed: {error}"


def add(win, y: int, x: int, value: str, attr: int = 0) -> None:
    height, width = win.getmaxyx()
    if y < 0 or y >= height or x >= width:
        return
    try:
        win.addstr(y, x, value[: max(0, width - x - 1)], attr)
    except curses.error:
        pass


def prompt_input(stdscr, label: str) -> str:
    """Single-line input on the last row; Esc cancels, Enter confirms."""
    curses.curs_set(1)
    stdscr.nodelay(False)
    stdscr.timeout(-1)
    height, width = stdscr.getmaxyx()
    value = ""
    while True:
        stdscr.move(height - 1, 0)
        stdscr.clrtoeol()
        add(stdscr, height - 1, 1, (label + value)[: width - 3], curses.A_BOLD)
        stdscr.refresh()
        key = stdscr.getch()
        if key in (10, 13, curses.KEY_ENTER):
            break
        if key == 27:
            value = ""
            break
        if key in (curses.KEY_BACKSPACE, 127, 8):
            value = value[:-1]
        elif 32 <= key < 127:
            value += chr(key)
    curses.curs_set(0)
    stdscr.timeout(200)
    return value.strip()


def confirm(stdscr, label: str) -> bool:
    return prompt_input(stdscr, f"{label} [y/N] ").lower().startswith("y")


def draw(app: "App", win, colors: dict) -> None:
    win.erase()
    height, width = win.getmaxyx()
    if height < 6 or width < 40:
        add(win, 0, 0, "terminal too small", curses.A_BOLD)
        win.refresh()
        return

    name = text(app.status, "name", "rextto")
    version = text(app.status, "version")
    active = bool(app.status.get("active"))
    add(win, 0, 1, f"{name} v{version}  ", colors["header"] | curses.A_BOLD)
    add(win, 0, 14, "ACTIVE" if active else "PAUSED",
        colors["ok"] if active else colors["warn"])

    x = 2
    for index, label in enumerate(TABS):
        attr = colors["tab_active"] | curses.A_BOLD if index == app.tab else colors["muted"]
        add(win, 1, x, f" {index + 1}:{label} ", attr)
        x += len(label) + 6
    add(win, 1, x, "  (Tab/1-5)", colors["muted"])

    top = 3
    bottom = height - 2
    if app.error:
        add(win, top, 2, f"cannot reach daemon: {app.error}", colors["err"])
        add(win, top + 2, 2, "set REXTTO_URL / REXTTO_API_TOKEN", colors["muted"])
    elif app.tab == 0:
        draw_status(app, win, top, colors)
    elif app.tab == 1:
        draw_torrents(app, win, top, bottom, width, colors)
    elif app.tab == 2:
        draw_events(app, win, top, bottom, colors)
    elif app.tab == 3:
        draw_lines(app.logs[-(bottom - top):], win, top, bottom, colors)
    else:
        body = json.dumps(app.health, indent=2).splitlines()
        draw_lines(body[: max(1, bottom - top)], win, top, bottom, colors)

    hints = " q quit · r refresh · a magnet · t file · c cycle"
    if app.tab == 1:
        hints += " · p pause/resume · d remove · k recheck · R reannounce · n no-rename"
    add(win, height - 1, 1, hints, colors["muted"])
    if app.message:
        add(win, height - 1, min(width - 2, len(hints) + 3), f"| {app.message}", colors["ok"])
    win.refresh()


def draw_lines(lines, win, top, bottom, colors) -> None:
    for offset, line in enumerate(lines):
        y = top + offset
        if y >= bottom:
            break
        attr = colors["err"] if " ERROR " in line else (
            colors["warn"] if " WARN " in line else colors["normal"])
        add(win, y, 2, line, attr)


def draw_status(app, win, top, colors) -> None:
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
    for index, torrent in enumerate(app.torrents[: max(1, bottom - top - 1)]):
        y = top + 1 + index
        if y >= bottom:
            break
        attr = colors["tab_active"] if index == app.selected else colors["normal"]
        progress = float(torrent.get("progress") or 0.0)
        line = (f"{shorten(text(torrent,'hash'),9):<9} {shorten(text(torrent,'state'),12):<12} "
                f"{progress:>5.1f}% {human_bytes(torrent.get('total_done') or 0):>10} "
                f"{human_bytes(torrent.get('download_rate') or 0) + '/s':>10} "
                f"{human_bytes(torrent.get('upload_rate') or 0) + '/s':>10}  "
                f"{shorten(text(torrent,'name'), name_width)}")
        add(win, y, 2, line, attr)


def draw_events(app, win, top, bottom, colors) -> None:
    for index, event in enumerate(list(reversed(app.events))[: max(1, bottom - top)]):
        y = top + index
        if y >= bottom:
            break
        add(win, y, 2,
            f"{shorten(text(event,'kind'),18):<18} {shorten(text(event,'hash'),9):<9} {text(event,'name')}",
            colors["normal"])


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
        elif key == ord("a"):
            app.add_magnet(prompt_input(stdscr, "Magnet: "))
        elif key == ord("t"):
            app.add_torrent_file(prompt_input(stdscr, "File .torrent: "))
        elif app.tab == 1 and key == ord("p"):
            app.toggle_selected()
        elif app.tab == 1 and key == ord("d"):
            name = text(app.torrents[app.selected], "name") if app.torrents else "?"
            if app.torrents and confirm(stdscr, f"Remove '{shorten(name, 40)}'?"):
                delete_files = confirm(stdscr, "Also delete downloaded files?")
                app.remove_selected(delete_files)
                app.refresh()
        elif app.tab == 1 and key == ord("k"):
            app.recheck_selected()
        elif app.tab == 1 and key == ord("R"):
            app.reannounce_selected()
        elif app.tab == 1 and key == ord("n"):
            app.toggle_no_rename()
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
        print("rextto_tui.py needs a terminal", file=sys.stderr)
        raise SystemExit(2)
    try:
        curses.wrapper(main)
    except KeyboardInterrupt:
        pass

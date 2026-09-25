#!/usr/bin/env python3
"""Rextto TUI in Python — terminal client for a running Rextto daemon.

Uses only the standard library (curses + urllib), so there is nothing to
install:

    python3 scripts/rextto_tui.py
    REXTTO_URL=http://127.0.0.1:5000 REXTTO_API_TOKEN=... python3 scripts/rextto_tui.py

Tabs: Status · Torrents · Logs · Health.
Keys: 1-4/Tab switch · ↑↓ select · Enter details · ? help · r refresh · q quit.
Global: a add magnet/URL · t add .torrent file · c run cycle · s search · e events.
Torrents tab: p pause/resume · b restart · d remove · X clean completed · k recheck
· R reannounce · n no-rename · i/u pin/unpin · L speed limits.
Logs tab: / filter · f follow · ↑↓/PgUp/PgDn scroll · Home/End.
Health tab: x empty trash.

It only talks to the daemon's HTTP API and never touches the databases.
"""

from __future__ import annotations

import curses
import json
import os
import sys
import textwrap
import threading
import time
import urllib.error
import urllib.request

BASE = os.environ.get("REXTTO_URL", "http://127.0.0.1:5000").rstrip("/")
TOKEN = os.environ.get("REXTTO_API_TOKEN", "").strip()
REFRESH_SECS = 2.0
TABS = ["Status", "Torrents", "Logs", "Health"]


def api(path, method="GET", body=None, raw=None, content_type="application/json", timeout=20):
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
        with urllib.request.urlopen(request, timeout=timeout) as response:
            return json.loads(response.read().decode("utf-8", "replace"))
    except urllib.error.HTTPError as error:
        detail = error.read().decode("utf-8", "replace")
        raise RuntimeError(f"HTTP {error.code}: {detail[:200]}") from error
    except (urllib.error.URLError, TimeoutError, OSError) as error:
        raise RuntimeError(str(error)) from error


def fetch_snapshot():
    """Fetches the data needed by the main view."""
    status = api("/api/status")
    torrents = api("/api/torrents") or []
    torrents = sorted(
        torrents,
        key=lambda torrent: (
            text(torrent, "name").casefold(),
            text(torrent, "hash").casefold(),
        ),
    )
    logs = (api("/api/logs?limit=200") or {}).get("items", [])
    return status, torrents, logs


def fetch_health():
    return api("/api/health") or {}


class LogStream:
    """Consumes the daemon SSE log stream without blocking the curses loop."""

    def __init__(self, app) -> None:
        self.app = app
        self.stop_event = threading.Event()
        self.thread = None

    def start(self) -> None:
        self.thread = threading.Thread(target=self._run, name="rextto-log-stream", daemon=True)
        self.thread.start()

    def stop(self) -> None:
        self.stop_event.set()

    def _run(self) -> None:
        while not self.stop_event.is_set():
            try:
                request = urllib.request.Request(
                    f"{BASE}/api/logs/stream?limit=500",
                    headers={"Accept": "text/event-stream", **({"x-rextto-token": TOKEN} if TOKEN else {})},
                )
                with urllib.request.urlopen(request, timeout=35) as response:
                    self.app.log_stream_connected = True
                    data_lines = []
                    while not self.stop_event.is_set():
                        raw_line = response.readline()
                        if not raw_line:
                            break
                        line = raw_line.decode("utf-8", "replace").rstrip("\r\n")
                        if line.startswith("data:"):
                            data_lines.append(line[5:].lstrip())
                        elif not line and data_lines:
                            self._event("\n".join(data_lines))
                            data_lines = []
            except (urllib.error.URLError, TimeoutError, OSError, ValueError):
                pass
            self.app.log_stream_connected = False
            self.stop_event.wait(2.0)

    def _event(self, payload: str) -> None:
        try:
            value = json.loads(payload)
        except json.JSONDecodeError:
            return
        if isinstance(value, dict) and isinstance(value.get("snapshot"), list):
            with self.app.log_lock:
                self.app.logs = [str(line) for line in value["snapshot"]]
        elif isinstance(value, dict) and "line" in value:
            with self.app.log_lock:
                self.app.logs.append(str(value["line"]))
                self.app.logs = self.app.logs[-500:]


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
        self.logs = []
        self.health = {}
        self.detail = None
        self.help_visible = False
        self.log_filter = ""
        self.log_scroll = 0
        self.log_follow = True
        self.loading = False
        self.health_error = ""
        self.log_lock = threading.Lock()
        self.log_stream_connected = False
        self.log_stream = LogStream(self)
        self.events = []
        self.events_visible = False
        self.search_results = []
        self.search_query = ""
        self.search_selected = 0
        self.search_visible = False
        self.detail_view = "general"
        self.detail_items = []
        self.detail_scroll = 0
        self.log_stream.start()

    def refresh(self) -> None:
        selected_hash = self.selected_hash()
        try:
            status, torrents, logs = fetch_snapshot()
            self.status = status
            self.torrents = torrents
            if not self.log_stream_connected:
                with self.log_lock:
                    self.logs = logs
            self.error = ""
        except RuntimeError as error:
            self.error = str(error)
        try:
            health = fetch_health()
            self.health = health
            self.health_error = ""
        except RuntimeError as error:
            self.health_error = str(error)
        self.loading = False
        self.last_refresh = time.time()
        self._restore_selection(selected_hash)

    def _restore_selection(self, selected_hash) -> None:
        count = len(self.torrents)
        if count == 0:
            self.selected = 0
        elif selected_hash:
            self.selected = next(
                (index for index, torrent in enumerate(self.torrents)
                 if text(torrent, "hash").lower() == selected_hash.lower()),
                min(self.selected, count - 1),
            )
        else:
            self.selected = min(self.selected, count - 1)

    def poll_refresh(self) -> None:
        return

    def close(self) -> None:
        self.log_stream.stop()

    def selected_hash(self):
        if not self.torrents:
            return None
        return text(self.torrents[self.selected], "hash") or None

    def filtered_logs(self):
        with self.log_lock:
            logs = list(self.logs)
        if not self.log_filter:
            return logs
        needle = self.log_filter.casefold()
        return [line for line in logs if needle in line.casefold()]

    def open_selected_details(self) -> None:
        hash_value = self.selected_hash()
        if not hash_value:
            self.message = "no torrent selected"
            return
        try:
            response = api(f"/api/torrents/{hash_value}")
            torrent = response.get("torrent") if isinstance(response, dict) else None
            if not isinstance(torrent, dict):
                torrent = response if isinstance(response, dict) else None
            if torrent is None:
                raise RuntimeError("invalid torrent details")
            self.detail = {
                "torrent": torrent,
                "magnet": text(response, "magnet"),
                "no_rename": bool(response.get("no_rename")),
                "source": text(self.torrents[self.selected], "source"),
                "reason": text(self.torrents[self.selected], "reason"),
                "archived": bool(self.torrents[self.selected].get("archived")),
            }
            self.detail_view = "general"
            self.detail_items = []
            self.detail_scroll = 0
            self.message = ""
        except RuntimeError as error:
            self.message = f"details failed: {error}"

    def close_details(self) -> None:
        self.detail = None

    # --- actions ---------------------------------------------------------

    def run_cycle(self, domain="full") -> None:
        if domain not in ("full", "series", "movies", "comics"):
            self.message = "invalid cycle domain"
            return
        try:
            path = "/api/run-now" if domain == "full" else f"/api/run-now?domain={domain}"
            result = api(path, "POST")
            if isinstance(result, dict) and result.get("queued"):
                self.message = f"{domain} cycle queued (another cycle is running)"
            else:
                self.message = f"{domain} cycle started"
        except RuntimeError as error:
            self.message = f"cycle failed: {error}"

    def restart_selected(self) -> None:
        hash_value = self.selected_hash()
        if hash_value:
            self._act(f"/api/torrents/{hash_value}/restart", "restart requested")

    def pin_selected(self) -> None:
        hash_value = self.selected_hash()
        if hash_value:
            self._act("/api/torrents/pin", "torrent pinned", {"hash": hash_value})

    def unpin(self) -> None:
        self._act("/api/torrents/unpin", "torrent unpinned")

    def clean_completed(self, delete_files: bool) -> None:
        try:
            result = api("/api/torrents/remove_completed", "POST",
                         {"delete_files": delete_files}, timeout=120)
            removed = result.get("removed", 0) if isinstance(result, dict) else 0
            skipped = result.get("skipped", 0) if isinstance(result, dict) else 0
            self.message = f"completed removed: {removed}, skipped: {skipped}"
        except RuntimeError as error:
            self.message = f"cleanup failed: {error}"

    def set_global_speed_limits(self, download: str, upload: str) -> None:
        try:
            dl = max(0, int(download))
            ul = max(0, int(upload))
        except ValueError:
            self.message = "limits must be KiB/s numbers"
            return
        try:
            api("/api/set-speed-limits", "POST",
                {"download_kib": dl, "upload_kib": ul})
            self.message = f"global limits set: {dl}/{ul} KiB/s"
        except RuntimeError as error:
            self.message = f"limits failed: {error}"

    def search(self, query: str) -> None:
        if not query:
            return
        try:
            result = api("/api/search", "POST", {"query": query}, timeout=120)
            self.search_results = result.get("results", []) if isinstance(result, dict) else []
            self.search_query = query
            self.search_selected = 0
            self.search_visible = True
            self.message = f"search: {len(self.search_results)} results"
        except RuntimeError as error:
            self.message = f"search failed: {error}"

    def add_search_result(self) -> None:
        if not self.search_results:
            self.message = "no search result selected"
            return
        release = self.search_results[self.search_selected]
        try:
            api("/api/search/add", "POST", {"release": release}, timeout=120)
            self.message = "search result queued"
            self.search_visible = False
            self.refresh()
        except RuntimeError as error:
            self.message = f"queue failed: {error}"

    def load_events(self) -> None:
        try:
            result = api("/api/torrent-events")
            self.events = result if isinstance(result, list) else []
            self.events_visible = True
            self.message = f"events: {len(self.events)}"
        except RuntimeError as error:
            self.message = f"events failed: {error}"

    def load_detail_view(self, view: str) -> None:
        if view == "general":
            self.detail_view = view
            self.detail_scroll = 0
            return
        hash_value = self.selected_hash()
        if not hash_value:
            return
        endpoint = {
            "trackers": "trackers",
            "files": "files",
            "peers": "peers",
        }.get(view)
        if endpoint is None:
            return
        try:
            result = api(f"/api/torrents/{hash_value}/{endpoint}")
            self.detail_items = result.get(endpoint, []) if isinstance(result, dict) else []
            self.detail_view = view
            self.detail_scroll = 0
        except RuntimeError as error:
            self.message = f"{view} failed: {error}"

    def move_detail_scroll(self, amount: int) -> None:
        self.detail_scroll = max(0, self.detail_scroll + amount)

    def clean_trash(self) -> None:
        try:
            result = api("/api/maintenance/clean-trash", "POST", {"force": True}, timeout=120)
            files = result.get("files") if isinstance(result, dict) else None
            self.message = f"trash cleaned ({files} files)" if files is not None else "trash cleaned"
        except RuntimeError as error:
            self.message = f"trash cleanup failed: {error}"

    def add_magnet(self, magnet: str) -> None:
        if not magnet:
            return
        if not (magnet.startswith("magnet:") or
                magnet.startswith("http://") or
                magnet.startswith("https://")):
            self.message = "not a magnet or torrent URL"
            return
        try:
            api("/api/send-magnet", "POST", {"magnet": magnet})
            self.message = "magnet/URL added"
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


def wrapped_lines(value, width: int) -> list[str]:
    """Return terminal-sized lines, including hard-wrapped long words."""
    if width <= 0:
        return []
    paragraphs = str(value).splitlines() or [""]
    result = []
    for paragraph in paragraphs:
        result.extend(textwrap.wrap(
            paragraph,
            width=width,
            break_long_words=True,
            break_on_hyphens=False,
            replace_whitespace=False,
        ) or [""])
    return result


def add_wrapped(win, y: int, x: int, value, attr: int = 0, max_lines=None) -> int:
    """Draw a value on consecutive rows and return the number of rows used."""
    _, width = win.getmaxyx()
    lines = wrapped_lines(value, max(1, width - x - 1))
    if max_lines is not None:
        lines = lines[:max_lines]
    for offset, line in enumerate(lines):
        add(win, y + offset, x, line, attr)
    return len(lines)


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
    if height < 6 or width < 12:
        add(win, 0, 0, "terminal too small", curses.A_BOLD)
        win.refresh()
        return

    name = text(app.status, "name", "rextto")
    version = text(app.status, "version")
    active = app.status.get("active") if isinstance(app.status, dict) else None
    dry_run = app.status.get("dry_run") if isinstance(app.status, dict) else None
    activity_label = "DRY-RUN" if dry_run is True else (
        "ACTIVE" if active is True else (
            "STANDBY" if active is False else "LOADING"))
    activity_attr = colors["warn"] if dry_run is True else (
        colors["ok"] if active is True else (
            colors["warn"] if active is False else colors["muted"]))
    add(win, 0, 1, f"{name} v{version}  ", colors["header"] | curses.A_BOLD)
    add(win, 0, 14, activity_label, activity_attr)

    x = 2
    for index, label in enumerate(TABS):
        attr = colors["tab_active"] | curses.A_BOLD if index == app.tab else colors["muted"]
        add(win, 1, x, f" {index + 1}:{label} ", attr)
        x += len(label) + 6
    add(win, 1, x, "  (Tab/1-4)", colors["muted"])

    hints = " q quit · ? help · r refresh · a magnet/URL · t file · c cycle · s search · e events"
    if app.loading:
        hints += " · loading..."
    if app.detail is not None:
        hints += " · 1 general · 2 trackers · 3 files · 4 peers · ↑↓ scroll · Enter/Esc back"
    elif app.tab == 1:
        hints += " · ↑↓/PgUp/PgDn select · Enter details · p pause/resume · b restart · d remove · X clean completed · k recheck · R reannounce · n no-rename · i/u pin/unpin · L limits"
    elif app.tab == 2:
        hints += " · ↑↓/PgUp/PgDn scroll · / filter · f follow · Home/End"
    elif app.tab == 3:
        hints += " · x empty trash"
    footer = hints + (f" | {app.message}" if app.message else "")
    footer_lines = wrapped_lines(footer, max(1, width - 2))
    top = 3
    footer_top = max(top + 1, height - max(1, len(footer_lines)))
    bottom = max(top + 1, footer_top - 1)
    if app.error:
        add_wrapped(win, top, 2, f"cannot reach daemon: {app.error}", colors["err"],
                    max(0, bottom - top))
        add_wrapped(win, top + 2, 2, "set REXTTO_URL / REXTTO_API_TOKEN", colors["muted"],
                    max(0, bottom - top - 2))
    elif app.detail is not None:
        draw_torrent_details(app, win, top, bottom, width, colors)
    elif app.tab == 0:
        draw_status(app, win, top, bottom, colors)
    elif app.tab == 1:
        draw_torrents(app, win, top, bottom, width, colors)
    elif app.tab == 2:
        draw_logs(app, win, top, bottom, width, colors)
    else:
        draw_health(app, win, top, bottom, width, colors)

    for offset, line in enumerate(footer_lines[-max(1, height - footer_top):]):
        add(win, footer_top + offset, 1, line,
            colors["ok"] if app.message and offset == len(footer_lines[-max(1, height - footer_top):]) - 1 else colors["muted"])
    if app.search_visible:
        draw_search(app, win, colors)
    elif app.events_visible:
        draw_events(app, win, colors)
    elif app.help_visible:
        draw_help(win, colors)
    win.refresh()


def draw_lines(lines, win, top, bottom, colors) -> None:
    for offset, line in enumerate(lines):
        y = top + offset
        if y >= bottom:
            break
        attr = colors["err"] if "ERROR" in line else (
            colors["warn"] if "WARN" in line else colors["normal"])
        add(win, y, 2, line, attr)


def draw_logs(app: App, win, top, bottom, width, colors) -> None:
    lines = app.filtered_logs()
    lines = [wrapped for line in lines for wrapped in wrapped_lines(line, max(1, width - 4))]
    visible = max(1, bottom - top - 1)
    max_scroll = max(0, len(lines) - visible)
    app.log_scroll = min(max(app.log_scroll, 0), max_scroll)
    end = len(lines) - app.log_scroll
    start = max(0, end - visible)
    label = f"Logs: {len(lines)} lines"
    if app.log_filter:
        label += f" · filter '{app.log_filter}'"
    if app.log_follow:
        label += " · LIVE SSE" if app.log_stream_connected else " · FOLLOW"
    add(win, top, 2, shorten(label, max(1, width - 4)), colors["header"] | curses.A_BOLD)
    draw_lines(lines[start:end], win, top + 1, bottom, colors)


def draw_help(win, colors) -> None:
    height, width = win.getmaxyx()
    lines = [
        "Rextto TUI - keyboard help",
        "",
        "Global:  1-4/Tab tabs · r refresh · ? close help · q quit",
        "         a add magnet/URL · t add .torrent · c cycle · s search · e events",
        "Torrents: ↑↓ or PgUp/PgDn select · Home/End · Enter details",
        "          p pause/resume · b restart · d remove · X clean completed",
        "          k recheck · R reannounce · n no-rename · i/u pin/unpin · L limits",
        "Details:  1 general · 2 trackers · 3 files · 4 peers · ↑↓ scroll",
        "Logs:     ↑↓ or PgUp/PgDn scroll · Home/End · / filter · f follow",
        "Health:   x empty trash (confirmation required)",
        "",
        "Press Esc, Enter or ? to close",
    ]
    box_width = min(width - 4, max(20, max(len(line) for line in lines) + 4))
    inner_width = max(1, box_width - 4)
    rendered = [wrapped for line in lines for wrapped in wrapped_lines(line, inner_width)]
    box_height = min(height - 2, max(3, len(rendered) + 2))
    left = max(1, (width - box_width) // 2)
    top = max(0, (height - box_height) // 2)
    attr = curses.A_REVERSE
    for row in range(box_height):
        add(win, top + row, left, " " * box_width, attr)
    add(win, top, left, "+" + "-" * max(0, box_width - 2) + "+", curses.A_BOLD)
    add(win, top + box_height - 1, left,
        "+" + "-" * max(0, box_width - 2) + "+", curses.A_BOLD)
    for offset, line in enumerate(rendered[: max(0, box_height - 2)], 1):
        add(win, top + offset, left + 2, line, attr)


def draw_status(app, win, top, bottom, colors) -> None:
    status = app.status
    torrents = status.get("torrent_stats", {})
    cycle = status.get("last_cycle", {})
    seen = status.get("seen", {})
    mode = "dry-run" if status.get("dry_run") else (
        "active" if status.get("active") else "stand-by")
    lines = [
        f"Mode: {mode}",
        f"Torrents: {text(torrents,'count')} ({text(torrents,'downloading')} downloading, "
        f"{text(torrents,'queued')} queued, {text(torrents,'seeding')} seeding)",
        f"Last cycle: scraped {text(cycle,'scraped')} | candidates {text(cycle,'candidates')} "
        f"| downloads {text(cycle,'downloads_started')} | gaps {text(cycle,'gaps_filled')} "
        f"| errors {text(cycle,'errors')}",
        f"Seen in feeds: groups {text(seen,'groups')} · movies {text(seen,'movies')} "
        f"· series {text(seen,'series')}",
    ]
    y = top
    for line in lines:
        if y >= bottom:
            break
        y += add_wrapped(win, y, 2, line, colors["normal"], bottom - y)


def draw_torrents(app, win, top, bottom, width, colors) -> None:
    selected_label = f"Torrents: {app.selected + 1}/{len(app.torrents)}  " if app.torrents else "Torrents: 0  "
    add(win, top, 2, selected_label, colors["header"] | curses.A_BOLD)
    if width < 80:
        add(win, top + 1, 2, "STATE   PROGRESS  NAME / HASH", colors["header"])
        visible = max(1, bottom - top - 2)
        blocks = []
        for torrent in app.torrents:
            progress = float(torrent.get("progress") or 0.0)
            first = f"{text(torrent, 'state', '-')} {progress:.1f}%  {text(torrent, 'name', '-')}"
            second = (f"{text(torrent, 'hash', '-')} · done {human_bytes(torrent.get('total_done') or 0)} · "
                      f"down {human_bytes(torrent.get('download_rate') or 0)}/s · "
                      f"up {human_bytes(torrent.get('upload_rate') or 0)}/s")
            block = wrapped_lines(first, max(1, width - 4))
            block.extend(wrapped_lines(second, max(1, width - 4)))
            blocks.append(block)
        start = min(app.selected, max(0, len(blocks) - 1))
        used = len(blocks[start]) if blocks else 0
        while start > 0 and used + len(blocks[start - 1]) <= visible:
            start -= 1
            used += len(blocks[start])
        y = top + 2
        for index in range(start, len(blocks)):
            block = blocks[index]
            if y >= bottom:
                break
            attr = colors["tab_active"] if index == app.selected else colors["normal"]
            for line_offset, line in enumerate(block[:bottom - y]):
                add(win, y + line_offset, 1,
                    ">" if index == app.selected and line_offset == 0 else " ", attr)
                add(win, y + line_offset, 2, line, attr)
            y += min(len(block), bottom - y)
        return
    add(win, top + 1, 2, f"{'HASH':<9} {'STATE':<12} {'PROG':>6} {'DONE':>10} "
                     f"{'DOWN':>10} {'UP':>10}  NAME", colors["header"])
    name_width = max(10, width - 64)
    visible = max(1, bottom - top - 2)
    start = max(0, min(app.selected - visible + 1,
                       max(0, len(app.torrents) - visible)))
    end = min(start + visible, len(app.torrents))
    for index in range(start, end):
        torrent = app.torrents[index]
        y = top + 2 + index - start
        if y >= bottom:
            break
        attr = colors["tab_active"] if index == app.selected else colors["normal"]
        progress = float(torrent.get("progress") or 0.0)
        line = (f"{shorten(text(torrent,'hash'),9):<9} {shorten(text(torrent,'state'),12):<12} "
                f"{progress:>5.1f}% {human_bytes(torrent.get('total_done') or 0):>10} "
                f"{human_bytes(torrent.get('download_rate') or 0) + '/s':>10} "
                f"{human_bytes(torrent.get('upload_rate') or 0) + '/s':>10}  "
                f"{shorten(text(torrent,'name'), name_width)}")
        add(win, y, 1, ">" if index == app.selected else " ", attr | curses.A_BOLD if index == app.selected else attr)
        add(win, y, 2, line, attr)


def draw_torrent_details(app: App, win, top, bottom, width, colors) -> None:
    detail = app.detail or {}
    torrent = detail.get("torrent", {})
    if not isinstance(torrent, dict):
        add(win, top, 2, "invalid torrent details", colors["err"])
        return

    if app.detail_view != "general":
        draw_detail_collection(app, win, top, bottom, width, colors)
        return

    ratio = torrent.get("seed_ratio")
    days = torrent.get("seed_days")
    if ratio == 0 or days == 0:
        seed_limit = "infinite"
    elif ratio is None and days is None:
        seed_limit = "-"
    else:
        seed_limit = f"ratio {text(torrent, 'seed_ratio', '-')} · {text(torrent, 'seed_days', '-')} days"

    metadata = "present" if torrent.get("has_metadata") else "pending"
    rows = [
        ("Name", text(torrent, "name", "-")),
        ("Hash", text(torrent, "hash", "-")),
        ("State", text(torrent, "state", "-")),
        ("Progress", f"{float(torrent.get('progress') or 0.0):.1f}%"),
        ("Size", human_bytes(torrent.get("total_size") or 0)),
        ("Downloaded", human_bytes(torrent.get("total_done") or 0)),
        ("All-time download", human_bytes(torrent.get("all_time_download") or 0)),
        ("All-time upload", human_bytes(torrent.get("all_time_upload") or 0)),
        ("Rates", f"{human_bytes(torrent.get('download_rate') or 0)}/s down · "
                  f"{human_bytes(torrent.get('upload_rate') or 0)}/s up"),
        ("Peers / seeds", f"{text(torrent, 'num_peers', '0')} / {text(torrent, 'num_seeds', '0')}"),
        ("Queue position", text(torrent, "queue_position", "-")),
        ("Seed limit", seed_limit),
        ("Metadata", metadata),
        ("Torrent version", text(torrent, "torrent_version", "-")),
        ("Auto-managed", "yes" if torrent.get("auto_managed") else "no"),
        ("No rename", "yes" if detail.get("no_rename") else "no"),
        ("Archived", "yes" if detail.get("archived") else "no"),
        ("Source", detail.get("source") or "-"),
        ("Reason", detail.get("reason") or "-"),
        ("Save path", text(torrent, "save_path", "-")),
        ("Magnet", detail.get("magnet") or "-"),
    ]
    y = top
    for label, value in rows:
        if y >= bottom:
            break
        y += add_wrapped(win, y, 2, f"{label:<18} {value}", colors["normal"], bottom - y)


def draw_detail_collection(app: App, win, top, bottom, width, colors) -> None:
    labels = {"trackers": "Trackers", "files": "Files", "peers": "Peers"}
    title = labels.get(app.detail_view, app.detail_view)
    add(win, top, 2, f"{title}: {len(app.detail_items)}", colors["header"] | curses.A_BOLD)
    rendered = []
    for item in app.detail_items:
        if not isinstance(item, dict):
            line = str(item)
        elif app.detail_view == "trackers":
            line = f"tier {text(item, 'tier', '-'):>3}  {text(item, 'url', '-')}"
        elif app.detail_view == "files":
            size = human_bytes(item.get("size") or 0)
            downloaded = human_bytes(item.get("downloaded") or 0)
            line = f"{downloaded:>10} / {size:<10}  {text(item, 'path', '-')}"
        else:
            line = (f"{text(item, 'address', '-'):>22}  {text(item, 'client', '-'):<24} "
                    f"down {human_bytes(item.get('download_rate') or 0)}/s  "
                    f"up {human_bytes(item.get('upload_rate') or 0)}/s  "
                    f"{'seed' if item.get('seed') else ''}")
        rendered.extend(wrapped_lines(line, max(1, width - 4)))
    visible = max(1, bottom - top - 1)
    start = min(app.detail_scroll, max(0, len(rendered) - visible))
    for offset, line in enumerate(rendered[start:start + visible]):
        add(win, top + 1 + offset, 2, line, colors["normal"])


def draw_search(app: App, win, colors) -> None:
    height, width = win.getmaxyx()
    results = app.search_results
    box_width = min(width - 4, max(50, min(width - 4, 100)))
    inner_width = max(1, box_width - 4)
    heading = wrapped_lines(
        f"Search '{app.search_query}' — {len(results)} results · Enter/a queue · Esc close",
        inner_width,
    )
    blocks = []
    for result in results:
        title = text(result, "title", "untitled")
        source = text(result, "source", "-")
        quality = result.get("quality", {}) if isinstance(result, dict) else {}
        quality_text = text(quality, "resolution") or text(quality, "source")
        blocks.append(wrapped_lines(f"{title} · {source} · {quality_text}", inner_width))
    box_height = min(height - 2, max(7, len(heading) + sum(map(len, blocks)) + 4))
    left = max(1, (width - box_width) // 2)
    top = max(0, (height - box_height) // 2)
    for row in range(box_height):
        add(win, top + row, left, " " * box_width, curses.A_REVERSE)
    add(win, top, left, "+" + "-" * max(0, box_width - 2) + "+", curses.A_BOLD)
    add(win, top + box_height - 1, left,
        "+" + "-" * max(0, box_width - 2) + "+", curses.A_BOLD)
    for offset, line in enumerate(heading[:max(0, box_height - 2)], 1):
        add(win, top + offset, left + 2, line, curses.A_BOLD | curses.A_REVERSE)
    start = min(app.search_selected, max(0, len(results) - 1))
    y = top + 1 + len(heading) + 1
    for index in range(start, len(blocks)):
        if y >= top + box_height - 1:
            break
        attr = curses.A_REVERSE | (curses.A_BOLD if index == app.search_selected else 0)
        for line in blocks[index]:
            if y >= top + box_height - 1:
                break
            add(win, y, left + 2, line, attr)
            y += 1


def draw_events(app: App, win, colors) -> None:
    height, width = win.getmaxyx()
    box_width = min(width - 4, max(50, min(width - 4, 100)))
    inner_width = max(1, box_width - 4)
    blocks = []
    for event in app.events:
        kind = text(event, "kind", "event")
        name = text(event, "name", text(event, "hash", "-"))
        save_path = text(event, "save_path", "")
        line = f"{kind:<22} {name}"
        if save_path:
            line += f" · {save_path}"
        blocks.append(wrapped_lines(line, inner_width))
    box_height = min(height - 2, max(7, sum(map(len, blocks)) + 4))
    left = max(1, (width - box_width) // 2)
    top = max(0, (height - box_height) // 2)
    for row in range(box_height):
        add(win, top + row, left, " " * box_width, curses.A_REVERSE)
    add(win, top, left, "+" + "-" * max(0, box_width - 2) + "+", curses.A_BOLD)
    add(win, top + box_height - 1, left,
        "+" + "-" * max(0, box_width - 2) + "+", curses.A_BOLD)
    heading = wrapped_lines("Recent torrent events · Esc close", inner_width)
    for offset, line in enumerate(heading, 1):
        add(win, top + offset, left + 2, line, curses.A_BOLD | curses.A_REVERSE)
    y = top + 1 + len(heading) + 1
    for block in blocks[-max(1, box_height - 3):]:
        for line in block:
            if y >= top + box_height - 1:
                break
            add(win, y, left + 2, line, curses.A_REVERSE)
            y += 1


def optional_number(value, suffix="", decimals=0) -> str:
    if value is None:
        return "-"
    try:
        number = float(value)
    except (TypeError, ValueError):
        return "-"
    if decimals == 0:
        return f"{number:.0f}{suffix}"
    return f"{number:.{decimals}f}{suffix}"


def human_duration(value) -> str:
    try:
        seconds = max(0, int(float(value or 0)))
    except (TypeError, ValueError):
        return "-"
    days, seconds = divmod(seconds, 86400)
    hours, seconds = divmod(seconds, 3600)
    minutes, _ = divmod(seconds, 60)
    if days:
        return f"{days}d {hours}h"
    if hours:
        return f"{hours}h {minutes}m"
    return f"{minutes}m"


def draw_health(app: App, win, top, bottom, width, colors) -> None:
    health = app.health if isinstance(app.health, dict) else {}
    status = text(health, "status", "offline")
    status_attr = colors["ok"] if status.lower() == "ok" else colors["warn"]
    health_label = f"Health: {status.upper()}"
    if app.health_error:
        health_label += " (refresh failed)"
    add(win, top, 2, shorten(health_label, max(1, width - 4)), status_attr | curses.A_BOLD)

    total_memory = health.get("memory_total_bytes") or 0
    available_memory = health.get("memory_available_bytes") or 0
    disk_total = health.get("disk_total_bytes") or 0
    disk_free = health.get("disk_free_bytes") or 0
    writable = "yes" if health.get("data_dir_writable") else "no"
    summary = [
        f"Process: PID {text(health, 'process_id', '-')} · uptime {human_duration(health.get('uptime_seconds'))} · CPU {optional_number(health.get('process_cpu_percent'), '%', 1)} · RAM {human_bytes(health.get('resident_bytes') or 0)}",
        f"System: CPU {optional_number(health.get('cpu_percent'), '%')} · load {optional_number(health.get('load_average'), '', 2)} · RAM free {human_bytes(available_memory)} / {human_bytes(total_memory)}",
        f"Disk: {human_bytes(disk_free)} free / {human_bytes(disk_total)} · data directory writable: {writable}",
        f"Trash: {text(health, 'trash_file_count', '0')} files · {human_bytes(health.get('trash_bytes') or 0)}",
    ]
    y = top + 1
    for line in summary:
        if y >= bottom:
            return
        y += add_wrapped(win, y, 2, line, colors["normal"], bottom - y)

    y += 1
    if y < bottom:
        add(win, y, 2, "Paths", colors["header"] | curses.A_BOLD)
        y += 1
    paths = health.get("paths", [])
    for path in paths if isinstance(paths, list) else []:
        if y >= bottom:
            return
        exists = bool(path.get("exists"))
        path_writable = bool(path.get("writable"))
        good = exists and path_writable
        state = "OK" if good else "FAIL"
        label = text(path, "label", "-")
        path_name = text(path, "path", "-")
        y += add_wrapped(win, y, 2, f"{state:<4} {label:<14} {path_name}",
                         colors["ok"] if good else colors["err"], bottom - y)

    disks = health.get("disks", [])
    if y < bottom:
        add(win, y, 2, "Disks", colors["header"] | curses.A_BOLD)
        y += 1
    for disk in disks if isinstance(disks, list) else []:
        if y >= bottom:
            return
        total = disk.get("total_bytes") or 0
        free = disk.get("free_bytes") or 0
        try:
            used = max(0.0, min(100.0, (1.0 - float(free) / float(total)) * 100.0)) if total else 0.0
        except (TypeError, ValueError):
            used = 0.0
        mount = text(disk, "mount", "-")
        line = f"{mount:<18} {human_bytes(free)} free / {human_bytes(total)} · {used:.0f}% used"
        y += add_wrapped(win, y, 2, line, colors["normal"], bottom - y)

    ramdisk = health.get("ramdisk")
    if isinstance(ramdisk, dict) and y < bottom:
        y += add_wrapped(
            win, y, 2,
            f"RAM disk: {text(ramdisk, 'path', '-')} · {human_bytes(ramdisk.get('free_bytes') or 0)} free / {human_bytes(ramdisk.get('total_bytes') or 0)}",
            colors["normal"], bottom - y)

    errors = health.get("last_errors", [])
    if isinstance(errors, list) and errors and y < bottom:
        add(win, y, 2, "Recent errors", colors["err"] | curses.A_BOLD)
        y += 1
        for error in errors[-2:]:
            if y >= bottom:
                break
            y += add_wrapped(win, y, 2, str(error), colors["err"], bottom - y)


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
        if app.search_visible:
            if key in (27, ord("q")):
                app.search_visible = False
            elif key in (curses.KEY_UP, curses.KEY_LEFT):
                app.search_selected = max(0, app.search_selected - 1)
            elif key in (curses.KEY_DOWN, curses.KEY_RIGHT):
                app.search_selected = min(app.search_selected + 1, max(0, len(app.search_results) - 1))
            elif key in (10, 13, curses.KEY_ENTER, ord("a")):
                app.add_search_result()
            continue
        if app.events_visible:
            if key in (27, ord("q")):
                app.events_visible = False
            elif key == ord("r"):
                app.load_events()
            continue
        if app.help_visible:
            if key in (27, 10, 13, curses.KEY_ENTER, ord("?"), ord("q")):
                app.help_visible = False
            continue
        if key == ord("?"):
            app.help_visible = True
            continue
        if key == ord("q"):
            break
        if app.detail is not None:
            if key in (10, 13, curses.KEY_ENTER, 27):
                app.close_details()
            elif key == ord("r"):
                app.refresh()
                app.open_selected_details()
            elif ord("1") <= key <= ord("4"):
                app.load_detail_view(("general", "trackers", "files", "peers")[key - ord("1")])
            elif key == curses.KEY_UP:
                app.move_detail_scroll(-1)
            elif key == curses.KEY_DOWN:
                app.move_detail_scroll(1)
            elif key == curses.KEY_PPAGE:
                app.move_detail_scroll(-max(1, stdscr.getmaxyx()[0] - 7))
            elif key == curses.KEY_NPAGE:
                app.move_detail_scroll(max(1, stdscr.getmaxyx()[0] - 7))
        elif key == 27:
            break
        elif key in (9, curses.KEY_RIGHT):
            app.tab = (app.tab + 1) % len(TABS)
        elif key in (getattr(curses, "KEY_BTAB", 353), curses.KEY_LEFT):
            app.tab = (app.tab - 1) % len(TABS)
        elif ord("1") <= key <= ord("4"):
            app.tab = key - ord("1")
        elif key == ord("r"):
            app.refresh()
            app.message = "refreshed"
        elif key == ord("c"):
            domain = prompt_input(stdscr, "Cycle [full/series/movies/comics] (full): ").lower() or "full"
            app.run_cycle(domain)
        elif key == ord("s"):
            app.search(prompt_input(stdscr, "Search: "))
        elif key == ord("e"):
            app.load_events()
        elif app.tab == 3 and key == ord("x"):
            if confirm(stdscr, "Delete all trash files now?"):
                app.clean_trash()
                app.refresh()
        elif key == ord("a"):
            app.add_magnet(prompt_input(stdscr, "Magnet/URL: "))
        elif key == ord("t"):
            app.add_torrent_file(prompt_input(stdscr, "File .torrent: "))
        elif app.tab == 1 and key == ord("p"):
            app.toggle_selected()
        elif app.tab == 1 and key == ord("b"):
            app.restart_selected()
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
        elif app.tab == 1 and key == ord("i"):
            app.pin_selected()
        elif app.tab == 1 and key == ord("u"):
            app.unpin()
        elif app.tab == 1 and key == ord("X"):
            if confirm(stdscr, "Remove completed torrents that reached seed limits?"):
                delete_files = confirm(stdscr, "Also delete downloaded files?")
                app.clean_completed(delete_files)
                app.refresh()
        elif app.tab == 1 and key == ord("L"):
            limits = prompt_input(stdscr, "Global DL UL limits KiB/s (0 0): ")
            values = limits.split()
            if len(values) == 2:
                app.set_global_speed_limits(values[0], values[1])
            elif limits:
                app.message = "enter two values: download upload"
        elif app.tab == 1 and key in (10, 13, curses.KEY_ENTER):
            app.open_selected_details()
        elif app.tab == 1:
            page = max(1, stdscr.getmaxyx()[0] - 7)
            if key == curses.KEY_DOWN:
                app.selected = min(app.selected + 1, max(0, len(app.torrents) - 1))
            elif key == curses.KEY_UP:
                app.selected = max(0, app.selected - 1)
            elif key == curses.KEY_NPAGE:
                app.selected = min(app.selected + page, max(0, len(app.torrents) - 1))
            elif key == curses.KEY_PPAGE:
                app.selected = max(0, app.selected - page)
            elif key == curses.KEY_HOME:
                app.selected = 0
            elif key == curses.KEY_END:
                app.selected = max(0, len(app.torrents) - 1)
        elif app.tab == 2:
            page = max(1, stdscr.getmaxyx()[0] - 5)
            max_scroll = max(0, len(app.filtered_logs()) - page)
            if key == curses.KEY_UP:
                app.log_scroll = min(app.log_scroll + 1, max_scroll)
                app.log_follow = False
            elif key == curses.KEY_DOWN:
                app.log_scroll = max(0, app.log_scroll - 1)
                app.log_follow = app.log_scroll == 0
            elif key == curses.KEY_NPAGE:
                app.log_scroll = min(app.log_scroll + page, max_scroll)
                app.log_follow = False
            elif key == curses.KEY_PPAGE:
                app.log_scroll = max(0, app.log_scroll - page)
                app.log_follow = app.log_scroll == 0
            elif key == curses.KEY_HOME:
                app.log_scroll = max_scroll
                app.log_follow = False
            elif key == curses.KEY_END:
                app.log_scroll = 0
                app.log_follow = True
            elif key == ord("/"):
                app.log_filter = prompt_input(stdscr, "Log filter (empty=all): ")
                app.log_scroll = 0
                app.log_follow = True
            elif key == ord("f"):
                app.log_follow = not app.log_follow
                if app.log_follow:
                    app.log_scroll = 0

        if time.time() - app.last_refresh >= REFRESH_SECS:
            app.refresh()

    app.close()


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

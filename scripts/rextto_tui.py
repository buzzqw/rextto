#!/usr/bin/env python3
"""Rextto TUI in Python — terminal client for a running Rextto daemon.

Uses only the standard library (curses + urllib), so there is nothing to
install:

    python3 scripts/rextto_tui.py
    REXTTO_URL=http://127.0.0.1:5000 REXTTO_API_TOKEN=... python3 scripts/rextto_tui.py

Tabs: Status · Torrents · Logs · Health.
Keys: 1-4/Tab switch · ↑↓ select · Enter details · ? help · r refresh · q quit.
Global: a add magnet · t add .torrent file · c run cycle.
Torrents tab: p pause/resume · d remove · k recheck · R reannounce · n no-rename.
Logs tab: / filter · f follow · ↑↓/PgUp/PgDn scroll · Home/End.
Health tab: x empty trash.

It only talks to the daemon's HTTP API and never touches the databases.
"""

from __future__ import annotations

import curses
import json
import os
import subprocess
import sys
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
    return status, torrents, logs, systemd_service_uptime()


def fetch_health():
    return api("/api/health") or {}


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
        self.service_uptime = None
        self.help_visible = False
        self.log_filter = ""
        self.log_scroll = 0
        self.log_follow = True
        self.loading = False
        self.health_error = ""

    def refresh(self) -> None:
        selected_hash = self.selected_hash()
        try:
            status, torrents, logs, service_uptime = fetch_snapshot()
            self.status = status
            self.torrents = torrents
            self.logs = logs
            self.service_uptime = service_uptime
            self.error = ""
            self.health = fetch_health()
            self.health_error = ""
        except RuntimeError as error:
            self.error = str(error)
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
        return

    def selected_hash(self):
        if not self.torrents:
            return None
        return text(self.torrents[self.selected], "hash") or None

    def filtered_logs(self):
        if not self.log_filter:
            return self.logs
        needle = self.log_filter.casefold()
        return [line for line in self.logs if needle in line.casefold()]

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
            }
            self.message = ""
        except RuntimeError as error:
            self.message = f"details failed: {error}"

    def close_details(self) -> None:
        self.detail = None

    # --- actions ---------------------------------------------------------

    def run_cycle(self) -> None:
        try:
            api("/api/run-now", "POST")
            self.message = "cycle requested"
        except RuntimeError as error:
            self.message = f"cycle failed: {error}"

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
    active = app.status.get("active") if isinstance(app.status, dict) else None
    activity_label = "ACTIVE" if active is True else (
        "PAUSED" if active is False else "LOADING")
    activity_attr = colors["ok"] if active is True else (
        colors["warn"] if active is False else colors["muted"])
    add(win, 0, 1, f"{name} v{version}  ", colors["header"] | curses.A_BOLD)
    add(win, 0, 14, activity_label, activity_attr)

    x = 2
    for index, label in enumerate(TABS):
        attr = colors["tab_active"] | curses.A_BOLD if index == app.tab else colors["muted"]
        add(win, 1, x, f" {index + 1}:{label} ", attr)
        x += len(label) + 6
    add(win, 1, x, "  (Tab/1-4)", colors["muted"])

    top = 3
    bottom = height - 2
    if app.error:
        add(win, top, 2, f"cannot reach daemon: {app.error}", colors["err"])
        add(win, top + 2, 2, "set REXTTO_URL / REXTTO_API_TOKEN", colors["muted"])
    elif app.detail is not None:
        draw_torrent_details(app, win, top, bottom, width, colors)
    elif app.tab == 0:
        draw_status(app, win, top, colors)
    elif app.tab == 1:
        draw_torrents(app, win, top, bottom, width, colors)
    elif app.tab == 2:
        draw_logs(app, win, top, bottom, width, colors)
    else:
        draw_health(app, win, top, bottom, width, colors)

    hints = " q quit · ? help · r refresh · a magnet · t file · c cycle"
    if app.loading:
        hints += " · loading..."
    if app.detail is not None:
        hints += " · Enter/Esc back"
    elif app.tab == 1:
        hints += " · ↑↓/PgUp/PgDn select · Enter details · p pause/resume · d remove · k recheck · R reannounce · n no-rename"
    elif app.tab == 2:
        hints += " · ↑↓/PgUp/PgDn scroll · / filter · f follow · Home/End"
    elif app.tab == 3:
        hints += " · x empty trash"
    add(win, height - 1, 1, hints, colors["muted"])
    if app.message:
        add(win, height - 1, min(width - 2, len(hints) + 3), f"| {app.message}", colors["ok"])
    if app.help_visible:
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
    visible = max(1, bottom - top - 1)
    max_scroll = max(0, len(lines) - visible)
    app.log_scroll = min(max(app.log_scroll, 0), max_scroll)
    end = len(lines) - app.log_scroll
    start = max(0, end - visible)
    label = f"Logs: {len(lines)} lines"
    if app.log_filter:
        label += f" · filter '{app.log_filter}'"
    if app.log_follow:
        label += " · FOLLOW"
    add(win, top, 2, shorten(label, max(1, width - 4)), colors["header"] | curses.A_BOLD)
    draw_lines(lines[start:end], win, top + 1, bottom, colors)


def draw_help(win, colors) -> None:
    height, width = win.getmaxyx()
    lines = [
        "Rextto TUI - keyboard help",
        "",
        "Global:  1-4/Tab tabs · r refresh · ? close help · q quit",
        "         a add magnet · t add .torrent · c run cycle",
        "Torrents: ↑↓ or PgUp/PgDn select · Home/End · Enter details",
        "          p pause/resume · d remove · k recheck · R reannounce",
        "          n no-rename",
        "Logs:     ↑↓ or PgUp/PgDn scroll · Home/End · / filter · f follow",
        "Health:   x empty trash (confirmation required)",
        "",
        "Press Esc, Enter or ? to close",
    ]
    box_width = min(width - 4, max(44, max(len(line) for line in lines) + 4))
    box_height = min(height - 2, len(lines) + 2)
    left = max(1, (width - box_width) // 2)
    top = max(0, (height - box_height) // 2)
    attr = curses.A_REVERSE
    for row in range(box_height):
        add(win, top + row, left, " " * box_width, attr)
    add(win, top, left, "+" + "-" * max(0, box_width - 2) + "+", curses.A_BOLD)
    add(win, top + box_height - 1, left,
        "+" + "-" * max(0, box_width - 2) + "+", curses.A_BOLD)
    for offset, line in enumerate(lines[: max(0, box_height - 2)], 1):
        add(win, top + offset, left + 2, shorten(line, max(1, box_width - 4)), attr)


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
    selected_label = f"Torrents: {app.selected + 1}/{len(app.torrents)}  " if app.torrents else "Torrents: 0  "
    add(win, top, 2, selected_label, colors["header"] | curses.A_BOLD)
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
        ("Save path", text(torrent, "save_path", "-")),
        ("Magnet", detail.get("magnet") or "-"),
    ]
    value_width = max(1, width - 23)
    for offset, (label, value) in enumerate(rows):
        y = top + offset
        if y >= bottom:
            break
        add(win, y, 2, f"{label:<18} {shorten(str(value), value_width)}", colors["normal"])


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


def systemd_service_uptime(service="rextto.service"):
    """Returns the local systemd service uptime in seconds, if available."""
    try:
        result = subprocess.run(
            ["systemctl", "show", service, "-p", "ActiveEnterTimestampMonotonic", "--value"],
            capture_output=True,
            text=True,
            timeout=2,
            check=True,
        )
        started_us = int(result.stdout.strip())
        if started_us <= 0:
            return None
        # systemd's monotonic timestamp and time.monotonic() share the same
        # CLOCK_MONOTONIC origin on Linux.
        return max(0.0, time.monotonic() - started_us / 1_000_000.0)
    except (OSError, ValueError, subprocess.SubprocessError):
        return None


def draw_health(app: App, win, top, bottom, width, colors) -> None:
    health = app.health if isinstance(app.health, dict) else {}
    status = text(health, "status", "offline")
    status_attr = colors["ok"] if status.lower() == "ok" else colors["warn"]
    add(win, top, 2, f"Health: {status.upper()}", status_attr | curses.A_BOLD)

    total_memory = health.get("memory_total_bytes") or 0
    available_memory = health.get("memory_available_bytes") or 0
    disk_total = health.get("disk_total_bytes") or 0
    disk_free = health.get("disk_free_bytes") or 0
    writable = "yes" if health.get("data_dir_writable") else "no"
    summary = [
        f"Process: PID {text(health, 'process_id', '-')} · uptime {human_duration(app.service_uptime)} · CPU {optional_number(health.get('process_cpu_percent'), '%', 1)} · RAM {human_bytes(health.get('resident_bytes') or 0)}",
        f"System: CPU {optional_number(health.get('cpu_percent'), '%')} · load {optional_number(health.get('load_average'), '', 2)} · RAM free {human_bytes(available_memory)} / {human_bytes(total_memory)}",
        f"Disk: {human_bytes(disk_free)} free / {human_bytes(disk_total)} · data directory writable: {writable}",
        f"Trash: {text(health, 'trash_file_count', '0')} files · {human_bytes(health.get('trash_bytes') or 0)}",
    ]
    for offset, line in enumerate(summary, 1):
        if top + offset >= bottom:
            return
        add(win, top + offset, 2, shorten(line, max(1, width - 4)), colors["normal"])

    y = top + len(summary) + 1
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
        path_name = shorten(text(path, "path", "-"), max(1, width - 25))
        add(win, y, 2, f"{state:<4} {label:<14} {path_name}",
            colors["ok"] if good else colors["err"])
        y += 1

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
        mount = shorten(text(disk, "mount", "-"), max(1, width - 48))
        line = f"{mount:<18} {human_bytes(free)} free / {human_bytes(total)} · {used:.0f}% used"
        add(win, y, 2, shorten(line, max(1, width - 4)), colors["normal"])
        y += 1

    ramdisk = health.get("ramdisk")
    if isinstance(ramdisk, dict) and y < bottom:
        add(win, y, 2,
            shorten(f"RAM disk: {text(ramdisk, 'path', '-')} · {human_bytes(ramdisk.get('free_bytes') or 0)} free / {human_bytes(ramdisk.get('total_bytes') or 0)}",
                    max(1, width - 4)), colors["normal"])
        y += 1

    errors = health.get("last_errors", [])
    if isinstance(errors, list) and errors and y < bottom:
        add(win, y, 2, "Recent errors", colors["err"] | curses.A_BOLD)
        y += 1
        for error in errors[-2:]:
            if y >= bottom:
                break
            add(win, y, 2, shorten(str(error), max(1, width - 4)), colors["err"])
            y += 1


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
            app.run_cycle()
        elif app.tab == 3 and key == ord("x"):
            if confirm(stdscr, "Delete all trash files now?"):
                app.clean_trash()
                app.refresh()
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

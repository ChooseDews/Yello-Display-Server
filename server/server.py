#!/usr/bin/env python3
"""Web designer, multi-device renderer, and ESP32 display gateway."""

from __future__ import annotations

import asyncio
import io
import json
import os
import re
import secrets
import tempfile
import time
from collections import deque
from dataclasses import dataclass, field
from datetime import datetime, timezone
from pathlib import Path
from urllib.parse import parse_qs, urlsplit

import numpy as np
import websockets
from aiohttp import ClientSession, ClientTimeout, web
from PIL import Image

import renderer
from frame_diff import dirty_regions
from home_assistant import HomeAssistantClient
from layout_model import (
    LayoutValidationError,
    default_layout,
    layout_dimensions,
    new_element,
    validate_layout,
)
from protocol import (
    DeviceCommand, DeviceCommandType, DeviceConfig, DeviceStatus, MsgType,
    Orientation, PROTO_MAGIC, PROTO_VERSION, TouchEvent, TouchType, ZoneUpdate,
)
from safe_http import fetch_bytes
from script_runtime import SCRIPT_EXAMPLES


def _configuration_value(*names: str) -> str:
    for name in names:
        if os.environ.get(name):
            return os.environ[name]
    for env_path in (Path(__file__).parent / ".env", Path(__file__).parent.parent / ".env"):
        try:
            lines = env_path.read_text(encoding="utf-8").splitlines()
        except OSError:
            continue
        values = {}
        for line in lines:
            line = line.strip()
            if not line or line.startswith("#") or "=" not in line:
                continue
            key, value = line.split("=", 1)
            values[key.strip()] = value.strip().strip("'\"")
        for name in names:
            if values.get(name):
                return values[name]
    return ""


WEB_HOST = os.environ.get("YELLO_WEB_HOST", "127.0.0.1")
WEB_PORT = int(os.environ.get("YELLO_WEB_PORT", "8080"))
DEVICE_HOST = os.environ.get("YELLO_DEVICE_HOST", "0.0.0.0")
DEVICE_WS_PORT = int(os.environ.get("YELLO_DEVICE_WS_PORT", "8765"))
ALLOWED_HTTP_HOSTS = {host.strip() for host in os.environ.get("YELLO_ALLOWED_HTTP_HOSTS", "").split(",") if host.strip()}

MAX_ZONE_AREA_PX = 480
ZONE_HEADER_BYTES = 16
STREAM_STATS_WINDOW_S = 10.0
ZONE_PACING_S = 0.002
MAX_EXTERNAL_BYTES = 8192
MAX_IMAGE_BYTES = 2 * 1024 * 1024
MAX_IMAGE_PIXELS = 4_000_000
FULL_RESYNC_INTERVAL_S = 300
EXTERNAL_FETCH_TIMEOUT_S = 10
DOT_LIFETIME_S = 4.0
BUTTON_FLASH_S = 0.25
DEFAULT_DESIGN_ID = "default"
DEVICE_ID_RE = re.compile(r"^[a-zA-Z0-9._:-]{1,64}$")
HA_URL = _configuration_value("YELLO_HA_URL", "HA_URL")
HA_TOKEN = _configuration_value("YELLO_HA_TOKEN", "HA_TOKEN")

ROOT_DIR = Path(__file__).parent
DATA_DIR = Path(os.environ.get("YELLO_DATA_DIR", str(ROOT_DIR))).expanduser()
LAYOUT_PATH = DATA_DIR / "layout.json"  # legacy migration source
STUDIO_PATH = DATA_DIR / "studio.json"
STATIC_DIR = ROOT_DIR / "static"
SECRETS_PATH = DATA_DIR / "secrets.json"


@dataclass
class DeviceSession:
    device_id: str
    ws: object
    remote: str
    last_frame: np.ndarray | None = None
    last_full_push: float = 0.0
    push_lock: asyncio.Lock = field(default_factory=asyncio.Lock)
    pressed: set[str] = field(default_factory=set)
    dots: list[tuple[int, int, float]] = field(default_factory=list)
    connected_at: float = field(default_factory=time.monotonic)
    connected_since: str = field(default_factory=lambda: datetime.now(timezone.utc).isoformat())
    stream_samples: deque = field(default_factory=deque)
    total_pixels: int = 0
    total_wire_bytes: int = 0
    total_zone_messages: int = 0
    frames_sent: int = 0
    last_push_ms: float = 0.0
    last_frame_at: str = ""
    orientation: str = ""
    last_ping_at: str = ""
    wifi_rssi_dbm: int | None = None
    uptime_ms: int = 0
    active_design_id: str = ""


@dataclass
class LivePreviewState:
    pressed: set[str] = field(default_factory=set)
    dots: list[tuple[int, int, float]] = field(default_factory=list)


_studio: dict = {}
_devices: dict[str, DeviceSession] = {}
_http_session: ClientSession | None = None
_external_values: dict[tuple[str, str], str] = {}
_external_last_fetch: dict[tuple[str, str], float] = {}
_images: dict[str, Image.Image] = {}
_image_fetches_started: set[str] = set()
_ha_client = HomeAssistantClient(HA_URL, HA_TOKEN)
_ha_values: dict[str, dict] = {}
_ha_last_fetch: dict[str, float] = {}
_live_states: dict[str, LivePreviewState] = {}


def load_dashboard_settings() -> None:
    """Apply dashboard-managed secrets over environment/.env defaults."""
    global _ha_client
    try:
        raw = json.loads(SECRETS_PATH.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return
    home_assistant = raw.get("homeAssistant", {}) if isinstance(raw, dict) else {}
    if not isinstance(home_assistant, dict):
        return
    url = str(home_assistant.get("url") or HA_URL)
    token = str(home_assistant.get("token") or HA_TOKEN)
    _ha_client = HomeAssistantClient(url, token)


def save_dashboard_settings(url: str, token: str) -> None:
    value = {"schemaVersion": 1, "homeAssistant": {"url": url, "token": token}}
    _atomic_json_write(SECRETS_PATH, value)
    os.chmod(SECRETS_PATH, 0o600)


def _new_studio(layout: dict | None = None) -> dict:
    initial = validate_layout(layout or default_layout())
    return {
        "schemaVersion": 1,
        "designs": {
            DEFAULT_DESIGN_ID: {
                "id": DEFAULT_DESIGN_ID,
                "name": initial.get("name", "Default design"),
                "revision": 0,
                "layout": initial,
            }
        },
        "screens": {},
    }


def _atomic_json_write(path: Path, value: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, temporary_path = tempfile.mkstemp(prefix=f"{path.stem}-", suffix=".json.tmp", dir=path.parent)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as output:
            json.dump(value, output, indent=2)
            output.write("\n")
            output.flush()
            os.fsync(output.fileno())
        os.replace(temporary_path, path)
    finally:
        try:
            os.unlink(temporary_path)
        except FileNotFoundError:
            pass


def save_layout_atomic(layout: dict) -> None:
    """Legacy helper retained for callers that explicitly persist one layout."""
    _atomic_json_write(LAYOUT_PATH, layout)


def save_studio_atomic() -> None:
    _atomic_json_write(STUDIO_PATH, _studio)


def load_studio() -> None:
    global _studio
    if STUDIO_PATH.exists():
        try:
            raw = json.loads(STUDIO_PATH.read_text(encoding="utf-8"))
            if raw.get("schemaVersion") != 1 or not isinstance(raw.get("designs"), dict) or not raw["designs"]:
                raise ValueError("unsupported studio schema")
            designs = {}
            for design_id, record in raw["designs"].items():
                if not DEVICE_ID_RE.fullmatch(design_id) or not isinstance(record, dict):
                    raise ValueError(f"invalid design id: {design_id}")
                designs[design_id] = {
                    "id": design_id,
                    "name": str(record.get("name") or design_id)[:100],
                    "revision": max(0, int(record.get("revision", 0))),
                    "layout": validate_layout(record.get("layout", {})),
                }
            screens = {}
            for screen_id, record in raw.get("screens", {}).items():
                if not DEVICE_ID_RE.fullmatch(screen_id) or not isinstance(record, dict):
                    continue
                design_id = record.get("designId", DEFAULT_DESIGN_ID)
                if design_id not in designs:
                    design_id = next(iter(designs))
                screens[screen_id] = {
                    "id": screen_id,
                    "name": str(record.get("name") or f"Yello {screen_id[-6:]}")[:100],
                    "designId": design_id,
                    "lastSeen": str(record.get("lastSeen") or ""),
                }
            _studio = {"schemaVersion": 1, "designs": designs, "screens": screens}
            return
        except (OSError, json.JSONDecodeError, ValueError, TypeError, LayoutValidationError) as exc:
            print(f"studio load failed; migrating legacy layout: {exc}")

    try:
        legacy = validate_layout(json.loads(LAYOUT_PATH.read_text(encoding="utf-8")))
    except (OSError, json.JSONDecodeError, LayoutValidationError):
        legacy = default_layout()
    _studio = _new_studio(legacy)
    save_studio_atomic()


def _design(design_id: str) -> dict:
    try:
        return _studio["designs"][design_id]
    except KeyError as exc:
        raise web.HTTPNotFound(text=json.dumps({"ok": False, "error": "design not found"}), content_type="application/json") from exc


def _screen(screen_id: str) -> dict:
    try:
        return _studio["screens"][screen_id]
    except KeyError as exc:
        raise web.HTTPNotFound(text=json.dumps({"ok": False, "error": "screen not found"}), content_type="application/json") from exc


def _design_id_for_session(session: DeviceSession) -> str:
    if session.active_design_id in _studio["designs"]:
        return session.active_design_id
    return _studio["screens"][session.device_id]["designId"]


def _layout_context(
    design_id: str,
    session: DeviceSession | None = None,
    live_state: LivePreviewState | None = None,
) -> dict:
    external = {
        source_id: value
        for (current_design_id, source_id), value in _external_values.items()
        if current_design_id == design_id
    }
    return {
        "external": external,
        "images": _images,
        "pressed": session.pressed if session else live_state.pressed if live_state else set(),
        "dots": session.dots if session else live_state.dots if live_state else [],
        "homeAssistant": _ha_values,
    }


def _to_rgb565_be(region: np.ndarray) -> bytes:
    red = region[..., 0].astype(np.uint16)
    green = region[..., 1].astype(np.uint16)
    blue = region[..., 2].astype(np.uint16)
    value = ((red & 0xF8) << 8) | ((green & 0xFC) << 3) | (blue >> 3)
    return value.astype(">u2").tobytes()


async def send_region(ws, frame: np.ndarray, x: int, y: int, width: int, height: int) -> None:
    rows_per_chunk = max(1, MAX_ZONE_AREA_PX // width)
    current_y = y
    while current_y < y + height:
        chunk_height = min(rows_per_chunk, y + height - current_y)
        pixels = _to_rgb565_be(frame[current_y:current_y + chunk_height, x:x + width])
        await ws.send(ZoneUpdate(x=x, y=current_y, w=width, h=chunk_height, pixels=pixels).pack())
        current_y += chunk_height
        # A small cadence is required in addition to socket back-pressure: the
        # ESP32 TCP receive window can fill before its WebSocket event loop
        # drains queued LCD work. This is 5x faster than the original 10 ms
        # delay while avoiding an indefinitely buffered full-frame burst.
        await asyncio.sleep(ZONE_PACING_S)


def _zone_count(width: int, height: int) -> int:
    rows_per_chunk = max(1, MAX_ZONE_AREA_PX // width)
    return (height + rows_per_chunk - 1) // rows_per_chunk


def _record_stream_sample(session: DeviceSession, pixels: int, wire_bytes: int, zones: int, push_ms: float) -> None:
    now = time.monotonic()
    session.stream_samples.append((now, pixels, wire_bytes, zones))
    while session.stream_samples and now - session.stream_samples[0][0] > STREAM_STATS_WINDOW_S:
        session.stream_samples.popleft()
    session.total_pixels += pixels
    session.total_wire_bytes += wire_bytes
    session.total_zone_messages += zones
    session.frames_sent += 1
    session.last_push_ms = push_ms
    session.last_frame_at = datetime.now(timezone.utc).isoformat()


def _stream_stats(session: DeviceSession) -> dict:
    now = time.monotonic()
    while session.stream_samples and now - session.stream_samples[0][0] > STREAM_STATS_WINDOW_S:
        session.stream_samples.popleft()
    duration = max(1.0, min(STREAM_STATS_WINDOW_S, now - session.connected_at))
    pixels = sum(sample[1] for sample in session.stream_samples)
    wire_bytes = sum(sample[2] for sample in session.stream_samples)
    zones = sum(sample[3] for sample in session.stream_samples)
    return {
        "framesPerSecond": round(len(session.stream_samples) / duration, 2),
        "pixelsPerSecond": round(pixels / duration),
        "kilobitsPerSecond": round(wire_bytes * 8 / duration / 1000, 1),
        "zoneMessagesPerSecond": round(zones / duration, 1),
        "lastPushMs": round(session.last_push_ms, 1),
        "totalPixels": session.total_pixels,
        "totalWireBytes": session.total_wire_bytes,
        "connectedSince": session.connected_since,
        "lastFrameAt": session.last_frame_at,
    }


async def push_frame(session: DeviceSession, *, full: bool = False) -> None:
    async with session.push_lock:
        push_started = time.perf_counter()
        design_id = _design_id_for_session(session)
        layout = _design(design_id)["layout"]
        orientation = layout.get("orientation", "portrait")
        width, height = layout_dimensions(layout)
        if session.orientation != orientation:
            config = DeviceConfig(
                orientation=Orientation.LANDSCAPE if orientation == "landscape" else Orientation.PORTRAIT,
                width=width,
                height=height,
            )
            await session.ws.send(config.pack())
            session.orientation = orientation
            session.last_frame = None
            full = True
        frame = np.asarray(renderer.render(layout, _layout_context(design_id, session)))
        should_send_full = (
            full or session.last_frame is None or
            time.monotonic() - session.last_full_push >= FULL_RESYNC_INTERVAL_S
        )
        regions = [(0, 0, width, height)] if should_send_full else dirty_regions(
            frame, session.last_frame, max_area=MAX_ZONE_AREA_PX
        )
        pixels_sent = 0
        zones_sent = 0
        for x, y, width, height in regions:
            await send_region(session.ws, frame, x, y, width, height)
            pixels_sent += width * height
            zones_sent += _zone_count(width, height)
        session.last_frame = frame
        if should_send_full:
            session.last_full_push = time.monotonic()
        if regions:
            wire_bytes = pixels_sent * 2 + zones_sent * ZONE_HEADER_BYTES
            _record_stream_sample(
                session, pixels_sent, wire_bytes, zones_sent,
                (time.perf_counter() - push_started) * 1000,
            )


async def push_design(design_id: str, *, full: bool = False) -> None:
    sessions = [session for session in _devices.values() if _design_id_for_session(session) == design_id]
    results = await asyncio.gather(*(push_frame(session, full=full) for session in sessions), return_exceptions=True)
    for result in results:
        if isinstance(result, Exception) and not isinstance(result, websockets.exceptions.ConnectionClosed):
            print(f"device push failed: {result}")


def _wanted_image_sources(layout: dict) -> set[str]:
    return {
        element["props"]["src"]
        for element in layout.get("elements", [])
        if element.get("type") == "image" and element.get("props", {}).get("src")
    }


def _designs_using_image(source: str) -> set[str]:
    return {
        design_id for design_id, record in _studio["designs"].items()
        if source in _wanted_image_sources(record["layout"])
    }


async def fetch_image(source: str) -> None:
    _image_fetches_started.add(source)
    try:
        data, _ = await fetch_bytes(
            _http_session, source, max_bytes=MAX_IMAGE_BYTES, allowed_hosts=ALLOWED_HTTP_HOSTS,
        )
        image = Image.open(io.BytesIO(data))
        if image.width * image.height > MAX_IMAGE_PIXELS:
            raise ValueError("image dimensions are too large")
        image.load()
        _images[source] = image.convert("RGB")
        await asyncio.gather(*(push_design(design_id) for design_id in _designs_using_image(source)))
    except Exception as exc:
        print(f"image fetch failed for {source}: {exc}")
        _image_fetches_started.discard(source)


def ensure_images(layout: dict, *, prune: bool = False) -> None:
    wanted = _wanted_image_sources(layout)
    if prune:
        all_wanted = set().union(*(_wanted_image_sources(record["layout"]) for record in _studio["designs"].values()))
        for source in set(_images) - all_wanted:
            _images.pop(source, None)
            _image_fetches_started.discard(source)
    for source in wanted:
        if source not in _image_fetches_started:
            asyncio.create_task(fetch_image(source))


async def external_refresh_loop() -> None:
    while True:
        active_keys = set()
        for design_id, record in list(_studio["designs"].items()):
            for source in record["layout"].get("dataSources", []):
                key = (design_id, source["id"])
                active_keys.add(key)
                if time.monotonic() - _external_last_fetch.get(key, 0) < source["interval"]:
                    continue
                _external_last_fetch[key] = time.monotonic()
                try:
                    data, _ = await fetch_bytes(
                        _http_session, source["url"], max_bytes=MAX_EXTERNAL_BYTES,
                        allowed_hosts=ALLOWED_HTTP_HOSTS,
                    )
                    text = data.decode("utf-8", errors="replace").strip()
                    value = text[:1000] if text else "(empty)"
                except Exception as exc:
                    value = f"(error: {type(exc).__name__})"
                if _external_values.get(key) != value:
                    _external_values[key] = value
                    await push_design(design_id)
        for key in set(_external_values) - active_keys:
            _external_values.pop(key, None)
            _external_last_fetch.pop(key, None)
        await asyncio.sleep(1)


def _home_assistant_entity_specs() -> dict[str, dict]:
    specs: dict[str, dict] = {}
    for design_id, record in _studio["designs"].items():
        for element in record["layout"].get("elements", []):
            if element.get("type") not in {"ha-state", "ha-toggle", "script"} or not element.get("visible", True):
                continue
            props = element.get("props", {})
            entity_ids = [props.get("entityId", "")]
            if element.get("type") == "script":
                entity_ids = re.findall(r"ha\(\s*['\"]([a-z0-9_]+\.[a-z0-9_]+)['\"]", props.get("code", ""))
            for entity_id in entity_ids:
                if not entity_id:
                    continue
                spec = specs.setdefault(entity_id, {"interval": 3600, "designs": set()})
                spec["interval"] = min(spec["interval"], max(1, int(props.get("refreshInterval", 5))))
                spec["designs"].add(design_id)
    return specs


async def home_assistant_refresh_loop() -> None:
    while True:
        specs = _home_assistant_entity_specs()
        changed_designs: set[str] = set()
        if _ha_client.configured:
            for entity_id, spec in specs.items():
                if time.monotonic() - _ha_last_fetch.get(entity_id, 0) < spec["interval"]:
                    continue
                _ha_last_fetch[entity_id] = time.monotonic()
                try:
                    value = await _ha_client.get_state(_http_session, entity_id)
                except Exception as exc:
                    value = {"entity_id": entity_id, "state": "unavailable", "attributes": {}, "error": type(exc).__name__}
                if _ha_values.get(entity_id) != value:
                    _ha_values[entity_id] = value
                    changed_designs.update(spec["designs"])
        for entity_id in set(_ha_values) - set(specs):
            _ha_values.pop(entity_id, None)
            _ha_last_fetch.pop(entity_id, None)
        await asyncio.gather(*(push_design(design_id) for design_id in changed_designs))
        await asyncio.sleep(0.25)


async def add_dot(session: DeviceSession, x: int, y: int) -> None:
    dot = (x, y, time.monotonic() + DOT_LIFETIME_S)
    session.dots.append(dot)
    await push_frame(session)
    await asyncio.sleep(DOT_LIFETIME_S)
    if dot in session.dots:
        session.dots.remove(dot)
    await push_frame(session)


async def trigger_button_action(element: dict, layout: dict, design_id: str) -> None:
    if element.get("type") == "ha-toggle":
        entity_id = element.get("props", {}).get("entityId", "")
        try:
            await _ha_client.toggle(_http_session, entity_id)
            await asyncio.sleep(0.1)
            _ha_values[entity_id] = await _ha_client.get_state(_http_session, entity_id)
            _ha_last_fetch[entity_id] = time.monotonic()
            print(f"Home Assistant toggle completed: {entity_id}")
        except Exception as exc:
            _ha_values[entity_id] = {"entity_id": entity_id, "state": "unavailable", "attributes": {}, "error": type(exc).__name__}
            print(f"Home Assistant toggle failed for {entity_id}: {type(exc).__name__}")
        await push_design(design_id)
    else:
        action = renderer.resolve_button_action(layout, element)
        if action:
            try:
                _, final_url = await fetch_bytes(
                    _http_session, action["url"], max_bytes=MAX_EXTERNAL_BYTES,
                    allowed_hosts=ALLOWED_HTTP_HOSTS, method=action["method"], body=action.get("body", ""),
                )
                print(f"button action {action['id']} completed: {final_url}")
            except Exception as exc:
                print(f"button action {action['id']} failed: {exc}")


async def press_button(session: DeviceSession, element: dict, layout: dict) -> None:
    session.pressed.add(element["id"])
    await push_frame(session)
    if element.get("type") == "design-link":
        await asyncio.sleep(BUTTON_FLASH_S)
        session.pressed.discard(element["id"])
        target_design_id = element.get("props", {}).get("targetDesignId", "")
        if target_design_id in _studio["designs"]:
            session.active_design_id = target_design_id
            session.last_frame = None
            await push_frame(session, full=True)
        else:
            await push_frame(session)
        return
    await trigger_button_action(element, layout, _design_id_for_session(session))
    await asyncio.sleep(BUTTON_FLASH_S)
    session.pressed.discard(element["id"])
    await push_frame(session)


async def _live_press(design_id: str, state: LivePreviewState, element: dict, layout: dict) -> None:
    state.pressed.add(element["id"])
    try:
        await trigger_button_action(element, layout, design_id)
        await asyncio.sleep(BUTTON_FLASH_S)
    finally:
        state.pressed.discard(element["id"])


async def _live_dot(state: LivePreviewState, x: int, y: int) -> None:
    dot = (x, y, time.monotonic() + DOT_LIFETIME_S)
    state.dots.append(dot)
    await asyncio.sleep(DOT_LIFETIME_S)
    if dot in state.dots:
        state.dots.remove(dot)


def _device_id_from_ws(ws) -> str:
    request = getattr(ws, "request", None)
    path = getattr(request, "path", "") or ""
    candidate = parse_qs(urlsplit(path).query).get("device_id", [""])[0]
    if DEVICE_ID_RE.fullmatch(candidate):
        return candidate.lower()
    remote = getattr(ws, "remote_address", None)
    address = str(remote[0] if isinstance(remote, tuple) and remote else remote or "unknown")
    safe = re.sub(r"[^a-zA-Z0-9._:-]", "-", address)[:48]
    return f"legacy-{safe}"


def _register_screen(device_id: str, remote_address: str = "") -> dict:
    legacy_id = f"legacy-{remote_address}" if remote_address else ""
    if device_id not in _studio["screens"] and legacy_id in _studio["screens"] and legacy_id != device_id:
        legacy = _studio["screens"].pop(legacy_id)
        legacy["id"] = device_id
        if legacy["name"].startswith("Yello "):
            legacy["name"] = f"Yello {device_id[-6:]}"
        _studio["screens"][device_id] = legacy
    screen = _studio["screens"].get(device_id)
    if screen is None:
        screen = {
            "id": device_id,
            "name": f"Yello {device_id[-6:]}",
            "designId": DEFAULT_DESIGN_ID if DEFAULT_DESIGN_ID in _studio["designs"] else next(iter(_studio["designs"])),
            "lastSeen": "",
        }
        _studio["screens"][device_id] = screen
    screen["lastSeen"] = datetime.now(timezone.utc).isoformat()
    save_studio_atomic()
    return screen


async def device_render_loop(session: DeviceSession) -> None:
    while True:
        await push_frame(session)
        layout = _design(_design_id_for_session(session))["layout"]
        delay = 1.0 - (time.time() % 1.0) if renderer.has_clock(layout) else 30.0
        await asyncio.sleep(delay)


async def handle_device(ws) -> None:
    device_id = _device_id_from_ws(ws)
    remote_address = getattr(ws, "remote_address", None)
    remote_ip = str(remote_address[0] if isinstance(remote_address, tuple) and remote_address else "")
    _register_screen(device_id, remote_ip)
    previous = _devices.get(device_id)
    if previous is not None:
        await previous.ws.close()
    remote = str(getattr(ws, "remote_address", ""))
    session = DeviceSession(device_id=device_id, ws=ws, remote=remote)
    _devices[device_id] = session
    print(f"screen connected: {device_id} from {remote}")
    render_task = asyncio.create_task(device_render_loop(session))
    try:
        async for message in ws:
            if not isinstance(message, bytes):
                continue
            if len(message) >= 3 and message[0] == PROTO_MAGIC and message[1] == PROTO_VERSION and message[2] == MsgType.DEVICE_STATUS:
                try:
                    status = DeviceStatus.unpack(message)
                except ValueError as exc:
                    print(f"bad device status from {device_id}: {exc}")
                    continue
                session.last_ping_at = datetime.now(timezone.utc).isoformat()
                session.wifi_rssi_dbm = status.wifi_rssi_dbm
                session.uptime_ms = status.uptime_ms
                continue
            try:
                event = TouchEvent.unpack(message)
            except ValueError as exc:
                print(f"bad touch event from {device_id}: {exc}")
                continue
            layout = _design(_design_id_for_session(session))["layout"]
            width, height = layout_dimensions(layout)
            if event.x >= width or event.y >= height:
                continue
            if event.touch_type == TouchType.DOWN:
                button = renderer.hit_test_buttons(layout, event.x, event.y)
                asyncio.create_task(press_button(session, button, layout) if button else add_dot(session, event.x, event.y))
    except websockets.exceptions.ConnectionClosedError as exc:
        print(f"screen {device_id} closed uncleanly: {exc}")
    finally:
        render_task.cancel()
        await asyncio.gather(render_task, return_exceptions=True)
        if _devices.get(device_id) is session:
            _devices.pop(device_id, None)
        print(f"screen disconnected: {device_id}")


async def _validated_request_layout(request: web.Request) -> dict:
    try:
        raw = await request.json()
    except (json.JSONDecodeError, ValueError):
        raise web.HTTPBadRequest(
            text=json.dumps({"ok": False, "errors": [{"path": "$", "message": "body must be JSON"}]}),
            content_type="application/json",
        )
    try:
        return validate_layout(raw)
    except LayoutValidationError as exc:
        raise web.HTTPUnprocessableEntity(
            text=json.dumps({"ok": False, "errors": exc.errors}), content_type="application/json",
        )


def _validate_design_link_targets(layout: dict) -> None:
    errors = []
    for index, element in enumerate(layout.get("elements", [])):
        if element.get("type") != "design-link":
            continue
        target = element.get("props", {}).get("targetDesignId", "")
        if target not in _studio.get("designs", {}):
            errors.append({
                "path": f"elements[{index}].props.targetDesignId",
                "message": "references an unknown screen design",
            })
    if errors:
        raise web.HTTPUnprocessableEntity(
            text=json.dumps({"ok": False, "errors": errors}), content_type="application/json",
        )


def _request_design_id(request: web.Request) -> str:
    return request.query.get("design", DEFAULT_DESIGN_ID)


async def api_get_layout(request: web.Request) -> web.Response:
    design_id = _request_design_id(request)
    record = _design(design_id)
    return web.json_response({"designId": design_id, "layout": record["layout"], "revision": record["revision"]})


async def api_validate_layout(request: web.Request) -> web.Response:
    _design(_request_design_id(request))
    layout = await _validated_request_layout(request)
    _validate_design_link_targets(layout)
    return web.json_response({"ok": True, "layout": layout})


async def api_save_layout(request: web.Request) -> web.Response:
    design_id = _request_design_id(request)
    record = _design(design_id)
    layout = await _validated_request_layout(request)
    _validate_design_link_targets(layout)
    record["layout"] = layout
    record["name"] = layout.get("name", record["name"])
    record["revision"] += 1
    save_studio_atomic()
    ensure_images(layout, prune=True)
    await push_design(design_id, full=True)
    applied = sum(1 for session in _devices.values() if _design_id_for_session(session) == design_id)
    return web.json_response({"ok": True, "applied": applied > 0, "appliedScreens": applied, "revision": record["revision"]})


async def api_preview(request: web.Request) -> web.Response:
    layout = await _validated_request_layout(request)
    ensure_images(layout)
    design_id = request.query.get("design", DEFAULT_DESIGN_ID)
    live_state = _live_states.setdefault(design_id, LivePreviewState()) if request.query.get("live") == "1" else None
    image = renderer.render(layout, _layout_context(design_id, live_state=live_state))
    output = io.BytesIO()
    image.save(output, format="PNG")
    return web.Response(body=output.getvalue(), content_type="image/png", headers={"Cache-Control": "no-store"})


async def api_live_touch(request: web.Request) -> web.Response:
    design_id = _request_design_id(request)
    _design(design_id)
    try:
        body = await request.json()
    except (json.JSONDecodeError, ValueError):
        raise web.HTTPBadRequest(
            text=json.dumps({"ok": False, "error": "body must be JSON"}),
            content_type="application/json",
        )
    if not isinstance(body, dict):
        raise web.HTTPUnprocessableEntity(
            text=json.dumps({"ok": False, "error": "invalid live touch payload"}),
            content_type="application/json",
        )
    try:
        layout = validate_layout(body.get("layout"))
        x, y = int(body.get("x")), int(body.get("y"))
    except (LayoutValidationError, TypeError, ValueError) as exc:
        raise web.HTTPUnprocessableEntity(
            text=json.dumps({"ok": False, "error": "invalid live touch payload"}),
            content_type="application/json",
        ) from exc
    _validate_design_link_targets(layout)
    width, height = layout_dimensions(layout)
    if not (0 <= x < width and 0 <= y < height):
        raise web.HTTPUnprocessableEntity(
            text=json.dumps({"ok": False, "error": "touch is outside the display"}),
            content_type="application/json",
        )
    state = _live_states.setdefault(design_id, LivePreviewState())
    button = renderer.hit_test_buttons(layout, x, y)
    if button:
        asyncio.create_task(_live_press(design_id, state, button, layout))
    else:
        asyncio.create_task(_live_dot(state, x, y))
    navigate_to = button.get("props", {}).get("targetDesignId") if button and button.get("type") == "design-link" else ""
    return web.json_response({
        "ok": True,
        "target": button["id"] if button else "canvas",
        "navigateTo": navigate_to,
    })


async def api_design_preview(request: web.Request) -> web.Response:
    design_id = request.match_info["design_id"]
    record = _design(design_id)
    ensure_images(record["layout"])
    image = renderer.render(record["layout"], _layout_context(design_id))
    output = io.BytesIO()
    image.save(output, format="PNG")
    return web.Response(
        body=output.getvalue(), content_type="image/png",
        headers={"Cache-Control": "no-store"},
    )


async def api_catalog(_request: web.Request) -> web.Response:
    types = ["text", "clock", "external-text", "color-block", "button", "design-link", "image", "ha-state", "ha-toggle", "script"]
    return web.json_response({
        "elementDefaults": {item: new_element(item, "example") for item in types},
        "integrations": {"homeAssistant": {"configured": _ha_client.configured}},
        "scriptExamples": SCRIPT_EXAMPLES,
    })


async def api_studio(_request: web.Request) -> web.Response:
    designs = [
        {
            "id": design_id, "name": record["name"], "revision": record["revision"],
            "orientation": record["layout"].get("orientation", "portrait"),
        }
        for design_id, record in _studio["designs"].items()
    ]
    screens = []
    for screen_id, record in _studio["screens"].items():
        session = _devices.get(screen_id)
        screens.append({
            **record,
            "connected": session is not None,
            "activeDesignId": _design_id_for_session(session) if session else record["designId"],
            "remote": session.remote if session else "",
            "stream": _stream_stats(session) if session else None,
            "heartbeat": {
                "lastPingAt": session.last_ping_at,
                "wifiRssiDbm": session.wifi_rssi_dbm,
                "uptimeMs": session.uptime_ms,
            } if session else None,
        })
    return web.json_response({
        "designs": designs,
        "screens": screens,
        "integrations": {"homeAssistant": {"configured": _ha_client.configured}},
    })


async def api_home_assistant(_request: web.Request) -> web.Response:
    if not _ha_client.configured:
        return web.json_response({"configured": False, "entities": []})
    try:
        states = await _ha_client.list_states(_http_session)
    except Exception as exc:
        return web.json_response({"configured": True, "entities": [], "error": type(exc).__name__})
    entities = []
    for value in states:
        entity_id = value["entity_id"]
        if not entity_id:
            continue
        attributes = value["attributes"]
        _ha_values[entity_id] = value
        entities.append({
            "entityId": entity_id,
            "domain": entity_id.partition(".")[0],
            "name": str(attributes.get("friendly_name") or entity_id),
            "state": value["state"],
            "unit": str(attributes.get("unit_of_measurement") or ""),
        })
    entities.sort(key=lambda item: (item["domain"], item["name"].lower()))
    return web.json_response({"configured": True, "entities": entities})


async def api_get_home_assistant_settings(_request: web.Request) -> web.Response:
    return web.json_response({
        "url": _ha_client.url,
        "configured": _ha_client.configured,
        "tokenConfigured": bool(_ha_client.token),
    })


async def api_save_home_assistant_settings(request: web.Request) -> web.Response:
    global _ha_client
    try:
        body = await request.json()
    except (json.JSONDecodeError, ValueError):
        raise web.HTTPBadRequest(
            text=json.dumps({"ok": False, "error": "body must be JSON"}),
            content_type="application/json",
        )
    url = str(body.get("url", "")).strip().rstrip("/")
    if len(url) > 2048:
        raise web.HTTPUnprocessableEntity(
            text=json.dumps({"ok": False, "error": "Home Assistant URL is too long"}),
            content_type="application/json",
        )
    if body.get("clearToken"):
        token = ""
    else:
        supplied_token = str(body.get("token", "")).strip()
        token = supplied_token or _ha_client.token
    candidate = HomeAssistantClient(url, token)
    if (url or token) and not candidate.configured:
        raise web.HTTPUnprocessableEntity(
            text=json.dumps({"ok": False, "error": "Enter a valid http(s) URL and access token"}),
            content_type="application/json",
        )
    save_dashboard_settings(url, token)
    _ha_client = candidate
    _ha_values.clear()
    _ha_last_fetch.clear()
    connected = False
    connection_error = ""
    entity_count = 0
    if candidate.configured:
        try:
            states = await candidate.list_states(_http_session)
            entity_count = len(states)
            connected = True
            for value in states:
                if value.get("entity_id"):
                    _ha_values[value["entity_id"]] = value
        except Exception as exc:
            connection_error = type(exc).__name__
    await asyncio.gather(*(push_design(design_id) for design_id in _studio["designs"]))
    return web.json_response({
        "ok": True,
        "configured": candidate.configured,
        "tokenConfigured": bool(candidate.token),
        "connected": connected,
        "entityCount": entity_count,
        "connectionError": connection_error,
    })


async def api_create_design(request: web.Request) -> web.Response:
    try:
        body = await request.json()
    except (json.JSONDecodeError, ValueError):
        body = {}
    clone_from = str(body.get("cloneFrom") or DEFAULT_DESIGN_ID)
    source = _design(clone_from)
    design_id = f"design-{secrets.token_hex(4)}"
    name = str(body.get("name") or "New design")[:100]
    layout = json.loads(json.dumps(source["layout"]))
    layout["name"] = name
    _studio["designs"][design_id] = {"id": design_id, "name": name, "revision": 0, "layout": layout}
    save_studio_atomic()
    ensure_images(layout)
    return web.json_response({"ok": True, "design": {"id": design_id, "name": name, "revision": 0}}, status=201)


async def api_assign_design(request: web.Request) -> web.Response:
    design_id = request.match_info["design_id"]
    _design(design_id)
    try:
        body = await request.json()
    except (json.JSONDecodeError, ValueError):
        raise web.HTTPBadRequest(
            text=json.dumps({"ok": False, "error": "body must be JSON"}),
            content_type="application/json",
        )
    screen_ids = body.get("screenIds")
    if not isinstance(screen_ids, list) or len(screen_ids) > 100 or any(not isinstance(value, str) for value in screen_ids):
        raise web.HTTPUnprocessableEntity(
            text=json.dumps({"ok": False, "error": "screenIds must be an array of at most 100 IDs"}),
            content_type="application/json",
        )
    unique_ids = list(dict.fromkeys(screen_ids))
    screens = [_screen(screen_id) for screen_id in unique_ids]
    for screen in screens:
        screen["designId"] = design_id
    save_studio_atomic()
    sessions = [_devices[screen_id] for screen_id in unique_ids if screen_id in _devices]
    for session in sessions:
        session.active_design_id = design_id
        session.last_frame = None
    await asyncio.gather(*(push_frame(session, full=True) for session in sessions))
    return web.json_response({"ok": True, "assigned": len(screens), "online": len(sessions)})


async def api_update_screen(request: web.Request) -> web.Response:
    screen_id = request.match_info["screen_id"]
    screen = _screen(screen_id)
    try:
        body = await request.json()
    except (json.JSONDecodeError, ValueError):
        raise web.HTTPBadRequest(text=json.dumps({"ok": False, "error": "body must be JSON"}), content_type="application/json")
    if "name" in body:
        name = str(body["name"]).strip()
        if not name:
            raise web.HTTPUnprocessableEntity(text=json.dumps({"ok": False, "error": "screen name is required"}), content_type="application/json")
        screen["name"] = name[:100]
    if "designId" in body:
        design_id = str(body["designId"])
        _design(design_id)
        screen["designId"] = design_id
    save_studio_atomic()
    session = _devices.get(screen_id)
    if session:
        if "designId" in body:
            session.active_design_id = screen["designId"]
        session.last_frame = None
        await push_frame(session, full=True)
    return web.json_response({
        "ok": True,
        "screen": {
            **screen,
            "connected": session is not None,
            "activeDesignId": _design_id_for_session(session) if session else screen["designId"],
        },
    })


async def api_restart_screen(request: web.Request) -> web.Response:
    screen_id = request.match_info["screen_id"]
    screen = _screen(screen_id)
    session = _devices.get(screen_id)
    if session is None:
        raise web.HTTPConflict(
            text=json.dumps({"ok": False, "error": "device is offline"}),
            content_type="application/json",
        )
    await session.ws.send(DeviceCommand(DeviceCommandType.RESTART).pack())
    return web.json_response({"ok": True, "screen": {"id": screen_id, "name": screen["name"]}})


async def api_status(_request: web.Request) -> web.Response:
    return web.json_response({
        "deviceConnected": bool(_devices),
        "connectedScreens": len(_devices),
        "knownScreens": len(_studio.get("screens", {})),
        "homeAssistantConfigured": _ha_client.configured,
    })


async def index(_request: web.Request) -> web.FileResponse:
    return web.FileResponse(STATIC_DIR / "index.html")


@web.middleware
async def security_headers(request: web.Request, handler):
    response = await handler(request)
    response.headers["Content-Security-Policy"] = (
        "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; "
        "img-src 'self' blob: data:; connect-src 'self'; object-src 'none'; "
        "base-uri 'self'; frame-ancestors 'none'"
    )
    response.headers["X-Content-Type-Options"] = "nosniff"
    response.headers["Referrer-Policy"] = "no-referrer"
    response.headers["X-Frame-Options"] = "DENY"
    return response


def create_web_app() -> web.Application:
    app = web.Application(client_max_size=512 * 1024, middlewares=[security_headers])
    app.router.add_get("/", index)
    app.router.add_get("/api/layout", api_get_layout)
    app.router.add_post("/api/layout", api_save_layout)
    app.router.add_post("/api/layout/validate", api_validate_layout)
    app.router.add_post("/api/preview", api_preview)
    app.router.add_post("/api/live-touch", api_live_touch)
    app.router.add_get("/api/designs/{design_id}/preview", api_design_preview)
    app.router.add_get("/api/catalog", api_catalog)
    app.router.add_get("/api/integrations/home-assistant", api_home_assistant)
    app.router.add_get("/api/settings/home-assistant", api_get_home_assistant_settings)
    app.router.add_put("/api/settings/home-assistant", api_save_home_assistant_settings)
    app.router.add_get("/api/studio", api_studio)
    app.router.add_post("/api/designs", api_create_design)
    app.router.add_post("/api/designs/{design_id}/assign", api_assign_design)
    app.router.add_put("/api/screens/{screen_id}", api_update_screen)
    app.router.add_post("/api/screens/{screen_id}/restart", api_restart_screen)
    app.router.add_get("/api/status", api_status)
    app.router.add_get("/designer", index)
    app.router.add_get("/devices", index)
    app.router.add_get("/designs", index)
    app.router.add_get("/settings", index)
    app.router.add_static("/static/", STATIC_DIR)
    return app


async def main() -> None:
    global _http_session
    load_studio()
    load_dashboard_settings()
    _http_session = ClientSession(timeout=ClientTimeout(total=EXTERNAL_FETCH_TIMEOUT_S))
    for record in _studio["designs"].values():
        ensure_images(record["layout"])
    refresh_task = asyncio.create_task(external_refresh_loop())
    home_assistant_task = asyncio.create_task(home_assistant_refresh_loop())
    runner = web.AppRunner(create_web_app())
    try:
        await runner.setup()
        await web.TCPSite(runner, WEB_HOST, WEB_PORT).start()
        print(f"web UI on http://{WEB_HOST}:{WEB_PORT}/")
        async with websockets.serve(handle_device, DEVICE_HOST, DEVICE_WS_PORT, ping_interval=None):
            print(f"device websocket on ws://{DEVICE_HOST}:{DEVICE_WS_PORT}/")
            await asyncio.Future()
    finally:
        refresh_task.cancel()
        home_assistant_task.cancel()
        await asyncio.gather(refresh_task, home_assistant_task, return_exceptions=True)
        await runner.cleanup()
        await _http_session.close()


if __name__ == "__main__":
    try:
        asyncio.run(main())
    except KeyboardInterrupt:
        pass

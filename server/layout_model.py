"""Versioned layout validation and migration.

The renderer and browser both consume the normalized v1 dictionary returned by
``validate_layout``.  Keeping normalization here prevents browser-only form
constraints from becoming the server's security boundary.
"""

from __future__ import annotations

import copy
import re
from typing import Any


SCHEMA_VERSION = 1
DISPLAY_WIDTH = 240
DISPLAY_HEIGHT = 320
LANDSCAPE_WIDTH = 320
LANDSCAPE_HEIGHT = 240
MAX_ELEMENTS = 100
MAX_DATA_SOURCES = 20
MAX_ACTIONS = 20

ID_RE = re.compile(r"^[A-Za-z][A-Za-z0-9_-]{0,63}$")
COLOR_RE = re.compile(r"^#[0-9a-fA-F]{6}$")
HA_ENTITY_RE = re.compile(r"^[a-z0-9_]+\.[a-z0-9_]+$")


class LayoutValidationError(ValueError):
    def __init__(self, errors: list[dict[str, str]]):
        super().__init__("invalid layout")
        self.errors = errors


def _default_element(element_type: str, element_id: str) -> dict[str, Any]:
    defaults: dict[str, dict[str, Any]] = {
        "text": {
            "frame": {"x": 20, "y": 140, "w": 200, "h": 40},
            "style": {"color": "#ffffff", "fontSize": 18, "align": "center", "verticalAlign": "middle"},
            "props": {"text": "New text", "wrap": True},
        },
        "clock": {
            "frame": {"x": 20, "y": 130, "w": 200, "h": 60},
            "style": {"color": "#ffffff", "fontSize": 32, "align": "center", "verticalAlign": "middle"},
            "props": {"format": "%H:%M:%S", "timezone": ""},
        },
        "external-text": {
            "frame": {"x": 20, "y": 140, "w": 200, "h": 40},
            "style": {"color": "#88ccff", "fontSize": 14, "align": "center", "verticalAlign": "middle"},
            "props": {"sourceId": "", "wrap": True},
        },
        "color-block": {
            "frame": {"x": 80, "y": 140, "w": 80, "h": 40},
            "style": {"color": "#3355aa", "radius": 0},
            "props": {},
        },
        "button": {
            "frame": {"x": 60, "y": 140, "w": 120, "h": 44},
            "style": {"color": "#2255aa", "fontSize": 16, "radius": 8},
            "props": {"label": "Press me", "actionId": ""},
        },
        "design-link": {
            "frame": {"x": 60, "y": 140, "w": 120, "h": 44},
            "style": {"color": "#6b9cff", "fontSize": 16, "radius": 8},
            "props": {"label": "Open screen", "targetDesignId": "default"},
        },
        "image": {
            "frame": {"x": 80, "y": 120, "w": 80, "h": 80},
            "style": {},
            "props": {"src": "", "fit": "contain"},
        },
        "ha-state": {
            "frame": {"x": 20, "y": 140, "w": 200, "h": 40},
            "style": {"color": "#88ccff", "fontSize": 18, "align": "center", "verticalAlign": "middle"},
            "props": {
                "entityId": "sensor.example", "attribute": "", "prefix": "", "suffix": "",
                "decimals": 1, "showUnit": True, "refreshInterval": 5, "wrap": True,
            },
        },
        "ha-toggle": {
            "frame": {"x": 60, "y": 140, "w": 120, "h": 44},
            "style": {"color": "#f2c94c", "offColor": "#353b48", "fontSize": 16, "radius": 8},
            "props": {"entityId": "light.example", "label": "Light", "refreshInterval": 2},
        },
        "script": {
            "frame": {"x": 20, "y": 130, "w": 200, "h": 60},
            "style": {"color": "#ffffff", "fontSize": 16, "align": "center", "verticalAlign": "middle"},
            "props": {
                "code": 'text("Hello from a script")',
                "refreshInterval": 5,
                "wrap": True,
            },
        },
    }
    if element_type not in defaults:
        raise KeyError(element_type)
    return {
        "id": element_id,
        "type": element_type,
        "name": element_type.replace("-", " ").title(),
        "visible": True,
        "locked": False,
        **copy.deepcopy(defaults[element_type]),
    }


def default_layout() -> dict[str, Any]:
    layout = {
        "schemaVersion": SCHEMA_VERSION,
        "name": "Yello screen",
        "profile": "ili9341-240x320",
        "orientation": "portrait",
        "background": "#0a0a1e",
        "dataSources": [],
        "actions": [],
        "elements": [],
    }
    clock = _default_element("clock", "clock")
    clock["frame"] = {"x": 0, "y": 56, "w": 240, "h": 68}
    clock["style"]["fontSize"] = 48
    date = _default_element("clock", "date")
    date["name"] = "Date"
    date["frame"] = {"x": 0, "y": 124, "w": 240, "h": 32}
    date["style"].update({"fontSize": 20, "color": "#78c8ff"})
    date["props"]["format"] = "%a %b %d, %Y"
    divider = _default_element("color-block", "divider")
    divider["name"] = "Divider"
    divider["frame"] = {"x": 20, "y": 179, "w": 200, "h": 2}
    divider["style"]["color"] = "#505078"
    greeting = _default_element("text", "greeting")
    greeting["name"] = "Greeting"
    greeting["frame"] = {"x": 0, "y": 188, "w": 240, "h": 44}
    greeting["style"].update({"fontSize": 18, "color": "#ffdc78"})
    greeting["props"]["text"] = "Hello from the server!"
    layout["elements"] = [clock, date, divider, greeting]
    return layout


def new_element(element_type: str, element_id: str) -> dict[str, Any]:
    """Return normalized defaults for use by tests and future APIs."""
    return _default_element(element_type, element_id)


def layout_dimensions(layout: dict[str, Any]) -> tuple[int, int]:
    if layout.get("orientation") == "landscape":
        return LANDSCAPE_WIDTH, LANDSCAPE_HEIGHT
    return DISPLAY_WIDTH, DISPLAY_HEIGHT


def migrate_layout(raw: Any) -> Any:
    """Convert the original flat prototype format to schema v1."""
    if not isinstance(raw, dict) or raw.get("schemaVersion") is not None:
        return raw

    migrated = {
        "schemaVersion": SCHEMA_VERSION,
        "name": str(raw.get("name", "Yello screen")),
        "profile": "ili9341-240x320",
        "orientation": "portrait",
        "background": raw.get("background", "#0a0a1e"),
        "dataSources": [],
        "actions": [],
        "elements": [],
    }
    source_by_url: dict[str, str] = {}
    action_by_url: dict[str, str] = {}
    type_aliases = {"time": "clock", "external": "external-text", "rect": "color-block"}

    for index, old in enumerate(raw.get("elements", [])):
        if not isinstance(old, dict):
            continue
        element_type = type_aliases.get(old.get("type"), old.get("type"))
        if element_type not in {"text", "clock", "external-text", "color-block", "button", "design-link", "image", "ha-state", "ha-toggle", "script"}:
            continue
        element_id = str(old.get("id") or f"element{index + 1}")
        el = _default_element(element_type, element_id)
        el["name"] = str(old.get("name") or el["name"])
        size = _legacy_int(old.get("size"), el["style"].get("fontSize", 18))

        if element_type in {"text", "clock", "external-text"}:
            width = _legacy_int(old.get("w"), DISPLAY_WIDTH)
            height = _legacy_int(old.get("h"), max(24, int(size * 1.45)))
            center_x = _legacy_int(old.get("x"), DISPLAY_WIDTH // 2)
            center_y = _legacy_int(old.get("y"), DISPLAY_HEIGHT // 2)
            el["frame"] = {"x": center_x - width // 2, "y": center_y - height // 2, "w": width, "h": height}
        else:
            el["frame"] = {
                "x": _legacy_int(old.get("x"), el["frame"]["x"]),
                "y": _legacy_int(old.get("y"), el["frame"]["y"]),
                "w": _legacy_int(old.get("w"), el["frame"]["w"]),
                "h": _legacy_int(old.get("h"), el["frame"]["h"]),
            }

        if "color" in old:
            el["style"]["color"] = old["color"]
        if "fontSize" in el["style"]:
            el["style"]["fontSize"] = size
        if element_type == "text":
            el["props"]["text"] = old.get("text", "")
        elif element_type == "clock":
            el["props"]["format"] = old.get("format", "%H:%M:%S")
        elif element_type == "external-text":
            url = str(old.get("url", ""))
            if url:
                source_id = source_by_url.get(url)
                if source_id is None:
                    source_id = f"source{len(source_by_url) + 1}"
                    source_by_url[url] = source_id
                    migrated["dataSources"].append({
                        "id": source_id,
                        "name": f"External source {len(source_by_url)}",
                        "type": "http-text",
                        "url": url,
                        "interval": _legacy_int(old.get("interval"), 60),
                    })
                el["props"]["sourceId"] = source_id
        elif element_type == "button":
            el["props"]["label"] = old.get("label", "Button")
            url = str(old.get("action", ""))
            if url:
                action_id = action_by_url.get(url)
                if action_id is None:
                    action_id = f"action{len(action_by_url) + 1}"
                    action_by_url[url] = action_id
                    migrated["actions"].append({
                        "id": action_id,
                        "name": f"Button action {len(action_by_url)}",
                        "type": "http",
                        "method": "GET",
                        "url": url,
                    })
                el["props"]["actionId"] = action_id
        elif element_type == "image":
            el["props"]["src"] = old.get("src", "")
        migrated["elements"].append(el)

    return migrated


def _legacy_int(value: Any, default: int) -> int:
    try:
        return int(value)
    except (TypeError, ValueError):
        return default


class _Validator:
    def __init__(self):
        self.errors: list[dict[str, str]] = []

    def error(self, path: str, message: str) -> None:
        self.errors.append({"path": path, "message": message})

    def string(self, value: Any, path: str, *, default: str = "", max_length: int = 200) -> str:
        if value is None:
            return default
        if not isinstance(value, str):
            self.error(path, "must be a string")
            return default
        if len(value) > max_length:
            self.error(path, f"must contain at most {max_length} characters")
            return value[:max_length]
        return value

    def integer(self, value: Any, path: str, *, default: int, minimum: int, maximum: int) -> int:
        if isinstance(value, bool):
            self.error(path, "must be an integer")
            return default
        try:
            parsed = int(value)
        except (TypeError, ValueError):
            self.error(path, "must be an integer")
            return default
        if parsed < minimum or parsed > maximum:
            self.error(path, f"must be between {minimum} and {maximum}")
        return min(max(parsed, minimum), maximum)

    def color(self, value: Any, path: str, *, default: str = "#ffffff") -> str:
        color = self.string(value, path, default=default, max_length=7)
        if not COLOR_RE.fullmatch(color):
            self.error(path, "must be a #rrggbb color")
            return default
        return color.lower()

    def identifier(self, value: Any, path: str, *, fallback: str) -> str:
        identifier = self.string(value, path, default=fallback, max_length=64)
        if not ID_RE.fullmatch(identifier):
            self.error(path, "must start with a letter and contain only letters, numbers, _ or -")
            return fallback
        return identifier


def validate_layout(raw: Any) -> dict[str, Any]:
    raw = migrate_layout(copy.deepcopy(raw))
    validator = _Validator()
    if not isinstance(raw, dict):
        raise LayoutValidationError([{"path": "$", "message": "must be an object"}])

    if raw.get("schemaVersion") != SCHEMA_VERSION:
        validator.error("schemaVersion", f"must equal {SCHEMA_VERSION}")
    result: dict[str, Any] = {
        "schemaVersion": SCHEMA_VERSION,
        "name": validator.string(raw.get("name"), "name", default="Yello screen", max_length=100),
        "profile": validator.string(raw.get("profile"), "profile", default="ili9341-240x320", max_length=64),
        "orientation": validator.string(raw.get("orientation"), "orientation", default="portrait", max_length=12),
        "background": validator.color(raw.get("background"), "background", default="#0a0a1e"),
        "dataSources": [],
        "actions": [],
        "elements": [],
    }
    if result["profile"] != "ili9341-240x320":
        validator.error("profile", "unsupported display profile")
    if result["orientation"] not in {"portrait", "landscape"}:
        validator.error("orientation", "must be portrait or landscape")
        result["orientation"] = "portrait"
    display_width, display_height = layout_dimensions(result)

    sources = raw.get("dataSources", [])
    if not isinstance(sources, list):
        validator.error("dataSources", "must be an array")
        sources = []
    if len(sources) > MAX_DATA_SOURCES:
        validator.error("dataSources", f"must contain at most {MAX_DATA_SOURCES} items")
    source_ids: set[str] = set()
    for index, source in enumerate(sources[:MAX_DATA_SOURCES]):
        path = f"dataSources[{index}]"
        if not isinstance(source, dict):
            validator.error(path, "must be an object")
            continue
        source_id = validator.identifier(source.get("id"), f"{path}.id", fallback=f"source{index + 1}")
        if source_id in source_ids:
            validator.error(f"{path}.id", "must be unique")
        source_ids.add(source_id)
        source_type = validator.string(source.get("type"), f"{path}.type", default="http-text", max_length=32)
        if source_type != "http-text":
            validator.error(f"{path}.type", "unsupported data source type")
        result["dataSources"].append({
            "id": source_id,
            "name": validator.string(source.get("name"), f"{path}.name", default=source_id, max_length=100),
            "type": "http-text",
            "url": validator.string(source.get("url"), f"{path}.url", max_length=2048),
            "interval": validator.integer(source.get("interval"), f"{path}.interval", default=60, minimum=5, maximum=86400),
        })

    actions = raw.get("actions", [])
    if not isinstance(actions, list):
        validator.error("actions", "must be an array")
        actions = []
    if len(actions) > MAX_ACTIONS:
        validator.error("actions", f"must contain at most {MAX_ACTIONS} items")
    action_ids: set[str] = set()
    for index, action in enumerate(actions[:MAX_ACTIONS]):
        path = f"actions[{index}]"
        if not isinstance(action, dict):
            validator.error(path, "must be an object")
            continue
        action_id = validator.identifier(action.get("id"), f"{path}.id", fallback=f"action{index + 1}")
        if action_id in action_ids:
            validator.error(f"{path}.id", "must be unique")
        action_ids.add(action_id)
        method = validator.string(action.get("method"), f"{path}.method", default="GET", max_length=4).upper()
        if method not in {"GET", "POST"}:
            validator.error(f"{path}.method", "must be GET or POST")
            method = "GET"
        result["actions"].append({
            "id": action_id,
            "name": validator.string(action.get("name"), f"{path}.name", default=action_id, max_length=100),
            "type": "http",
            "method": method,
            "url": validator.string(action.get("url"), f"{path}.url", max_length=2048),
            "body": validator.string(action.get("body"), f"{path}.body", max_length=4096),
        })

    elements = raw.get("elements", [])
    if not isinstance(elements, list):
        validator.error("elements", "must be an array")
        elements = []
    if len(elements) > MAX_ELEMENTS:
        validator.error("elements", f"must contain at most {MAX_ELEMENTS} items")
    element_ids: set[str] = set()
    supported_types = {"text", "clock", "external-text", "color-block", "button", "design-link", "image", "ha-state", "ha-toggle", "script"}
    for index, element in enumerate(elements[:MAX_ELEMENTS]):
        path = f"elements[{index}]"
        if not isinstance(element, dict):
            validator.error(path, "must be an object")
            continue
        element_type = validator.string(element.get("type"), f"{path}.type", max_length=32)
        if element_type not in supported_types:
            validator.error(f"{path}.type", "unsupported element type")
            continue
        element_id = validator.identifier(element.get("id"), f"{path}.id", fallback=f"element{index + 1}")
        if element_id in element_ids:
            validator.error(f"{path}.id", "must be unique")
        element_ids.add(element_id)
        defaults = _default_element(element_type, element_id)
        frame_raw = element.get("frame", {})
        style_raw = element.get("style", {})
        props_raw = element.get("props", {})
        if not isinstance(frame_raw, dict):
            validator.error(f"{path}.frame", "must be an object")
            frame_raw = {}
        if not isinstance(style_raw, dict):
            validator.error(f"{path}.style", "must be an object")
            style_raw = {}
        if not isinstance(props_raw, dict):
            validator.error(f"{path}.props", "must be an object")
            props_raw = {}
        frame = {
            "x": validator.integer(frame_raw.get("x"), f"{path}.frame.x", default=defaults["frame"]["x"], minimum=-display_width, maximum=display_width - 1),
            "y": validator.integer(frame_raw.get("y"), f"{path}.frame.y", default=defaults["frame"]["y"], minimum=-display_height, maximum=display_height - 1),
            "w": validator.integer(frame_raw.get("w"), f"{path}.frame.w", default=defaults["frame"]["w"], minimum=1, maximum=display_width),
            "h": validator.integer(frame_raw.get("h"), f"{path}.frame.h", default=defaults["frame"]["h"], minimum=1, maximum=display_height),
        }
        style: dict[str, Any] = {}
        if "color" in defaults["style"]:
            style["color"] = validator.color(style_raw.get("color"), f"{path}.style.color", default=defaults["style"]["color"])
        if "fontSize" in defaults["style"]:
            style["fontSize"] = validator.integer(style_raw.get("fontSize"), f"{path}.style.fontSize", default=defaults["style"]["fontSize"], minimum=6, maximum=120)
        if element_type in {"text", "clock", "external-text", "ha-state", "script"}:
            align = validator.string(style_raw.get("align"), f"{path}.style.align", default="center", max_length=8)
            vertical = validator.string(style_raw.get("verticalAlign"), f"{path}.style.verticalAlign", default="middle", max_length=8)
            if align not in {"left", "center", "right"}:
                validator.error(f"{path}.style.align", "must be left, center or right")
                align = "center"
            if vertical not in {"top", "middle", "bottom"}:
                validator.error(f"{path}.style.verticalAlign", "must be top, middle or bottom")
                vertical = "middle"
            style.update({"align": align, "verticalAlign": vertical})
        if element_type in {"color-block", "button", "design-link", "ha-toggle"}:
            style["radius"] = validator.integer(style_raw.get("radius"), f"{path}.style.radius", default=defaults["style"]["radius"], minimum=0, maximum=50)
        if element_type == "ha-toggle":
            style["offColor"] = validator.color(style_raw.get("offColor"), f"{path}.style.offColor", default=defaults["style"]["offColor"])

        props: dict[str, Any]
        if element_type == "text":
            props = {
                "text": validator.string(props_raw.get("text"), f"{path}.props.text", max_length=1000),
                "wrap": bool(props_raw.get("wrap", True)),
            }
        elif element_type == "clock":
            props = {
                "format": validator.string(props_raw.get("format"), f"{path}.props.format", default="%H:%M:%S", max_length=100),
                "timezone": validator.string(props_raw.get("timezone"), f"{path}.props.timezone", max_length=64),
            }
        elif element_type == "external-text":
            source_id = validator.string(props_raw.get("sourceId"), f"{path}.props.sourceId", max_length=64)
            if source_id and source_id not in source_ids:
                validator.error(f"{path}.props.sourceId", "references an unknown data source")
            props = {"sourceId": source_id, "wrap": bool(props_raw.get("wrap", True))}
        elif element_type == "button":
            action_id = validator.string(props_raw.get("actionId"), f"{path}.props.actionId", max_length=64)
            if action_id and action_id not in action_ids:
                validator.error(f"{path}.props.actionId", "references an unknown action")
            props = {
                "label": validator.string(props_raw.get("label"), f"{path}.props.label", default="Button", max_length=100),
                "actionId": action_id,
            }
        elif element_type == "design-link":
            target_design_id = validator.string(
                props_raw.get("targetDesignId"), f"{path}.props.targetDesignId",
                default="default", max_length=64,
            )
            if not re.fullmatch(r"[A-Za-z0-9._:-]{1,64}", target_design_id):
                validator.error(f"{path}.props.targetDesignId", "must be a valid design ID")
                target_design_id = "default"
            props = {
                "label": validator.string(
                    props_raw.get("label"), f"{path}.props.label",
                    default="Open screen", max_length=100,
                ),
                "targetDesignId": target_design_id,
            }
        elif element_type == "image":
            fit = validator.string(props_raw.get("fit"), f"{path}.props.fit", default="contain", max_length=8)
            if fit not in {"contain", "cover", "stretch"}:
                validator.error(f"{path}.props.fit", "must be contain, cover or stretch")
                fit = "contain"
            props = {"src": validator.string(props_raw.get("src"), f"{path}.props.src", max_length=2048), "fit": fit}
        elif element_type == "ha-state":
            entity_id = validator.string(props_raw.get("entityId"), f"{path}.props.entityId", default="sensor.example", max_length=255)
            if not HA_ENTITY_RE.fullmatch(entity_id):
                validator.error(f"{path}.props.entityId", "must be a Home Assistant entity ID such as sensor.temperature")
            props = {
                "entityId": entity_id,
                "attribute": validator.string(props_raw.get("attribute"), f"{path}.props.attribute", max_length=100),
                "prefix": validator.string(props_raw.get("prefix"), f"{path}.props.prefix", max_length=100),
                "suffix": validator.string(props_raw.get("suffix"), f"{path}.props.suffix", max_length=100),
                "decimals": validator.integer(props_raw.get("decimals"), f"{path}.props.decimals", default=1, minimum=0, maximum=6),
                "showUnit": bool(props_raw.get("showUnit", True)),
                "refreshInterval": validator.integer(props_raw.get("refreshInterval"), f"{path}.props.refreshInterval", default=5, minimum=1, maximum=3600),
                "wrap": bool(props_raw.get("wrap", True)),
            }
        elif element_type == "ha-toggle":
            entity_id = validator.string(props_raw.get("entityId"), f"{path}.props.entityId", default="light.example", max_length=255)
            if not HA_ENTITY_RE.fullmatch(entity_id):
                validator.error(f"{path}.props.entityId", "must be a Home Assistant entity ID such as light.kitchen")
            elif entity_id.partition(".")[0] not in {"light", "switch", "input_boolean", "fan"}:
                validator.error(f"{path}.props.entityId", "must be a light, switch, input_boolean, or fan entity")
            props = {
                "entityId": entity_id,
                "label": validator.string(props_raw.get("label"), f"{path}.props.label", default="Light", max_length=100),
                "refreshInterval": validator.integer(props_raw.get("refreshInterval"), f"{path}.props.refreshInterval", default=2, minimum=1, maximum=3600),
            }
        elif element_type == "script":
            props = {
                "code": validator.string(
                    props_raw.get("code"), f"{path}.props.code",
                    default='text("Hello from a script")', max_length=8000,
                ),
                "refreshInterval": validator.integer(
                    props_raw.get("refreshInterval"), f"{path}.props.refreshInterval",
                    default=5, minimum=1, maximum=3600,
                ),
                "wrap": bool(props_raw.get("wrap", True)),
            }
        else:
            props = {}

        result["elements"].append({
            "id": element_id,
            "type": element_type,
            "name": validator.string(element.get("name"), f"{path}.name", default=defaults["name"], max_length=100),
            "visible": bool(element.get("visible", True)),
            "locked": bool(element.get("locked", False)),
            "frame": frame,
            "style": style,
            "props": props,
        })

    if validator.errors:
        raise LayoutValidationError(validator.errors)
    return result


def find_by_id(items: list[dict[str, Any]], item_id: str) -> dict[str, Any] | None:
    return next((item for item in items if item.get("id") == item_id), None)

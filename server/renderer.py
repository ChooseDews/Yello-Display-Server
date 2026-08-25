"""Canonical Pillow renderer for normalized schema-v1 Yello layouts."""

from __future__ import annotations

import time
from datetime import datetime
from zoneinfo import ZoneInfo, ZoneInfoNotFoundError

from PIL import Image, ImageDraw, ImageFont, ImageOps

from layout_model import find_by_id, layout_dimensions
from script_runtime import ScriptError, execute_script


FONT_PATH = "/usr/share/fonts/liberation-sans-fonts/LiberationSans-Bold.ttf"
DOT_RADIUS = 10
_fonts: dict[int, ImageFont.FreeTypeFont] = {}


def font(size: int) -> ImageFont.FreeTypeFont:
    size = max(6, min(120, int(size)))
    if size not in _fonts:
        _fonts[size] = ImageFont.truetype(FONT_PATH, size)
    return _fonts[size]


def parse_color(value, default=(255, 255, 255)):
    if isinstance(value, (list, tuple)) and len(value) == 3:
        return tuple(max(0, min(255, int(component))) for component in value)
    if isinstance(value, str):
        value = value.removeprefix("#")
        if len(value) == 6:
            try:
                return tuple(int(value[index:index + 2], 16) for index in (0, 2, 4))
            except ValueError:
                pass
    return default


def _brightness(rgb) -> float:
    return 0.299 * rgb[0] + 0.587 * rgb[1] + 0.114 * rgb[2]


def _wrap_text(text: str, selected_font: ImageFont.FreeTypeFont, width: int) -> str:
    lines: list[str] = []
    for paragraph in text.splitlines() or [""]:
        words = paragraph.split(" ")
        current = ""
        for word in words:
            candidate = word if not current else f"{current} {word}"
            if current and selected_font.getlength(candidate) > width:
                lines.append(current)
                current = word
            else:
                current = candidate
        lines.append(current)
    return "\n".join(lines)


def _draw_text_box(layer: Image.Image, text: str, style: dict, *, wrap: bool = False) -> None:
    width, height = layer.size
    selected_font = font(style.get("fontSize", 18))
    if wrap:
        text = _wrap_text(text, selected_font, max(1, width - 4))
    draw = ImageDraw.Draw(layer)
    align = style.get("align", "center")
    vertical = style.get("verticalAlign", "middle")
    anchor_x = {"left": 2, "center": width // 2, "right": max(0, width - 2)}[align]
    anchor = {"left": "la", "center": "ma", "right": "ra"}[align]
    bbox = draw.multiline_textbbox((0, 0), text, font=selected_font, spacing=2, align=align)
    text_height = bbox[3] - bbox[1]
    if vertical == "top":
        y = 1 - bbox[1]
    elif vertical == "bottom":
        y = max(0, height - text_height - bbox[1] - 1)
    else:
        y = (height - text_height) // 2 - bbox[1]
    draw.multiline_text(
        (anchor_x, y), text, font=selected_font, fill=parse_color(style.get("color")),
        spacing=2, align=align, anchor=anchor,
    )


def _fit_image(source: Image.Image, size: tuple[int, int], fit: str) -> Image.Image:
    width, height = max(1, size[0]), max(1, size[1])
    if fit == "stretch":
        return source.resize((width, height), Image.Resampling.LANCZOS)
    if fit == "cover":
        return ImageOps.fit(source, (width, height), method=Image.Resampling.LANCZOS)
    contained = ImageOps.contain(source, (width, height), method=Image.Resampling.LANCZOS)
    result = Image.new("RGB", (width, height), (0, 0, 0))
    result.paste(contained, ((width - contained.width) // 2, (height - contained.height) // 2))
    return result


def _clock_text(props: dict) -> str:
    timezone = props.get("timezone", "")
    try:
        now = datetime.now(ZoneInfo(timezone)) if timezone else datetime.now().astimezone()
    except ZoneInfoNotFoundError:
        now = datetime.now().astimezone()
    return now.strftime(props.get("format", "%H:%M:%S"))


def _ha_text(props: dict, ctx: dict) -> str:
    entity_id = props.get("entityId", "")
    entity = ctx.get("homeAssistant", {}).get(entity_id)
    if not entity:
        return "HA: waiting"
    if entity.get("error"):
        return "HA: unavailable"
    attributes = entity.get("attributes", {})
    attribute = props.get("attribute", "")
    raw_value = attributes.get(attribute, "unknown") if attribute else entity.get("state", "unknown")
    value = str(raw_value)
    if isinstance(raw_value, (int, float)) or (isinstance(raw_value, str) and _is_number(raw_value)):
        value = f"{float(raw_value):.{props.get('decimals', 1)}f}"
    suffix = props.get("suffix", "")
    if not suffix and props.get("showUnit", True) and not attribute:
        suffix = str(attributes.get("unit_of_measurement", ""))
    spacing = " " if suffix and not suffix.startswith(("°", "%")) else ""
    return f"{props.get('prefix', '')}{value}{spacing}{suffix}"


def _is_number(value: str) -> bool:
    try:
        float(value)
        return True
    except ValueError:
        return False


def _render_element(img: Image.Image, element: dict, ctx: dict) -> None:
    frame = element["frame"]
    x, y, width, height = frame["x"], frame["y"], frame["w"], frame["h"]
    style, props = element.get("style", {}), element.get("props", {})
    layer = Image.new("RGB", (width, height), (0, 0, 0))
    layer_mask = Image.new("L", (width, height), 0)
    mask_draw = ImageDraw.Draw(layer_mask)
    element_type = element["type"]

    if element_type in {"text", "clock", "external-text", "ha-state", "script"}:
        layer = Image.new("RGBA", (width, height), (0, 0, 0, 0))
        if element_type == "text":
            text = str(props.get("text", ""))
        elif element_type == "clock":
            text = _clock_text(props)
        elif element_type == "external-text":
            text = str(ctx.get("external", {}).get(props.get("sourceId", ""), "…"))
        elif element_type == "ha-state":
            text = _ha_text(props, ctx)
        else:
            try:
                output = execute_script(str(props.get("code", "")), width, height, ctx)
                if output.image is not None:
                    img.paste(output.image, (x, y), output.image)
                    return
                text = output.text or ""
            except ScriptError as exc:
                text = f"Script: {exc}"
                style = {**style, "color": "#ff6577", "fontSize": min(12, style.get("fontSize", 12))}
        _draw_text_box(layer, text, style, wrap=bool(props.get("wrap", False)))
        img.paste(layer, (x, y), layer)
        return

    if element_type == "color-block":
        radius = min(style.get("radius", 0), width // 2, height // 2)
        mask_draw.rounded_rectangle((0, 0, width - 1, height - 1), radius=radius, fill=255)
        layer.paste(parse_color(style.get("color")), (0, 0, width, height))
    elif element_type in {"button", "design-link"}:
        color = parse_color(style.get("color"))
        if element.get("id") in ctx.get("pressed", set()):
            color = tuple(min(255, component + 70) for component in color)
        radius = min(style.get("radius", 8), width // 2, height // 2)
        layer_draw = ImageDraw.Draw(layer)
        layer_draw.rounded_rectangle((0, 0, width - 1, height - 1), radius=radius, fill=color, outline=(255, 255, 255), width=2)
        label_style = {
            "fontSize": style.get("fontSize", 16),
            "color": "#000000" if _brightness(color) > 140 else "#ffffff",
            "align": "center", "verticalAlign": "middle",
        }
        _draw_text_box(layer, str(props.get("label", "Button")), label_style)
        mask_draw.rectangle((0, 0, width - 1, height - 1), fill=255)
    elif element_type == "ha-toggle":
        entity = ctx.get("homeAssistant", {}).get(props.get("entityId", ""), {})
        is_on = entity.get("state") == "on"
        color = parse_color(style.get("color") if is_on else style.get("offColor", "#353b48"))
        if element.get("id") in ctx.get("pressed", set()):
            color = tuple(min(255, component + 50) for component in color)
        radius = min(style.get("radius", 8), width // 2, height // 2)
        layer_draw = ImageDraw.Draw(layer)
        layer_draw.rounded_rectangle((0, 0, width - 1, height - 1), radius=radius, fill=color, outline=(255, 255, 255), width=2)
        state_label = "ON" if is_on else "OFF"
        if entity.get("error"):
            state_label = "ERR"
        label = props.get("label", "Light")
        label_style = {
            "fontSize": style.get("fontSize", 16),
            "color": "#000000" if _brightness(color) > 140 else "#ffffff",
            "align": "center", "verticalAlign": "middle",
        }
        _draw_text_box(layer, f"{label}: {state_label}", label_style)
        mask_draw.rectangle((0, 0, width - 1, height - 1), fill=255)
    elif element_type == "image":
        source = ctx.get("images", {}).get(props.get("src", ""))
        if source is not None:
            layer = _fit_image(source, (width, height), props.get("fit", "contain"))
        else:
            layer_draw = ImageDraw.Draw(layer)
            layer_draw.rectangle((0, 0, width - 1, height - 1), fill=(45, 47, 55), outline=(105, 110, 125))
            label_style = {"fontSize": 12, "color": "#aeb4c5", "align": "center", "verticalAlign": "middle"}
            _draw_text_box(layer, "image", label_style)
        mask_draw.rectangle((0, 0, width - 1, height - 1), fill=255)
    img.paste(layer, (x, y), layer_mask)


def render(layout: dict, ctx: dict | None = None) -> Image.Image:
    ctx = ctx or {}
    width, height = layout_dimensions(layout)
    img = Image.new("RGB", (width, height), parse_color(layout.get("background"), (10, 10, 30)))
    for element in layout.get("elements", []):
        if element.get("visible", True):
            _render_element(img, element, ctx)
    now_mono = time.monotonic()
    draw = ImageDraw.Draw(img)
    for x, y, expiry in ctx.get("dots", []):
        if expiry > now_mono:
            draw.ellipse(
                (x - DOT_RADIUS, y - DOT_RADIUS, x + DOT_RADIUS, y + DOT_RADIUS),
                fill=(255, 60, 60), outline=(255, 255, 255), width=2,
            )
    return img


def hit_test_buttons(layout: dict, x: int, y: int) -> dict | None:
    for element in reversed(layout.get("elements", [])):
        if element.get("type") not in {"button", "design-link", "ha-toggle"} or not element.get("visible", True) or element.get("locked", False):
            continue
        frame = element["frame"]
        if frame["x"] <= x < frame["x"] + frame["w"] and frame["y"] <= y < frame["y"] + frame["h"]:
            return element
    return None


def resolve_button_action(layout: dict, element: dict) -> dict | None:
    return find_by_id(layout.get("actions", []), element.get("props", {}).get("actionId", ""))


def has_clock(layout: dict) -> bool:
    return any(
        element.get("type") in {"clock", "script"} and element.get("visible", True)
        for element in layout.get("elements", [])
    )

"""Bounded, expression-oriented scripts for designer script blocks.

This deliberately does not use ``eval`` or ``exec``.  A small AST interpreter
exposes only display drawing primitives and sanitized values supplied by the
renderer.
"""

from __future__ import annotations

import ast
import math
from dataclasses import dataclass
from datetime import datetime
from typing import Any

from PIL import Image, ImageDraw


MAX_AST_NODES = 400
MAX_OPERATIONS = 3000
MAX_LOOP_ITEMS = 100
MAX_TEXT_LENGTH = 2000

SCRIPT_EXAMPLES = [
    {
        "id": "sensor-text",
        "name": "HA sensor text",
        "description": "Formats a Home Assistant sensor as text.",
        "code": 'value = ha("sensor.office_temperature")\ntext(f"Office {value}°C")',
    },
    {
        "id": "progress-bar",
        "name": "Sensor progress bar",
        "description": "Draws a value from 0–100 as a labeled bar.",
        "code": (
            'value = number(ha("sensor.battery_level"), 72)\n'
            'clear("#18202b")\n'
            'rect(4, 4, width - 8, height - 8, "#273447", 6)\n'
            'rect(8, 8, (width - 16) * clamp(value, 0, 100) / 100, height - 16, "#f2c94c", 4)\n'
            'label(width / 2, height / 2, f"{value:.0f}%", "#ffffff", 14, "mm")'
        ),
    },
    {
        "id": "status-badge",
        "name": "Status badge",
        "description": "Changes color from an HA entity state.",
        "code": (
            'state = ha("binary_sensor.front_door")\n'
            'if state == "on":\n'
            '    clear("#9b2c3b")\n'
            '    label(width / 2, height / 2, "DOOR OPEN", "#ffffff", 15, "mm")\n'
            'else:\n'
            '    clear("#1f7a4d")\n'
            '    label(width / 2, height / 2, "Door closed", "#ffffff", 15, "mm")'
        ),
    },
    {
        "id": "mini-chart",
        "name": "Decorative mini chart",
        "description": "Shows bounded loops and line drawing without external data.",
        "code": (
            'clear("#121826")\n'
            'last_x = 0\n'
            'last_y = height / 2\n'
            'for point in range(1, 13):\n'
            '    x = point * width / 12\n'
            '    y = height / 2 + sin(point * 0.9) * height * 0.3\n'
            '    line(last_x, last_y, x, y, "#6b9cff", 2)\n'
            '    last_x = x\n'
            '    last_y = y\n'
            'label(5, 5, now("%H:%M"), "#f2c94c", 11, "la")'
        ),
    },
]


class ScriptError(ValueError):
    pass


@dataclass
class ScriptOutput:
    image: Image.Image | None = None
    text: str | None = None


class _Interpreter:
    def __init__(self, width: int, height: int, context: dict):
        self.width = width
        self.height = height
        self.context = context
        self.variables: dict[str, Any] = {"width": width, "height": height, "pi": math.pi}
        self.image = Image.new("RGBA", (width, height), (0, 0, 0, 0))
        self.draw = ImageDraw.Draw(self.image)
        self.text_output: str | None = None
        self.drew = False
        self.operations = 0

    def tick(self, amount: int = 1) -> None:
        self.operations += amount
        if self.operations > MAX_OPERATIONS:
            raise ScriptError("script operation limit exceeded")

    @staticmethod
    def color(value: Any) -> str:
        value = str(value)
        if len(value) != 7 or not value.startswith("#"):
            raise ScriptError("colors must use #rrggbb")
        try:
            int(value[1:], 16)
        except ValueError as exc:
            raise ScriptError("colors must use #rrggbb") from exc
        return value

    @staticmethod
    def number_value(value: Any, default: Any = 0) -> float:
        try:
            return float(value)
        except (TypeError, ValueError):
            return float(default)

    @staticmethod
    def coordinate(value: Any) -> int:
        try:
            return int(round(float(value)))
        except (TypeError, ValueError) as exc:
            raise ScriptError("drawing coordinates must be numbers") from exc

    def call(self, name: str, args: list[Any]) -> Any:
        self.tick()
        if name == "ha":
            if not 1 <= len(args) <= 2:
                raise ScriptError("ha() expects an entity ID and optional attribute")
            entity = self.context.get("homeAssistant", {}).get(str(args[0]), {})
            if not entity or entity.get("error"):
                return "unavailable"
            if len(args) == 2 and str(args[1]):
                return entity.get("attributes", {}).get(str(args[1]), "unknown")
            return entity.get("state", "unknown")
        if name == "now":
            if len(args) > 1:
                raise ScriptError("now() accepts one optional format")
            return datetime.now().astimezone().strftime(str(args[0]) if args else "%H:%M")
        if name == "number":
            if not 1 <= len(args) <= 2:
                raise ScriptError("number() expects a value and optional default")
            return self.number_value(args[0], args[1] if len(args) > 1 else 0)
        if name == "clamp":
            if len(args) != 3:
                raise ScriptError("clamp() expects value, minimum, maximum")
            return min(max(args[0], args[1]), args[2])
        helpers = {
            "min": min, "max": max, "round": round, "abs": abs,
            "str": str, "int": int, "float": float,
            "sin": math.sin, "cos": math.cos,
        }
        if name in helpers:
            try:
                return helpers[name](*args)
            except (TypeError, ValueError) as exc:
                raise ScriptError(f"invalid arguments for {name}()") from exc
        if name == "text":
            if len(args) != 1:
                raise ScriptError("text() expects one value")
            self.text_output = str(args[0])[:MAX_TEXT_LENGTH]
            return None
        if name == "clear":
            if len(args) != 1:
                raise ScriptError("clear() expects one color")
            self.draw.rectangle((0, 0, self.width, self.height), fill=self.color(args[0]))
            self.drew = True
            return None
        if name == "rect":
            if len(args) not in {5, 6}:
                raise ScriptError("rect() expects x, y, width, height, color, optional radius")
            x, y, width, height = map(self.coordinate, args[:4])
            radius = self.coordinate(args[5]) if len(args) == 6 else 0
            if width > 0 and height > 0:
                self.draw.rounded_rectangle((x, y, x + width - 1, y + height - 1), radius=max(0, radius), fill=self.color(args[4]))
                self.drew = True
            return None
        if name == "line":
            if len(args) not in {5, 6}:
                raise ScriptError("line() expects x1, y1, x2, y2, color, optional width")
            coords = tuple(self.coordinate(value) for value in args[:4])
            line_width = max(1, min(20, self.coordinate(args[5]) if len(args) == 6 else 1))
            self.draw.line(coords, fill=self.color(args[4]), width=line_width)
            self.drew = True
            return None
        if name == "circle":
            if len(args) != 4:
                raise ScriptError("circle() expects x, y, radius, color")
            x, y, radius = (self.coordinate(value) for value in args[:3])
            radius = max(0, radius)
            self.draw.ellipse((x - radius, y - radius, x + radius, y + radius), fill=self.color(args[3]))
            self.drew = True
            return None
        if name == "label":
            if not 3 <= len(args) <= 6:
                raise ScriptError("label() expects x, y, value and optional color, size, anchor")
            # Imported lazily to avoid a renderer/script_runtime import cycle.
            from renderer import font
            x, y = self.coordinate(args[0]), self.coordinate(args[1])
            value = str(args[2])[:MAX_TEXT_LENGTH]
            color = self.color(args[3]) if len(args) >= 4 else "#ffffff"
            size = max(6, min(120, self.coordinate(args[4]))) if len(args) >= 5 else 12
            anchor = str(args[5]) if len(args) >= 6 else "la"
            if anchor not in {"la", "ma", "ra", "lm", "mm", "rm", "ls", "ms", "rs"}:
                raise ScriptError("unsupported label anchor")
            self.draw.text((x, y), value, fill=color, font=font(size), anchor=anchor)
            self.drew = True
            return None
        raise ScriptError(f"function not allowed: {name}")

    def expression(self, node: ast.AST) -> Any:
        self.tick()
        if isinstance(node, ast.Constant):
            if isinstance(node.value, (str, int, float, bool)) or node.value is None:
                return node.value
        elif isinstance(node, ast.Name):
            if node.id in self.variables:
                return self.variables[node.id]
            raise ScriptError(f"unknown name: {node.id}")
        elif isinstance(node, ast.BinOp):
            left, right = self.expression(node.left), self.expression(node.right)
            if isinstance(node.op, ast.Add) and isinstance(left, str) and isinstance(right, str):
                if len(left) + len(right) > MAX_TEXT_LENGTH:
                    raise ScriptError("text result is too long")
            if isinstance(node.op, ast.Mult):
                text_value, count = (left, right) if isinstance(left, str) else (right, left)
                if isinstance(text_value, str) and isinstance(count, int) and len(text_value) * max(0, count) > MAX_TEXT_LENGTH:
                    raise ScriptError("text result is too long")
            operators = {
                ast.Add: lambda: left + right, ast.Sub: lambda: left - right,
                ast.Mult: lambda: left * right, ast.Div: lambda: left / right,
                ast.FloorDiv: lambda: left // right, ast.Mod: lambda: left % right,
            }
            operation = operators.get(type(node.op))
            if operation:
                try:
                    value = operation()
                except (TypeError, ValueError, ZeroDivisionError) as exc:
                    raise ScriptError("invalid arithmetic") from exc
                if isinstance(value, str) and len(value) > MAX_TEXT_LENGTH:
                    raise ScriptError("text result is too long")
                return value
        elif isinstance(node, ast.UnaryOp):
            value = self.expression(node.operand)
            if isinstance(node.op, ast.USub): return -value
            if isinstance(node.op, ast.UAdd): return +value
            if isinstance(node.op, ast.Not): return not value
        elif isinstance(node, ast.BoolOp):
            values = [self.expression(item) for item in node.values]
            return all(values) if isinstance(node.op, ast.And) else any(values)
        elif isinstance(node, ast.Compare):
            left = self.expression(node.left)
            for operator, comparator in zip(node.ops, node.comparators):
                right = self.expression(comparator)
                try:
                    if isinstance(operator, ast.Eq): result = left == right
                    elif isinstance(operator, ast.NotEq): result = left != right
                    elif isinstance(operator, ast.Lt): result = left < right
                    elif isinstance(operator, ast.LtE): result = left <= right
                    elif isinstance(operator, ast.Gt): result = left > right
                    elif isinstance(operator, ast.GtE): result = left >= right
                    else: raise ScriptError("comparison not allowed")
                except TypeError as exc:
                    raise ScriptError("invalid comparison") from exc
                if not result:
                    return False
                left = right
            return True
        elif isinstance(node, ast.IfExp):
            return self.expression(node.body if self.expression(node.test) else node.orelse)
        elif isinstance(node, ast.Call) and isinstance(node.func, ast.Name) and not node.keywords:
            return self.call(node.func.id, [self.expression(arg) for arg in node.args])
        elif isinstance(node, ast.JoinedStr):
            parts = []
            for item in node.values:
                if isinstance(item, ast.Constant):
                    parts.append(str(item.value))
                elif isinstance(item, ast.FormattedValue):
                    value = self.expression(item.value)
                    format_spec = ""
                    if item.format_spec:
                        format_spec = str(self.expression(item.format_spec))
                    parts.append(format(value, format_spec))
                else:
                    raise ScriptError("unsupported formatted string")
            return "".join(parts)[:MAX_TEXT_LENGTH]
        raise ScriptError(f"syntax not allowed: {type(node).__name__}")

    def statements(self, statements: list[ast.stmt]) -> None:
        for statement in statements:
            self.tick()
            if isinstance(statement, ast.Expr):
                self.expression(statement.value)
            elif isinstance(statement, ast.Assign) and len(statement.targets) == 1 and isinstance(statement.targets[0], ast.Name):
                name = statement.targets[0].id
                if name.startswith("_") or name in {"width", "height", "pi"}:
                    raise ScriptError(f"cannot assign to {name}")
                self.variables[name] = self.expression(statement.value)
            elif isinstance(statement, ast.If):
                self.statements(statement.body if self.expression(statement.test) else statement.orelse)
            elif isinstance(statement, ast.For) and isinstance(statement.target, ast.Name):
                if not isinstance(statement.iter, ast.Call) or not isinstance(statement.iter.func, ast.Name) or statement.iter.func.id != "range" or statement.iter.keywords:
                    raise ScriptError("for loops must use range()")
                range_args = [self.expression(arg) for arg in statement.iter.args]
                try:
                    values = range(*map(int, range_args))
                except (TypeError, ValueError) as exc:
                    raise ScriptError("invalid range()") from exc
                if len(values) > MAX_LOOP_ITEMS:
                    raise ScriptError(f"loops are limited to {MAX_LOOP_ITEMS} items")
                for value in values:
                    self.variables[statement.target.id] = value
                    self.statements(statement.body)
            else:
                raise ScriptError(f"statement not allowed: {type(statement).__name__}")


def execute_script(code: str, width: int, height: int, context: dict | None = None) -> ScriptOutput:
    if len(code) > 8000:
        raise ScriptError("script is too long")
    try:
        tree = ast.parse(code, mode="exec")
    except SyntaxError as exc:
        raise ScriptError(f"line {exc.lineno}: {exc.msg}") from exc
    if sum(1 for _ in ast.walk(tree)) > MAX_AST_NODES:
        raise ScriptError("script is too complex")
    interpreter = _Interpreter(max(1, width), max(1, height), context or {})
    interpreter.statements(tree.body)
    if interpreter.drew:
        return ScriptOutput(image=interpreter.image)
    return ScriptOutput(text=interpreter.text_output or "")

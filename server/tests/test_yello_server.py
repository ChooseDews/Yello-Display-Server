from __future__ import annotations

import asyncio
import json
import struct
import sys
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

import numpy as np
from aiohttp import web

SERVER_DIR = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SERVER_DIR))

import renderer
import server
from frame_diff import dirty_regions
from home_assistant import HomeAssistantClient, HomeAssistantError
from layout_model import LayoutValidationError, default_layout, migrate_layout, new_element, validate_layout
from protocol import (
    PROTO_MAGIC, PROTO_VERSION, DeviceCommand, DeviceCommandType, DeviceConfig,
    DeviceStatus, MsgType, Orientation, PixelFormat, TouchEvent, TouchType, ZoneUpdate,
)
from safe_http import UnsafeUrlError, parse_remote_url, validate_remote_url
from script_runtime import SCRIPT_EXAMPLES, ScriptError, execute_script


class LayoutModelTests(unittest.TestCase):
    def test_default_layout_is_valid_and_normalized(self):
        layout = validate_layout(default_layout())
        self.assertEqual(layout["schemaVersion"], 1)
        self.assertEqual([item["type"] for item in layout["elements"]], ["clock", "clock", "color-block", "text"])
        self.assertTrue(all(set(item["frame"]) == {"x", "y", "w", "h"} for item in layout["elements"]))

    def test_legacy_layout_migrates_sources_actions_and_center_coordinates(self):
        legacy = {
            "background": "#010203",
            "elements": [
                {"id": "weather", "type": "external", "x": 120, "y": 40, "url": "https://example.com/value", "interval": 30},
                {"id": "go", "type": "button", "x": 10, "y": 20, "w": 90, "h": 40, "label": "Go", "action": "https://example.com/go"},
            ],
        }
        layout = validate_layout(legacy)
        external, button = layout["elements"]
        self.assertEqual(external["type"], "external-text")
        self.assertEqual(external["frame"]["x"], 0)
        self.assertEqual(external["props"]["sourceId"], layout["dataSources"][0]["id"])
        self.assertEqual(button["props"]["actionId"], layout["actions"][0]["id"])

    def test_duplicate_ids_and_dangling_references_are_rejected(self):
        layout = default_layout()
        duplicate = new_element("text", layout["elements"][0]["id"])
        layout["elements"].append(duplicate)
        external = new_element("external-text", "external")
        external["props"]["sourceId"] = "missing"
        layout["elements"].append(external)
        with self.assertRaises(LayoutValidationError) as caught:
            validate_layout(layout)
        paths = {error["path"] for error in caught.exception.errors}
        self.assertIn("elements[4].id", paths)
        self.assertIn("elements[5].props.sourceId", paths)

    def test_invalid_geometry_color_and_element_type_are_rejected(self):
        layout = default_layout()
        layout["background"] = "blue"
        layout["elements"][0]["frame"]["w"] = 0
        layout["elements"].append({"id": "bad", "type": "video"})
        with self.assertRaises(LayoutValidationError) as caught:
            validate_layout(layout)
        self.assertGreaterEqual(len(caught.exception.errors), 3)

    def test_migrate_is_non_destructive_for_v1(self):
        layout = default_layout()
        self.assertEqual(migrate_layout(layout), layout)

    def test_home_assistant_blocks_validate_entity_ids(self):
        layout = default_layout()
        sensor = new_element("ha-state", "temperature")
        sensor["props"]["entityId"] = "sensor.office_temperature"
        toggle = new_element("ha-toggle", "officeLight")
        toggle["props"]["entityId"] = "light.office"
        layout["elements"] = [sensor, toggle]
        normalized = validate_layout(layout)
        self.assertEqual([item["type"] for item in normalized["elements"]], ["ha-state", "ha-toggle"])
        sensor["props"]["entityId"] = "not an entity"
        with self.assertRaises(LayoutValidationError):
            validate_layout({**layout, "elements": [sensor]})

    def test_script_block_validates_and_preserves_bounded_code(self):
        layout = default_layout()
        script = new_element("script", "statusScript")
        script["props"]["code"] = 'text(f"{ha(\'sensor.office\')}")'
        layout["elements"] = [script]
        normalized = validate_layout(layout)
        self.assertEqual(normalized["elements"][0]["type"], "script")
        self.assertEqual(normalized["elements"][0]["props"]["refreshInterval"], 5)

    def test_landscape_layout_uses_320_by_240_bounds(self):
        layout = default_layout()
        layout["orientation"] = "landscape"
        block = new_element("color-block", "edge")
        block["frame"] = {"x": 300, "y": 220, "w": 20, "h": 20}
        layout["elements"] = [block]
        normalized = validate_layout(layout)
        self.assertEqual(normalized["orientation"], "landscape")
        self.assertEqual(renderer.render(normalized).size, (320, 240))

    def test_screen_button_validates_and_preserves_its_target(self):
        layout = default_layout()
        button = new_element("design-link", "openMenu")
        button["props"].update({"label": "Menu", "targetDesignId": "menu"})
        layout["elements"] = [button]
        normalized = validate_layout(layout)
        self.assertEqual(normalized["elements"][0]["type"], "design-link")
        self.assertEqual(normalized["elements"][0]["props"], {"label": "Menu", "targetDesignId": "menu"})


class RendererTests(unittest.TestCase):
    def test_render_has_exact_device_dimensions(self):
        image = renderer.render(default_layout())
        self.assertEqual(image.size, (240, 320))
        self.assertEqual(image.mode, "RGB")

    def test_color_block_uses_frame_and_layer_order(self):
        layout = default_layout()
        layout["elements"] = []
        first = new_element("color-block", "first")
        first["frame"] = {"x": 10, "y": 10, "w": 30, "h": 30}
        first["style"]["color"] = "#ff0000"
        second = new_element("color-block", "second")
        second["frame"] = {"x": 20, "y": 20, "w": 30, "h": 30}
        second["style"]["color"] = "#0000ff"
        layout["elements"] = [first, second]
        image = renderer.render(validate_layout(layout))
        self.assertEqual(image.getpixel((12, 12)), (255, 0, 0))
        self.assertEqual(image.getpixel((25, 25)), (0, 0, 255))

    def test_hit_test_returns_topmost_visible_unlocked_button(self):
        layout = default_layout()
        layout["elements"] = []
        lower = new_element("button", "lower")
        upper = new_element("button", "upper")
        lower["frame"] = upper["frame"] = {"x": 10, "y": 10, "w": 100, "h": 50}
        layout["elements"] = [lower, upper]
        self.assertEqual(renderer.hit_test_buttons(layout, 20, 20)["id"], "upper")
        upper["locked"] = True
        self.assertEqual(renderer.hit_test_buttons(layout, 20, 20)["id"], "lower")

    def test_external_text_reads_context_by_source_id(self):
        layout = default_layout()
        layout["dataSources"] = [{"id": "source1", "name": "Source", "type": "http-text", "url": "https://example.com", "interval": 60}]
        element = new_element("external-text", "external")
        element["props"]["sourceId"] = "source1"
        layout["elements"] = [element]
        image = renderer.render(validate_layout(layout), {"external": {"source1": "42"}})
        self.assertNotEqual(image.getbbox(), None)

    def test_home_assistant_sensor_formats_state_and_unit(self):
        element = new_element("ha-state", "temperature")
        element["props"].update({"entityId": "sensor.office", "decimals": 1, "showUnit": True})
        context = {"homeAssistant": {"sensor.office": {
            "state": "21.45", "attributes": {"unit_of_measurement": "°C"},
        }}}
        self.assertEqual(renderer._ha_text(element["props"], context), "21.4°C")

    def test_home_assistant_toggle_is_touch_target(self):
        layout = default_layout()
        toggle = new_element("ha-toggle", "lightButton")
        toggle["frame"] = {"x": 10, "y": 20, "w": 100, "h": 40}
        layout["elements"] = [toggle]
        self.assertEqual(renderer.hit_test_buttons(layout, 20, 30)["id"], "lightButton")

    def test_screen_button_is_rendered_and_touchable(self):
        layout = default_layout()
        button = new_element("design-link", "menuButton")
        button["frame"] = {"x": 10, "y": 20, "w": 100, "h": 40}
        button["props"].update({"label": "Menu", "targetDesignId": "menu"})
        layout["elements"] = [button]
        normalized = validate_layout(layout)
        self.assertEqual(renderer.hit_test_buttons(normalized, 20, 30)["id"], "menuButton")
        self.assertNotEqual(renderer.render(normalized).getpixel((20, 30)), (9, 10, 27))

    def test_script_can_render_text_and_graphics(self):
        layout = default_layout()
        script = new_element("script", "scriptBlock")
        script["frame"] = {"x": 10, "y": 20, "w": 100, "h": 40}
        script["props"]["code"] = 'clear("#ff0000")\nrect(10, 10, 20, 10, "#00ff00")'
        layout["elements"] = [script]
        image = renderer.render(validate_layout(layout))
        self.assertEqual(image.getpixel((12, 22)), (255, 0, 0))
        self.assertEqual(image.getpixel((22, 32)), (0, 255, 0))


class ScriptRuntimeTests(unittest.TestCase):
    def test_examples_execute_with_bounded_context(self):
        context = {"homeAssistant": {
            "sensor.office_temperature": {"state": "21.5", "attributes": {}},
            "sensor.battery_level": {"state": "72", "attributes": {}},
            "binary_sensor.front_door": {"state": "off", "attributes": {}},
        }}
        outputs = [execute_script(example["code"], 200, 60, context) for example in SCRIPT_EXAMPLES]
        self.assertEqual(outputs[0].text, "Office 21.5°C")
        self.assertTrue(all(output.text is not None or output.image is not None for output in outputs))

    def test_imports_attributes_and_large_loops_are_rejected(self):
        for code in ["import os", "text((1).__class__)", "for x in range(101):\n    text(x)"]:
            with self.assertRaises(ScriptError):
                execute_script(code, 100, 50, {})


class HomeAssistantClientTests(unittest.IsolatedAsyncioTestCase):
    class Response:
        def __init__(self, value=None):
            self.status = 200
            self.value = value

        async def __aenter__(self):
            return self

        async def __aexit__(self, *_args):
            return False

        def raise_for_status(self):
            return None

        async def json(self):
            return self.value

        async def read(self):
            return b"[]"

    class Session:
        def __init__(self, value=None):
            self.value = value
            self.posted = None

        def get(self, url, **kwargs):
            return HomeAssistantClientTests.Response(self.value)

        def post(self, url, **kwargs):
            self.posted = (url, kwargs.get("json"))
            return HomeAssistantClientTests.Response([])

    async def test_state_read_is_sanitized_and_toggle_uses_service_api(self):
        client = HomeAssistantClient("http://ha.local:8123", "secret")
        session = self.Session({
            "entity_id": "light.office", "state": "on", "last_changed": "now",
            "attributes": {"friendly_name": "Office", "nested": {"not": "exposed"}},
        })
        state = await client.get_state(session, "light.office")
        self.assertEqual(state["attributes"], {"friendly_name": "Office"})
        await client.toggle(session, "light.office")
        self.assertEqual(session.posted[1], {"entity_id": "light.office"})
        self.assertTrue(session.posted[0].endswith("/api/services/homeassistant/toggle"))

    async def test_invalid_entity_is_rejected_before_request(self):
        client = HomeAssistantClient("http://ha.local:8123", "secret")
        with self.assertRaises(HomeAssistantError):
            await client.get_state(self.Session(), "../../secrets")
        with self.assertRaises(HomeAssistantError):
            await client.toggle(self.Session(), "lock.front_door")


class FrameDiffTests(unittest.TestCase):
    def test_no_change_has_no_regions(self):
        frame = np.zeros((320, 240, 3), dtype=np.uint8)
        self.assertEqual(dirty_regions(frame, frame.copy()), [])

    def test_distant_pixels_create_small_bounded_regions(self):
        previous = np.zeros((320, 240, 3), dtype=np.uint8)
        current = previous.copy()
        current[5, 5] = 255
        current[300, 220] = 255
        regions = dirty_regions(current, previous)
        self.assertEqual(len(regions), 2)
        self.assertTrue(all(width * height <= 480 for _, _, width, height in regions))

    def test_large_change_uses_full_frame_marker(self):
        previous = np.zeros((320, 240, 3), dtype=np.uint8)
        current = np.full_like(previous, 255)
        self.assertEqual(dirty_regions(current, previous), [(0, 0, 240, 320)])


class ProtocolTests(unittest.TestCase):
    def test_rgb565_conversion_preserves_channel_order_and_wire_endianness(self):
        pixels = np.array([[
            [255, 0, 0], [0, 255, 0], [0, 0, 255],
            [255, 255, 255], [0, 0, 0], [123, 201, 77],
        ]], dtype=np.uint8)
        self.assertEqual(
            server._to_rgb565_be(pixels),
            bytes.fromhex("f800 07e0 001f ffff 0000 7e49"),
        )

    def test_zone_update_header_and_big_endian_pixels(self):
        packed = ZoneUpdate(x=1, y=2, w=1, h=1, pixels=b"\x12\x34").pack()
        header = struct.unpack("<BBBBHHHHI", packed[:16])
        self.assertEqual(header, (PROTO_MAGIC, PROTO_VERSION, MsgType.ZONE_UPDATE, PixelFormat.RAW_RGB565, 1, 2, 1, 1, 2))
        self.assertEqual(packed[16:], b"\x12\x34")

    def test_touch_event_rejects_bad_magic(self):
        packed = struct.pack("<BBBBHHI", 0, PROTO_VERSION, MsgType.TOUCH_EVENT, TouchType.DOWN, 1, 2, 3)
        with self.assertRaises(ValueError):
            TouchEvent.unpack(packed)

    def test_orientation_status_and_restart_messages_are_fixed_size(self):
        config = DeviceConfig(Orientation.LANDSCAPE, 320, 240).pack()
        self.assertEqual(len(config), 8)
        self.assertEqual(config[:4], bytes([PROTO_MAGIC, PROTO_VERSION, MsgType.DEVICE_CONFIG, Orientation.LANDSCAPE]))
        status = struct.pack("<BBBBIhH", PROTO_MAGIC, PROTO_VERSION, MsgType.DEVICE_STATUS, 0, 12345, -62, 0)
        self.assertEqual(DeviceStatus.unpack(status).wifi_rssi_dbm, -62)
        command = DeviceCommand(DeviceCommandType.RESTART).pack()
        self.assertEqual(command, bytes([PROTO_MAGIC, PROTO_VERSION, MsgType.DEVICE_COMMAND, DeviceCommandType.RESTART]))

class StreamTelemetryTests(unittest.IsolatedAsyncioTestCase):
    async def test_full_frame_stream_uses_bounded_messages_and_reports_stats(self):
        class FakeWebSocket:
            def __init__(self):
                self.messages = []

            async def send(self, message):
                self.messages.append(message)

        websocket = FakeWebSocket()
        frame = np.zeros((320, 240, 3), dtype=np.uint8)
        await server.send_region(websocket, frame, 0, 0, 240, 320)
        self.assertEqual(len(websocket.messages), 160)
        self.assertTrue(all(len(message) <= 976 for message in websocket.messages))

        session = server.DeviceSession("screen", websocket, "remote")
        session.connected_at -= 2
        server._record_stream_sample(session, 76800, 156160, 160, 120.0)
        stats = server._stream_stats(session)
        self.assertGreater(stats["pixelsPerSecond"], 0)
        self.assertEqual(stats["lastPushMs"], 120.0)


class SafeHttpTests(unittest.IsolatedAsyncioTestCase):
    def test_parse_rejects_credentials_and_non_http_schemes(self):
        with self.assertRaises(UnsafeUrlError):
            parse_remote_url("file:///etc/passwd")
        with self.assertRaises(UnsafeUrlError):
            parse_remote_url("http://user:pass@example.com/")

    async def test_private_literal_is_blocked_unless_allowlisted(self):
        with self.assertRaises(UnsafeUrlError):
            await validate_remote_url("http://127.0.0.1/private")
        await validate_remote_url("http://127.0.0.1/private", {"127.0.0.1"})


class PersistenceAndApiTests(unittest.IsolatedAsyncioTestCase):
    async def test_atomic_save_round_trip(self):
        with tempfile.TemporaryDirectory() as directory:
            original = server.LAYOUT_PATH
            server.LAYOUT_PATH = Path(directory) / "layout.json"
            try:
                layout = default_layout()
                server.save_layout_atomic(layout)
                self.assertEqual(json.loads(server.LAYOUT_PATH.read_text()), layout)
                self.assertEqual(list(Path(directory).glob("*.tmp")), [])
            finally:
                server.LAYOUT_PATH = original

    async def test_request_validation_returns_422(self):
        class FakeRequest:
            async def json(self):
                return {"schemaVersion": 1, "background": "invalid", "elements": []}

        with self.assertRaises(web.HTTPUnprocessableEntity) as caught:
            await server._validated_request_layout(FakeRequest())
        body = json.loads(caught.exception.text)
        self.assertFalse(body["ok"])
        self.assertEqual(body["errors"][0]["path"], "background")

    async def test_code_editor_validation_normalizes_without_saving(self):
        original_studio = server._studio
        saved = default_layout()
        server._studio = {
            "schemaVersion": 1,
            "designs": {"default": {"id": "default", "name": "Default", "revision": 3, "layout": saved}},
            "screens": {},
        }

        class Request:
            query = {"design": "default"}

            async def json(self):
                candidate = default_layout()
                candidate["name"] = "Edited in code"
                return candidate

        try:
            result = json.loads((await server.api_validate_layout(Request())).text)
            self.assertTrue(result["ok"])
            self.assertEqual(result["layout"]["name"], "Edited in code")
            self.assertEqual(server._studio["designs"]["default"]["revision"], 3)
            self.assertIs(server._studio["designs"]["default"]["layout"], saved)
        finally:
            server._studio = original_studio

    async def test_dashboard_settings_never_return_token_and_use_private_file(self):
        class Request:
            async def json(self):
                return {"url": "http://ha.local:8123", "token": "top-secret"}

        with tempfile.TemporaryDirectory() as directory:
            original_path = server.SECRETS_PATH
            original_client = server._ha_client
            original_session = server._http_session
            original_studio = server._studio
            server.SECRETS_PATH = Path(directory) / "secrets.json"
            server._http_session = HomeAssistantClientTests.Session([])
            server._studio = {"schemaVersion": 1, "designs": {}, "screens": {}}
            try:
                response = await server.api_save_home_assistant_settings(Request())
                saved_result = json.loads(response.text)
                public_result = json.loads((await server.api_get_home_assistant_settings(None)).text)
                self.assertTrue(saved_result["connected"])
                self.assertNotIn("token", public_result)
                self.assertTrue(public_result["tokenConfigured"])
                self.assertEqual(server.SECRETS_PATH.stat().st_mode & 0o777, 0o600)
                self.assertIn("top-secret", server.SECRETS_PATH.read_text())
            finally:
                server.SECRETS_PATH = original_path
                server._ha_client = original_client
                server._http_session = original_session
                server._studio = original_studio


class MultiScreenTests(unittest.IsolatedAsyncioTestCase):
    def setUp(self):
        self.original_studio = server._studio
        self.original_devices = server._devices
        server._devices = {}

    def tearDown(self):
        server._studio = self.original_studio
        server._devices = self.original_devices

    async def test_device_identity_comes_from_websocket_query(self):
        ws = SimpleNamespace(
            request=SimpleNamespace(path="/ws?device_id=d0ef765d7530"),
            remote_address=("192.0.2.10", 1234),
        )
        self.assertEqual(server._device_id_from_ws(ws), "d0ef765d7530")

    async def test_legacy_client_gets_stable_address_fallback(self):
        ws = SimpleNamespace(request=SimpleNamespace(path="/ws"), remote_address=("192.0.2.10", 1234))
        self.assertEqual(server._device_id_from_ws(ws), "legacy-192.0.2.10")

    async def test_registering_mac_migrates_legacy_screen_assignment(self):
        layout = default_layout()
        server._studio = {
            "schemaVersion": 1,
            "designs": {"default": {"id": "default", "name": "Default", "revision": 0, "layout": layout}},
            "screens": {
                "legacy-192.0.2.10": {
                    "id": "legacy-192.0.2.10",
                    "name": "Kitchen",
                    "designId": "default",
                    "lastSeen": "",
                },
            },
        }

        with patch.object(server, "save_studio_atomic"):
            screen = server._register_screen("d0ef765d7530", "192.0.2.10")

        self.assertEqual(screen["id"], "d0ef765d7530")
        self.assertEqual(screen["name"], "Kitchen")
        self.assertNotIn("legacy-192.0.2.10", server._studio["screens"])

    async def test_each_session_renders_its_assigned_design(self):
        red = default_layout()
        red["background"] = "#ff0000"
        red["elements"] = []
        blue = default_layout()
        blue["background"] = "#0000ff"
        blue["elements"] = []
        server._studio = {
            "schemaVersion": 1,
            "designs": {
                "red": {"id": "red", "name": "Red", "revision": 0, "layout": red},
                "blue": {"id": "blue", "name": "Blue", "revision": 0, "layout": blue},
            },
            "screens": {
                "screen-a": {"id": "screen-a", "name": "A", "designId": "red", "lastSeen": ""},
                "screen-b": {"id": "screen-b", "name": "B", "designId": "blue", "lastSeen": ""},
            },
        }
        class WebSocket:
            async def send(self, _message):
                pass

        first = server.DeviceSession("screen-a", WebSocket(), "first")
        second = server.DeviceSession("screen-b", WebSocket(), "second")
        observed = []

        async def capture_region(ws, frame, x, y, width, height):
            observed.append(tuple(frame[0, 0]))

        with patch.object(server, "send_region", new=capture_region):
            await server.push_frame(first, full=True)
            await server.push_frame(second, full=True)

        self.assertEqual(observed, [(255, 0, 0), (0, 0, 255)])
        self.assertIsNot(first.last_frame, second.last_frame)

    async def test_push_design_targets_only_assigned_connected_screens(self):
        layout = default_layout()
        server._studio = {
            "schemaVersion": 1,
            "designs": {
                "one": {"id": "one", "name": "One", "revision": 0, "layout": layout},
                "two": {"id": "two", "name": "Two", "revision": 0, "layout": layout},
            },
            "screens": {
                "a": {"id": "a", "name": "A", "designId": "one", "lastSeen": ""},
                "b": {"id": "b", "name": "B", "designId": "two", "lastSeen": ""},
            },
        }
        server._devices = {
            "a": server.DeviceSession("a", SimpleNamespace(), "a"),
            "b": server.DeviceSession("b", SimpleNamespace(), "b"),
        }
        pushed = []

        async def capture_push(session, *, full=False):
            pushed.append((session.device_id, full))

        with patch.object(server, "push_frame", new=capture_push):
            await server.push_design("one", full=True)
        self.assertEqual(pushed, [("a", True)])

    async def test_screen_button_changes_only_the_session_active_design(self):
        home = default_layout()
        button = new_element("design-link", "openMenu")
        button["props"].update({"label": "Menu", "targetDesignId": "menu"})
        home["elements"] = [button]
        menu = default_layout()
        server._studio = {
            "schemaVersion": 1,
            "designs": {
                "home": {"id": "home", "name": "Home", "revision": 0, "layout": home},
                "menu": {"id": "menu", "name": "Menu", "revision": 0, "layout": menu},
            },
            "screens": {
                "a": {"id": "a", "name": "A", "designId": "home", "lastSeen": ""},
            },
        }
        session = server.DeviceSession("a", SimpleNamespace(), "remote")
        pushed = []

        async def capture_push(current, *, full=False):
            pushed.append((server._design_id_for_session(current), full))

        with patch.object(server, "push_frame", new=capture_push), patch.object(server, "BUTTON_FLASH_S", 0):
            await server.press_button(session, button, home)

        self.assertEqual(pushed, [("home", False), ("menu", True)])
        self.assertEqual(session.active_design_id, "menu")
        self.assertEqual(server._studio["screens"]["a"]["designId"], "home")

    async def test_batch_assignment_moves_multiple_screens(self):
        layout = default_layout()
        server._studio = {
            "schemaVersion": 1,
            "designs": {
                "one": {"id": "one", "name": "One", "revision": 0, "layout": layout},
                "two": {"id": "two", "name": "Two", "revision": 0, "layout": layout},
            },
            "screens": {
                "a": {"id": "a", "name": "A", "designId": "one", "lastSeen": ""},
                "b": {"id": "b", "name": "B", "designId": "one", "lastSeen": ""},
            },
        }

        class Request:
            match_info = {"design_id": "two"}
            async def json(self):
                return {"screenIds": ["a", "b"]}

        with patch.object(server, "save_studio_atomic"):
            result = json.loads((await server.api_assign_design(Request())).text)
        self.assertEqual(result["assigned"], 2)
        self.assertEqual(server._studio["screens"]["a"]["designId"], "two")
        self.assertEqual(server._studio["screens"]["b"]["designId"], "two")

    async def test_restart_api_sends_device_command(self):
        layout = default_layout()
        server._studio = {
            "schemaVersion": 1,
            "designs": {"default": {"id": "default", "name": "Default", "revision": 0, "layout": layout}},
            "screens": {"a": {"id": "a", "name": "Kitchen", "designId": "default", "lastSeen": ""}},
        }

        class WebSocket:
            def __init__(self): self.messages = []
            async def send(self, message): self.messages.append(message)

        websocket = WebSocket()
        server._devices = {"a": server.DeviceSession("a", websocket, "remote")}
        request = SimpleNamespace(match_info={"screen_id": "a"})
        response = await server.api_restart_screen(request)
        self.assertTrue(json.loads(response.text)["ok"])
        self.assertEqual(websocket.messages, [DeviceCommand(DeviceCommandType.RESTART).pack()])


if __name__ == "__main__":
    unittest.main()

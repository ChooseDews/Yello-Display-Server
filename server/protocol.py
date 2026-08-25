"""Wire protocol shared with firmware/main/protocol.h — keep both in sync by hand."""

import struct
from dataclasses import dataclass
from enum import IntEnum

PROTO_MAGIC = 0x59  # 'Y'
PROTO_VERSION = 1


class MsgType(IntEnum):
    ZONE_UPDATE = 1  # server -> device
    TOUCH_EVENT = 2  # device -> server
    DEVICE_CONFIG = 3  # server -> device
    DEVICE_STATUS = 4  # device -> server heartbeat
    DEVICE_COMMAND = 5  # server -> device


class PixelFormat(IntEnum):
    RAW_RGB565 = 0  # implemented
    RLE_RGB565 = 1  # reserved, not implemented yet


class TouchType(IntEnum):
    DOWN = 0
    MOVE = 1
    UP = 2


class Orientation(IntEnum):
    PORTRAIT = 0
    LANDSCAPE = 1


class DeviceCommandType(IntEnum):
    RESTART = 1


# magic, version, msg_type, format, x, y, w, h, payload_len
_ZONE_HEADER = struct.Struct("<BBBBHHHHI")
assert _ZONE_HEADER.size == 16

# magic, version, msg_type, touch_type, x, y, timestamp_ms
_TOUCH_EVENT = struct.Struct("<BBBBHHI")
assert _TOUCH_EVENT.size == 12

# magic, version, msg_type, orientation, logical_width, logical_height
_DEVICE_CONFIG = struct.Struct("<BBBBHH")
assert _DEVICE_CONFIG.size == 8

# magic, version, msg_type, flags, uptime_ms, wifi_rssi_dbm, reserved
_DEVICE_STATUS = struct.Struct("<BBBBIhH")
assert _DEVICE_STATUS.size == 12

# magic, version, msg_type, command
_DEVICE_COMMAND = struct.Struct("<BBBB")
assert _DEVICE_COMMAND.size == 4


@dataclass
class ZoneUpdate:
    x: int
    y: int
    w: int
    h: int
    pixels: bytes  # row-major RGB565, BIG-endian (ILI9341-native), len == w*h*2 for RAW_RGB565
    format: PixelFormat = PixelFormat.RAW_RGB565

    def pack(self) -> bytes:
        header = _ZONE_HEADER.pack(
            PROTO_MAGIC, PROTO_VERSION, MsgType.ZONE_UPDATE,
            self.format, self.x, self.y, self.w, self.h, len(self.pixels),
        )
        return header + self.pixels


@dataclass
class TouchEvent:
    touch_type: TouchType
    x: int
    y: int
    timestamp_ms: int

    @classmethod
    def unpack(cls, data: bytes) -> "TouchEvent":
        if len(data) != _TOUCH_EVENT.size:
            raise ValueError(f"expected {_TOUCH_EVENT.size} bytes, got {len(data)}")
        magic, version, msg_type, touch_type, x, y, timestamp_ms = _TOUCH_EVENT.unpack(data)
        if magic != PROTO_MAGIC:
            raise ValueError(f"bad magic byte: {magic:#x}")
        if version != PROTO_VERSION:
            raise ValueError(f"unsupported protocol version: {version}")
        if msg_type != MsgType.TOUCH_EVENT:
            raise ValueError(f"unexpected msg_type: {msg_type}")
        return cls(TouchType(touch_type), x, y, timestamp_ms)


@dataclass
class DeviceConfig:
    orientation: Orientation
    width: int
    height: int

    def pack(self) -> bytes:
        return _DEVICE_CONFIG.pack(
            PROTO_MAGIC, PROTO_VERSION, MsgType.DEVICE_CONFIG,
            self.orientation, self.width, self.height,
        )


@dataclass
class DeviceStatus:
    uptime_ms: int
    wifi_rssi_dbm: int

    @classmethod
    def unpack(cls, data: bytes) -> "DeviceStatus":
        if len(data) != _DEVICE_STATUS.size:
            raise ValueError(f"expected {_DEVICE_STATUS.size} bytes, got {len(data)}")
        magic, version, msg_type, _flags, uptime_ms, wifi_rssi_dbm, _reserved = _DEVICE_STATUS.unpack(data)
        if magic != PROTO_MAGIC or version != PROTO_VERSION or msg_type != MsgType.DEVICE_STATUS:
            raise ValueError("invalid device status header")
        return cls(uptime_ms=uptime_ms, wifi_rssi_dbm=wifi_rssi_dbm)


@dataclass
class DeviceCommand:
    command: DeviceCommandType

    def pack(self) -> bytes:
        return _DEVICE_COMMAND.pack(PROTO_MAGIC, PROTO_VERSION, MsgType.DEVICE_COMMAND, self.command)

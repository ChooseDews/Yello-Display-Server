//! Wire protocol shared with firmware/main/protocol.h - keep both in sync by hand.

/// Magic byte: 'Y'
pub const PROTO_MAGIC: u8 = 0x59;
/// Protocol version
pub const PROTO_VERSION: u8 = 1;

/// Message types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MsgType {
    ZoneUpdate = 1,      // server -> device
    TouchEvent = 2,      // device -> server
    DeviceConfig = 3,    // server -> device
    DeviceStatus = 4,    // device -> server heartbeat
    DeviceCommand = 5,   // server -> device
}

/// Pixel formats
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    RawRgb565 = 0,  // implemented
    RleRgb565 = 1,  // reserved
}

/// Touch event types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TouchType {
    Down = 0,
    Move = 1,
    Up = 2,
}

/// Display orientation
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orientation {
    Portrait = 0,
    Landscape = 1,
}

/// Device commands
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceCommandType {
    Restart = 1,
}

/// Zone update: server -> device
#[derive(Debug, Clone)]
pub struct ZoneUpdate {
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
    pub pixels: Vec<u8>,
    pub format: PixelFormat,
}

impl ZoneUpdate {
    pub fn new(x: u16, y: u16, w: u16, h: u16, pixels: Vec<u8>) -> Self {
        Self { x, y, w, h, pixels, format: PixelFormat::RawRgb565 }
    }

    /// Pack into wire format (16-byte header + RGB565 payload)
    pub fn pack(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(16 + self.pixels.len());
        buf.push(PROTO_MAGIC);
        buf.push(PROTO_VERSION);
        buf.push(MsgType::ZoneUpdate as u8);
        buf.push(self.format as u8);
        buf.extend_from_slice(&self.x.to_le_bytes());
        buf.extend_from_slice(&self.y.to_le_bytes());
        buf.extend_from_slice(&self.w.to_le_bytes());
        buf.extend_from_slice(&self.h.to_le_bytes());
        buf.extend_from_slice(&(self.pixels.len() as u32).to_le_bytes());
        buf.extend_from_slice(&self.pixels);
        buf
    }
}

/// Touch event: device -> server
#[derive(Debug, Clone)]
pub struct TouchEvent {
    pub touch_type: TouchType,
    pub x: u16,
    pub y: u16,
    pub timestamp_ms: u32,
}

impl TouchEvent {
    pub fn unpack(data: &[u8]) -> Result<Self, String> {
        if data.len() < 12 {
            return Err(format!("expected 12 bytes, got {}", data.len()));
        }
        if data[0] != PROTO_MAGIC {
            return Err(format!("bad magic byte: {:#x}", data[0]));
        }
        if data[1] != PROTO_VERSION {
            return Err(format!("unsupported protocol version: {}", data[1]));
        }
        if data[2] != MsgType::TouchEvent as u8 {
            return Err(format!("unexpected msg_type: {}", data[2]));
        }
        let touch_type = data[3];
        let x = u16::from_le_bytes([data[4], data[5]]);
        let y = u16::from_le_bytes([data[6], data[7]]);
        let timestamp_ms = u32::from_le_bytes([data[8], data[9], data[10], data[11]]);
        Ok(Self {
            touch_type: match touch_type {
                0 => TouchType::Down,
                1 => TouchType::Move,
                _ => TouchType::Up,
            },
            x, y, timestamp_ms,
        })
    }
}

/// Device config: server -> device
#[derive(Debug, Clone, Copy)]
pub struct DeviceConfig {
    pub orientation: Orientation,
    pub width: u16,
    pub height: u16,
}

impl DeviceConfig {
    pub fn pack(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(8);
        buf.push(PROTO_MAGIC);
        buf.push(PROTO_VERSION);
        buf.push(MsgType::DeviceConfig as u8);
        buf.push(self.orientation as u8);
        buf.extend_from_slice(&self.width.to_le_bytes());
        buf.extend_from_slice(&self.height.to_le_bytes());
        buf
    }
}

/// Device status: device -> server heartbeat
#[derive(Debug, Clone, Copy)]
pub struct DeviceStatus {
    pub uptime_ms: u32,
    pub wifi_rssi_dbm: i16,
}

impl DeviceStatus {
    pub fn unpack(data: &[u8]) -> Result<Self, String> {
        if data.len() < 12 {
            return Err(format!("expected 12 bytes, got {}", data.len()));
        }
        if data[0] != PROTO_MAGIC || data[1] != PROTO_VERSION || data[2] != MsgType::DeviceStatus as u8 {
            return Err("invalid device status header".into());
        }
        let _flags = data[3];
        let uptime_ms = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
        let wifi_rssi_dbm = i16::from_le_bytes([data[8], data[9]]);
        // reserved = data[10..12]
        Ok(Self { uptime_ms, wifi_rssi_dbm })
    }
}

/// Device command: server -> device
#[derive(Debug, Clone, Copy)]
pub struct DeviceCommand {
    pub command: DeviceCommandType,
}

impl DeviceCommand {
    pub fn pack(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(4);
        buf.push(PROTO_MAGIC);
        buf.push(PROTO_VERSION);
        buf.push(MsgType::DeviceCommand as u8);
        buf.push(self.command as u8);
        buf
    }
}

/// Convert an RGB888 image row slice to big-endian RGB565 bytes.
/// Input: rows of [R, G, B, R, G, B, ...] (width*3 bytes per row)
/// Output: row-major RGB565 big-endian (ILI9341-native)
pub fn rgb888_to_rgb565_be(pixels: &[u8], width: usize, height: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(width * height * 2);
    for y in 0..height {
        for x in 0..width {
            let base = (y * width + x) * 3;
            let r = pixels[base] as u16;
            let g = pixels[base + 1] as u16;
            let b = pixels[base + 2] as u16;
            let value = ((r & 0xF8) << 8) | ((g & 0xFC) << 3) | (b >> 3);
            out.extend_from_slice(&value.to_be_bytes());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_zone_update_pack_size() {
        let z = ZoneUpdate::new(0, 0, 10, 10, vec![0u8; 200]);
        let packed = z.pack();
        assert_eq!(packed.len(), 16 + 200);
        assert_eq!(packed[0], PROTO_MAGIC);
        assert_eq!(packed[1], PROTO_VERSION);
        assert_eq!(packed[2], MsgType::ZoneUpdate as u8);
    }

    #[test]
    fn test_touch_event_unpack() {
        let mut data = vec![PROTO_MAGIC, PROTO_VERSION, MsgType::TouchEvent as u8, TouchType::Down as u8];
        data.extend_from_slice(&100u16.to_le_bytes());
        data.extend_from_slice(&200u16.to_le_bytes());
        data.extend_from_slice(&12345u32.to_le_bytes());
        let event = TouchEvent::unpack(&data).unwrap();
        assert_eq!(event.touch_type, TouchType::Down);
        assert_eq!(event.x, 100);
        assert_eq!(event.y, 200);
        assert_eq!(event.timestamp_ms, 12345);
    }

    #[test]
    fn test_device_config_pack() {
        let cfg = DeviceConfig { orientation: Orientation::Landscape, width: 320, height: 240 };
        let packed = cfg.pack();
        assert_eq!(packed.len(), 8);
        assert_eq!(packed[0], PROTO_MAGIC);
        assert_eq!(packed[2], MsgType::DeviceConfig as u8);
        assert_eq!(packed[4], 320u16.to_le_bytes()[0]);
        assert_eq!(packed[6], 240u16.to_le_bytes()[0]);
    }
}

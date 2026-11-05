//! Wire protocol v2, shared with firmware/main/protocol.h - keep both in sync by hand.
//!
//! v2 (breaking, Nov 2026): the per-zone `ZoneUpdate` messages are gone.
//! Each frame push is ONE `FramePatch` WebSocket binary message bundling
//! every dirty zone record, DEFLATE-compressed as a single zlib stream
//! (windowBits = 12, so the ESP32 decoder only needs a 4 KiB dictionary)
//! with an automatic raw fallback when compression does not pay. The
//! device answers a dropped/corrupt patch with `ResyncRequest` and the
//! server re-sends a full frame.
//!
//! Versioning / capability negotiation (Nov 2026, still v2): on connect
//! the device sends `DeviceHello` carrying its firmware version
//! (major/minor/patch), the protocol version it speaks, and a capability
//! bitmask. The server records all three on the session and gates
//! per-device behaviour on the capabilities — most importantly it only
//! sends DEFLATE-compressed FramePatch payloads to devices that set
//! `CAP_DEFLATE`; everyone else (older v2 builds, a future no-DEFLATE
//! variant, or a device that has not said hello yet) gets the raw
//! bundle, which every v2 firmware already accepts. The same three
//! values also ride the WebSocket URL as `fw` / `proto` / `caps` query
//! params so the server knows them before the first frame is pushed;
//! the Hello message is authoritative and overrides the URL hint.

use std::io::Read;

/// Magic byte: 'Y'
pub const PROTO_MAGIC: u8 = 0x59;
/// Protocol version
pub const PROTO_VERSION: u8 = 2;

/// FramePatch header size in bytes.
pub const FRAME_PATCH_HEADER_BYTES: usize = 20;
/// Zone record header size in bytes (x, y, w, h as u16 LE).
pub const ZONE_RECORD_HEADER_BYTES: usize = 8;
/// zlib window bits used for FramePatch compression. 12 => 4 KiB window:
/// the client inflate dictionary is 4 KiB instead of 32 KiB, at a measured
/// cost of a few bytes on UI frames (~+1% on photos).
pub const PATCH_WINDOW_BITS: u8 = 12;
/// DEFLATE level for FramePatch compression (9 buys ~2% on UI frames for
/// ~5x the encode time; 6 is the sweet spot measured Nov 2, 2026).
pub const PATCH_COMPRESSION_LEVEL: u32 = 6;
/// Send the raw bundle instead when the compressed payload is at least
/// this fraction of the raw bundle (incompressible frames can expand).
pub const PATCH_RAW_FALLBACK_RATIO: f64 = 0.95;

/// FramePatch flags (header byte 3).
pub const PATCH_FLAG_COMPRESSED: u8 = 0x01;
pub const PATCH_FLAG_FULL_FRAME: u8 = 0x02;

/// Message types
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MsgType {
    TouchEvent = 2,    // device -> server
    DeviceConfig = 3,  // server -> device
    DeviceStatus = 4,  // device -> server heartbeat
    DeviceCommand = 5, // server -> device
    FramePatch = 6,    // server -> device
    ResyncRequest = 7, // device -> server
    DeviceHello = 8,   // device -> server, first message after connect
}

/// Device capability bits (`DeviceHello::capabilities`, also the `caps`
/// WebSocket URL param). A device sets exactly the bits it implements;
/// the server must never send a feature whose bit is clear.
pub const CAP_RAW_BUNDLE: u32 = 1 << 0; // raw (uncompressed) FramePatch bundle — every v2 device
pub const CAP_DEFLATE: u32 = 1 << 1; // DEFLATE/zlib FramePatch payloads (tinfl, 4 KiB window)
pub const CAP_FRAME_PATCH: u32 = 1 << 2; // FramePatch message itself (vs the removed v1 ZoneUpdate)

/// Capabilities of a fully current v2 firmware build.
pub const CAPS_CURRENT: u32 = CAP_RAW_BUNDLE | CAP_DEFLATE | CAP_FRAME_PATCH;

/// Fixed size of a DeviceHello packet in bytes.
pub const DEVICE_HELLO_BYTES: usize = 20;

/// Why a device asked for a full-frame resync.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResyncReason {
    SeqGap = 1,
    InflateFailed = 2,
    AdlerMismatch = 3,
    Malformed = 4,
    QueueOverflow = 5,
}

impl ResyncReason {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            1 => Some(Self::SeqGap),
            2 => Some(Self::InflateFailed),
            3 => Some(Self::AdlerMismatch),
            4 => Some(Self::Malformed),
            5 => Some(Self::QueueOverflow),
            _ => None,
        }
    }
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

/// One dirty rectangle inside a FramePatch bundle. `pixels` is
/// `w * h * 2` bytes of big-endian RGB565, row-major.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZoneRecord {
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
    pub pixels: Vec<u8>,
}

impl ZoneRecord {
    pub fn new(x: u16, y: u16, w: u16, h: u16, pixels: Vec<u8>) -> Self {
        debug_assert_eq!(pixels.len(), w as usize * h as usize * 2);
        Self { x, y, w, h, pixels }
    }

    /// Bytes this record occupies in the uncompressed bundle.
    pub fn bundle_len(&self) -> usize {
        ZONE_RECORD_HEADER_BYTES + self.pixels.len()
    }
}

/// Serialize zone records into the uncompressed bundle byte sequence:
/// per record, x/y/w/h (u16 LE) followed by the BE RGB565 pixels.
pub fn build_raw_bundle(zones: &[ZoneRecord]) -> Vec<u8> {
    let total: usize = zones.iter().map(|z| z.bundle_len()).sum();
    let mut buf = Vec::with_capacity(total);
    for z in zones {
        buf.extend_from_slice(&z.x.to_le_bytes());
        buf.extend_from_slice(&z.y.to_le_bytes());
        buf.extend_from_slice(&z.w.to_le_bytes());
        buf.extend_from_slice(&z.h.to_le_bytes());
        buf.extend_from_slice(&z.pixels);
    }
    buf
}

/// Parse an uncompressed bundle back into zone records. Used by tests and
/// tooling; the firmware parses the same layout incrementally.
pub fn parse_raw_bundle(mut data: &[u8], zone_count: usize) -> Result<Vec<ZoneRecord>, String> {
    let mut zones = Vec::with_capacity(zone_count);
    for _ in 0..zone_count {
        if data.len() < ZONE_RECORD_HEADER_BYTES {
            return Err("truncated zone record header".into());
        }
        let x = u16::from_le_bytes([data[0], data[1]]);
        let y = u16::from_le_bytes([data[2], data[3]]);
        let w = u16::from_le_bytes([data[4], data[5]]);
        let h = u16::from_le_bytes([data[6], data[7]]);
        data = &data[ZONE_RECORD_HEADER_BYTES..];
        let pixel_len = w as usize * h as usize * 2;
        if data.len() < pixel_len {
            return Err("truncated zone record pixels".into());
        }
        zones.push(ZoneRecord { x, y, w, h, pixels: data[..pixel_len].to_vec() });
        data = &data[pixel_len..];
    }
    if !data.is_empty() {
        return Err(format!("{} trailing bytes after {} zone records", data.len(), zone_count));
    }
    Ok(zones)
}

/// Compress a raw bundle as one zlib stream with a 4 KiB window.
///
/// Uses flate2's `zlib-rs` backend on purpose: the default `rust_backend`
/// (miniz_oxide) silently IGNORES the window_bits argument in flate2
/// 1.1.x (`ffi/miniz_oxide.rs: Deflate::make(.., _window_bits)`), so it
/// would emit a 32 KiB-window stream that the client's 4 KiB dictionary
/// rightly rejects. zlib-rs honours window_bits and is still pure Rust.
pub fn compress_bundle(raw: &[u8]) -> Result<Vec<u8>, String> {
    use flate2::{Compress, FlushCompress, Status};
    let mut engine = Compress::new_with_window_bits(
        flate2::Compression::new(PATCH_COMPRESSION_LEVEL),
        true, // zlib wrapper (header + Adler-32 trailer)
        PATCH_WINDOW_BITS,
    );
    let mut out = Vec::with_capacity(raw.len() / 4 + 64);
    let mut consumed = 0usize;
    loop {
        out.reserve(raw.len() / 2 + 1024);
        let before_in = engine.total_in();
        let before_out = engine.total_out();
        let status = engine
            .compress_vec(&raw[consumed..], &mut out, FlushCompress::Finish)
            .map_err(|e| e.to_string())?;
        consumed += (engine.total_in() - before_in) as usize;
        let _ = before_out;
        match status {
            Status::StreamEnd => break,
            Status::Ok | Status::BufError => {
                if consumed >= raw.len() && engine.total_out() == before_out {
                    // No progress possible with a fresh reserve is a bug, not
                    // a transient state; bail instead of spinning.
                    return Err("compressor made no progress".into());
                }
            }
        }
    }
    Ok(out)
}

/// Decompress a FramePatch payload. The decoder is capped at a 4 KiB
/// window through the zlib header check in `flate2`'s zlib-rs inflate
/// only when the stream itself declares it; callers that need the strict
/// client-side guarantee should check the CMF byte (see tests).
pub fn decompress_bundle(payload: &[u8], uncompressed_len: usize) -> Result<Vec<u8>, String> {
    let mut decoder = flate2::read::ZlibDecoder::new(payload);
    let mut out = Vec::with_capacity(uncompressed_len);
    decoder.read_to_end(&mut out).map_err(|e| e.to_string())?;
    if out.len() != uncompressed_len {
        return Err(format!(
            "decompressed {} bytes, header promised {}",
            out.len(),
            uncompressed_len
        ));
    }
    Ok(out)
}

/// Frame patch: server -> device, one bundled message per push.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FramePatch {
    pub frame_seq: u32,
    pub full_frame: bool,
    pub zones: Vec<ZoneRecord>,
}

impl FramePatch {
    /// Pack into wire format. Compression and the raw fallback are decided
    /// here: returns the full packet plus whether it ended up compressed.
    pub fn pack(&self) -> (Vec<u8>, bool) {
        self.pack_for(true)
    }

    /// Pack for a specific device. `allow_compression` is the device's
    /// negotiated `CAP_DEFLATE` capability: when false the bundle is sent
    /// raw unconditionally (the compressed flag stays clear), even when
    /// compression would have paid — a device that cannot inflate must
    /// never receive a stream it would have to drop. When true, the
    /// usual raw fallback for incompressible bundles still applies.
    pub fn pack_for(&self, allow_compression: bool) -> (Vec<u8>, bool) {
        let raw = build_raw_bundle(&self.zones);
        let (payload, compressed) = if !allow_compression {
            (raw.clone(), false)
        } else {
            match compress_bundle(&raw) {
                Ok(z) if (z.len() as f64) < PATCH_RAW_FALLBACK_RATIO * (raw.len() as f64) => (z, true),
                _ => (raw.clone(), false),
            }
        };
        let mut flags = 0u8;
        if compressed {
            flags |= PATCH_FLAG_COMPRESSED;
        }
        if self.full_frame {
            flags |= PATCH_FLAG_FULL_FRAME;
        }
        let mut buf = Vec::with_capacity(FRAME_PATCH_HEADER_BYTES + payload.len());
        buf.push(PROTO_MAGIC);
        buf.push(PROTO_VERSION);
        buf.push(MsgType::FramePatch as u8);
        buf.push(flags);
        buf.extend_from_slice(&self.frame_seq.to_le_bytes());
        buf.extend_from_slice(&(self.zones.len() as u16).to_le_bytes());
        buf.extend_from_slice(&0u16.to_le_bytes()); // reserved
        buf.extend_from_slice(&(raw.len() as u32).to_le_bytes());
        buf.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        buf.extend_from_slice(&payload);
        (buf, compressed)
    }

    /// Unpack a wire packet (header check + payload decode + record parse).
    pub fn unpack(data: &[u8]) -> Result<(Self, bool), String> {
        let hdr = FramePatchHeader::parse(data)?;
        let payload = &data[FRAME_PATCH_HEADER_BYTES..];
        if payload.len() != hdr.payload_len as usize {
            return Err(format!(
                "payload_len {} != actual {}",
                hdr.payload_len,
                payload.len()
            ));
        }
        let raw = if hdr.compressed() {
            decompress_bundle(payload, hdr.uncompressed_len as usize)?
        } else {
            if payload.len() != hdr.uncompressed_len as usize {
                return Err("raw payload length != uncompressed_len".into());
            }
            payload.to_vec()
        };
        let zones = parse_raw_bundle(&raw, hdr.zone_count as usize)?;
        Ok((
            Self { frame_seq: hdr.frame_seq, full_frame: hdr.full_frame(), zones },
            hdr.compressed(),
        ))
    }
}

/// Parsed FramePatch header, mirroring `yello_frame_patch_header_t`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FramePatchHeader {
    pub flags: u8,
    pub frame_seq: u32,
    pub zone_count: u16,
    pub uncompressed_len: u32,
    pub payload_len: u32,
}

impl FramePatchHeader {
    pub fn compressed(&self) -> bool {
        self.flags & PATCH_FLAG_COMPRESSED != 0
    }
    pub fn full_frame(&self) -> bool {
        self.flags & PATCH_FLAG_FULL_FRAME != 0
    }
    pub fn parse(data: &[u8]) -> Result<Self, String> {
        if data.len() < FRAME_PATCH_HEADER_BYTES {
            return Err(format!("expected >= {} bytes, got {}", FRAME_PATCH_HEADER_BYTES, data.len()));
        }
        if data[0] != PROTO_MAGIC {
            return Err(format!("bad magic byte: {:#x}", data[0]));
        }
        if data[1] != PROTO_VERSION {
            return Err(format!("unsupported protocol version: {}", data[1]));
        }
        if data[2] != MsgType::FramePatch as u8 {
            return Err(format!("unexpected msg_type: {}", data[2]));
        }
        Ok(Self {
            flags: data[3],
            frame_seq: u32::from_le_bytes([data[4], data[5], data[6], data[7]]),
            zone_count: u16::from_le_bytes([data[8], data[9]]),
            uncompressed_len: u32::from_le_bytes([data[12], data[13], data[14], data[15]]),
            payload_len: u32::from_le_bytes([data[16], data[17], data[18], data[19]]),
        })
    }
}

/// Resync request: device -> server (8 bytes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResyncRequest {
    pub reason: ResyncReason,
    pub last_good_seq: u32,
}

impl ResyncRequest {
    pub fn pack(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(8);
        buf.push(PROTO_MAGIC);
        buf.push(PROTO_VERSION);
        buf.push(MsgType::ResyncRequest as u8);
        buf.push(self.reason as u8);
        buf.extend_from_slice(&self.last_good_seq.to_le_bytes());
        buf
    }
    pub fn unpack(data: &[u8]) -> Result<Self, String> {
        if data.len() < 8 {
            return Err(format!("expected 8 bytes, got {}", data.len()));
        }
        if data[0] != PROTO_MAGIC || data[1] != PROTO_VERSION || data[2] != MsgType::ResyncRequest as u8 {
            return Err("invalid resync request header".into());
        }
        let reason = ResyncReason::from_u8(data[3]).ok_or_else(|| format!("unknown resync reason {}", data[3]))?;
        Ok(Self {
            reason,
            last_good_seq: u32::from_le_bytes([data[4], data[5], data[6], data[7]]),
        })
    }
}

/// Device hello: device -> server, sent once right after the WebSocket
/// connects (20 bytes, fixed layout — see `yello_device_hello_t`).
///
/// This is the versioning handshake. `proto_version` is the wire
/// protocol the firmware speaks (the same value it puts in byte 1 of
/// every packet — unpack deliberately does NOT reject a different one,
/// so a future v3 device can still introduce itself to a v2 server,
/// which then knows exactly what it is talking to instead of silently
/// dropping its frames). `firmware_version()` is the human version
/// shown in Studio. `capabilities` gates server behaviour per device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceHello {
    pub proto_version: u8,
    pub capabilities: u32,
    pub fw_major: u8,
    pub fw_minor: u8,
    pub fw_patch: u8,
    /// Inflate window the device can accept, in bits (12 = 4 KiB) when
    /// `CAP_DEFLATE` is set, else 0.
    pub max_window_bits: u8,
    pub display_width: u16,
    pub display_height: u16,
}

impl DeviceHello {
    pub fn firmware_version(&self) -> String {
        format!("{}.{}.{}", self.fw_major, self.fw_minor, self.fw_patch)
    }

    /// Whether the server may send this device DEFLATE-compressed
    /// FramePatch payloads.
    pub fn supports_compression(&self) -> bool {
        self.capabilities & CAP_DEFLATE != 0
    }

    pub fn pack(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(DEVICE_HELLO_BYTES);
        buf.push(PROTO_MAGIC);
        buf.push(self.proto_version);
        buf.push(MsgType::DeviceHello as u8);
        buf.push(0); // reserved
        buf.extend_from_slice(&self.capabilities.to_le_bytes());
        buf.push(self.fw_major);
        buf.push(self.fw_minor);
        buf.push(self.fw_patch);
        buf.push(self.max_window_bits);
        buf.extend_from_slice(&self.display_width.to_le_bytes());
        buf.extend_from_slice(&self.display_height.to_le_bytes());
        buf.extend_from_slice(&0u32.to_le_bytes()); // reserved
        buf
    }

    pub fn unpack(data: &[u8]) -> Result<Self, String> {
        if data.len() < DEVICE_HELLO_BYTES {
            return Err(format!("expected {} bytes, got {}", DEVICE_HELLO_BYTES, data.len()));
        }
        if data[0] != PROTO_MAGIC {
            return Err(format!("bad magic byte: {:#x}", data[0]));
        }
        if data[2] != MsgType::DeviceHello as u8 {
            return Err(format!("unexpected msg_type: {}", data[2]));
        }
        // NOTE: data[1] (the device's protocol version) is reported, not
        // validated — see the struct docs.
        Ok(Self {
            proto_version: data[1],
            capabilities: u32::from_le_bytes([data[4], data[5], data[6], data[7]]),
            fw_major: data[8],
            fw_minor: data[9],
            fw_patch: data[10],
            max_window_bits: data[11],
            display_width: u16::from_le_bytes([data[12], data[13]]),
            display_height: u16::from_le_bytes([data[14], data[15]]),
        })
    }
}

/// Parse the WebSocket URL hint form of a hello (`fw=2.0.0&proto=2&caps=7`).
/// Every part is optional; anything malformed is ignored rather than
/// failing the connection. Returns (firmware_version, proto_version,
/// capabilities).
pub fn parse_hello_hint(
    fw: Option<&str>,
    proto: Option<&str>,
    caps: Option<&str>,
) -> (Option<String>, Option<u8>, Option<u32>) {
    let fw_version = fw.and_then(|s| {
        let parts: Vec<&str> = s.split('.').collect();
        (parts.len() == 3 && parts.iter().all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit())))
            .then(|| s.to_string())
    });
    let proto_version = proto.and_then(|s| s.parse::<u8>().ok());
    let capabilities = caps.and_then(|s| {
        s.strip_prefix("0x")
            .and_then(|hex| u32::from_str_radix(hex, 16).ok())
            .or_else(|| s.parse::<u32>().ok())
    });
    (fw_version, proto_version, capabilities)
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

    fn ui_like_zones() -> Vec<ZoneRecord> {
        // A mostly-flat full frame with a small text-like disturbance:
        // the shape of a real Studio UI push.
        let mut pixels = vec![0u8; 240 * 320 * 2];
        for i in (0..pixels.len()).step_by(2) {
            pixels[i] = 0x10;
            pixels[i + 1] = 0x20;
        }
        for y in 40..48 {
            for x in 20..120 {
                let off = (y * 240 + x) * 2;
                pixels[off] = (x as u8) ^ (y as u8);
                pixels[off + 1] = 0xFF;
            }
        }
        vec![ZoneRecord::new(0, 0, 240, 320, pixels)]
    }

    #[test]
    fn test_frame_patch_golden_header() {
        let patch = FramePatch {
            frame_seq: 0x0102_0304,
            full_frame: true,
            zones: vec![ZoneRecord::new(1, 2, 3, 2, vec![0xAB; 12])],
        };
        let (packed, _compressed) = patch.pack();
        // Header layout must match firmware/main/protocol.h byte for byte.
        assert_eq!(packed[0], PROTO_MAGIC);
        assert_eq!(packed[1], 2);
        assert_eq!(packed[2], MsgType::FramePatch as u8);
        assert_eq!(packed[4..8], 0x0102_0304u32.to_le_bytes());
        assert_eq!(packed[8..10], 1u16.to_le_bytes()); // zone_count
        assert_eq!(packed[10..12], 0u16.to_le_bytes()); // reserved
        // uncompressed_len = 8-byte record header + 12 pixel bytes
        assert_eq!(packed[12..16], 20u32.to_le_bytes());
        let payload_len = u32::from_le_bytes([packed[16], packed[17], packed[18], packed[19]]);
        assert_eq!(packed.len(), FRAME_PATCH_HEADER_BYTES + payload_len as usize);
    }

    #[test]
    fn test_frame_patch_roundtrip_raw_and_compressed() {
        // Compressible zones travel compressed and round-trip exactly.
        let patch = FramePatch { frame_seq: 7, full_frame: false, zones: ui_like_zones() };
        let raw_len = build_raw_bundle(&patch.zones).len();
        let (packed, compressed) = patch.pack();
        assert!(compressed, "UI-like frame must compress");
        assert!(packed.len() < raw_len / 4, "packed {} vs raw {}", packed.len(), raw_len);
        let (back, was_compressed) = FramePatch::unpack(&packed).unwrap();
        assert!(was_compressed);
        assert_eq!(back, patch);

        // Incompressible zones fall back to a raw bundle.
        let mut rng: u32 = 0x1234_5678;
        let mut noise = vec![0u8; 240 * 40 * 2];
        for b in noise.iter_mut() {
            rng = rng.wrapping_mul(1664525).wrapping_add(1013904223);
            *b = (rng >> 24) as u8;
        }
        let patch = FramePatch {
            frame_seq: 8,
            full_frame: false,
            zones: vec![ZoneRecord::new(0, 0, 240, 40, noise)],
        };
        let (packed, compressed) = patch.pack();
        assert!(!compressed, "noise must use the raw fallback");
        assert_eq!(packed[3] & PATCH_FLAG_COMPRESSED, 0);
        let (back, _) = FramePatch::unpack(&packed).unwrap();
        assert_eq!(back, patch);
    }

    #[test]
    fn test_compressed_stream_declares_4k_window() {
        // CMF byte: low nibble CM=8, high nibble CINFO=log2(window)-8.
        // windowBits=12 => CINFO=4 => CMF=0x48. The firmware's 4 KiB
        // dictionary relies on the encoder never referencing farther
        // back than this header promises.
        let raw = build_raw_bundle(&ui_like_zones());
        let z = compress_bundle(&raw).unwrap();
        assert_eq!(z[0], 0x48, "zlib CMF must declare a 4 KiB window");
        assert_eq!((z[0] as u16 * 256 + z[1] as u16) % 31, 0, "FCHECK");
        let back = decompress_bundle(&z, raw.len()).unwrap();
        assert_eq!(back, raw);
    }

    #[test]
    fn test_bundle_record_golden_bytes() {
        // Pins the zone-record layout against the firmware's independent
        // parser (firmware/host_test uses a Python struct '<HHHH' and a
        // C reference parser in the same order): x, y, w, h as u16 LE,
        // then the BE pixels verbatim.
        let zone = ZoneRecord::new(0x0102, 0x0304, 2, 1, vec![0xAA, 0xBB, 0xCC, 0xDD]);
        let bundle = build_raw_bundle(&[zone]);
        assert_eq!(
            bundle,
            vec![0x02, 0x01, 0x04, 0x03, 0x02, 0x00, 0x01, 0x00, 0xAA, 0xBB, 0xCC, 0xDD]
        );
    }

    #[test]
    fn test_device_hello_roundtrip_and_golden_bytes() {
        let hello = DeviceHello {
            proto_version: 2,
            capabilities: CAPS_CURRENT,
            fw_major: 2,
            fw_minor: 0,
            fw_patch: 0,
            max_window_bits: PATCH_WINDOW_BITS,
            display_width: 240,
            display_height: 320,
        };
        let packed = hello.pack();
        // Layout must match firmware/main/protocol.h byte for byte.
        assert_eq!(packed.len(), DEVICE_HELLO_BYTES);
        assert_eq!(packed[0], PROTO_MAGIC);
        assert_eq!(packed[1], 2);
        assert_eq!(packed[2], MsgType::DeviceHello as u8);
        assert_eq!(packed[4..8], CAPS_CURRENT.to_le_bytes());
        assert_eq!(packed[8..11], [2, 0, 0]);
        assert_eq!(packed[11], PATCH_WINDOW_BITS);
        assert_eq!(packed[12..14], 240u16.to_le_bytes());
        assert_eq!(packed[14..16], 320u16.to_le_bytes());
        let back = DeviceHello::unpack(&packed).unwrap();
        assert_eq!(back, hello);
        assert_eq!(back.firmware_version(), "2.0.0");
        assert!(back.supports_compression());

        // A device WITHOUT the DEFLATE bit must read as not supporting
        // compression, and a hello from a different protocol version is
        // still parseable (it is reported, not rejected).
        let mut legacy = hello.clone();
        legacy.capabilities = CAP_RAW_BUNDLE | CAP_FRAME_PATCH;
        legacy.proto_version = 3;
        let back = DeviceHello::unpack(&legacy.pack()).unwrap();
        assert_eq!(back.proto_version, 3);
        assert!(!back.supports_compression());
    }

    #[test]
    fn test_pack_for_without_compression_capability_is_raw() {
        // The same compressible UI frame that pack() compresses must go
        // out raw, flag clear, when the device did not negotiate DEFLATE.
        let patch = FramePatch { frame_seq: 9, full_frame: true, zones: ui_like_zones() };
        let (packed, compressed) = patch.pack_for(false);
        assert!(!compressed);
        assert_eq!(packed[3] & PATCH_FLAG_COMPRESSED, 0);
        let (back, was_compressed) = FramePatch::unpack(&packed).unwrap();
        assert!(!was_compressed);
        assert_eq!(back, patch);
        // And pack_for(true) behaves exactly like pack().
        let (packed_c, compressed_c) = patch.pack_for(true);
        assert!(compressed_c);
        assert_eq!(packed_c, patch.pack().0);
    }

    #[test]
    fn test_parse_hello_hint() {
        assert_eq!(
            parse_hello_hint(Some("2.0.0"), Some("2"), Some("7")),
            (Some("2.0.0".to_string()), Some(2), Some(7))
        );
        assert_eq!(
            parse_hello_hint(Some("2.0.0"), Some("2"), Some("0x7")),
            (Some("2.0.0".to_string()), Some(2), Some(7))
        );
        // Malformed parts are dropped individually, never fatal.
        assert_eq!(parse_hello_hint(Some("banana"), Some("x"), Some("-1")), (None, None, None));
        assert_eq!(parse_hello_hint(None, None, None), (None, None, None));
    }

    #[test]
    fn test_resync_request_roundtrip() {
        let req = ResyncRequest { reason: ResyncReason::SeqGap, last_good_seq: 41 };
        let packed = req.pack();
        assert_eq!(packed.len(), 8);
        assert_eq!(packed[2], MsgType::ResyncRequest as u8);
        assert_eq!(ResyncRequest::unpack(&packed).unwrap(), req);
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

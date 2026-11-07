//! Bundled-firmware info, semver gating, OTA eligibility, and the
//! device-port `/firmware.bin` peek/classify helpers.

use crate::protocol::CAP_OTA;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub const ESP_APP_DESC_MAGIC: u32 = 0xABCD_5432;

/// Parse the version string from an ESP app image: scan the first
/// 64 KiB for the esp_app_desc magic (LE bytes 32 54 CD AB), then read
/// the NUL-terminated ASCII `version[32]` at magic_offset + 16
/// (magic u32 + secure_version u32 + reserv1[8]).
pub fn parse_app_desc_version(data: &[u8]) -> Option<String> {
    let scan = &data[..data.len().min(64 * 1024)];
    let magic = ESP_APP_DESC_MAGIC.to_le_bytes();
    let pos = scan.windows(4).position(|w| w == magic)?;
    let start = pos + 16;
    if start >= data.len() {
        return None;
    }
    let field = &data[start..(start + 32).min(data.len())];
    let end = field.iter().position(|&b| b == 0).unwrap_or(field.len());
    let s = std::str::from_utf8(&field[..end]).ok()?.trim().to_string();
    if s.is_empty() || !s.chars().all(|c| c.is_ascii_graphic()) {
        return None;
    }
    Some(s)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BundledFirmware {
    pub version: String,
    pub size: u64,
    pub sha256: String,
    pub path: PathBuf,
}

pub fn bundled_firmware_from_bytes(path: PathBuf, bytes: &[u8]) -> Option<BundledFirmware> {
    let version = parse_app_desc_version(bytes)?;
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let sha256: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    Some(BundledFirmware { version, size: bytes.len() as u64, sha256, path })
}

pub fn load_bundled_firmware(static_dir: &Path) -> Option<BundledFirmware> {
    let path = static_dir.join("firmware").join("yello_device.bin");
    let bytes = std::fs::read(&path).ok()?;
    bundled_firmware_from_bytes(path, &bytes)
}

/// Parse "X.Y.Z", tolerating missing parts ("2.1" -> 2.1.0) and a
/// non-numeric suffix on a part ("0-beta" -> 0). Returns None when a
/// present part has no leading digits at all.
pub fn parse_semver(s: &str) -> Option<(u64, u64, u64)> {
    let s = s.trim().trim_start_matches('v');
    if s.is_empty() {
        return None;
    }
    let mut out = [0u64; 3];
    for (i, part) in s.split('.').take(3).enumerate() {
        let digits: String = part.chars().take_while(|c| c.is_ascii_digit()).collect();
        if digits.is_empty() {
            // A part that is purely a suffix ("2.x") is unparseable,
            // unless it is beyond what we were given.
            return None;
        }
        out[i] = digits.parse().ok()?;
    }
    // Require at least the first part to have been present (split always yields one).
    Some((out[0], out[1], out[2]))
}

/// True iff `device` is strictly behind `bundled`. Exact string
/// equality is a fast path to not-behind; unparseable versions are
/// treated as not-behind.
pub fn semver_is_behind(device: &str, bundled: &str) -> bool {
    if device == bundled {
        return false;
    }
    match (parse_semver(device), parse_semver(bundled)) {
        (Some(d), Some(b)) => d < b,
        _ => false,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct OtaAttemptRecord {
    pub target_version: String,
    /// offered | in-progress | succeeded | failed | rolled-back
    pub outcome: String,
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_version: Option<String>,
    #[serde(default)]
    pub rebooting: bool,
}

impl OtaAttemptRecord {
    pub fn is_failed_for(&self, target: &str) -> bool {
        self.target_version == target && (self.outcome == "failed" || self.outcome == "rolled-back")
    }
    pub fn is_pending(&self) -> bool {
        self.outcome == "offered" || self.outcome == "in-progress"
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OtaDecision {
    Offer,
    NotConnected,
    NoOtaCap,
    NoBundled,
    UpToDate,
    SuppressedByRecord,
    AlreadyInFlight,
}

impl OtaDecision {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Offer => "offered",
            Self::NotConnected => "not-connected",
            Self::NoOtaCap => "no-ota-cap",
            Self::NoBundled => "no-bundled-firmware",
            Self::UpToDate => "up-to-date",
            Self::SuppressedByRecord => "suppressed-by-failed-record",
            Self::AlreadyInFlight => "in-progress",
        }
    }
}

/// Pure eligibility decision for an OTA offer. `manual` bypasses the
/// failed/rolled-back suppression only.
#[allow(clippy::too_many_arguments)]
pub fn ota_eligibility(
    connected: bool,
    capabilities: u32,
    device_fw: Option<&str>,
    bundled_version: Option<&str>,
    record: Option<&OtaAttemptRecord>,
    in_flight: bool,
    manual: bool,
) -> OtaDecision {
    if !connected {
        return OtaDecision::NotConnected;
    }
    if in_flight {
        return OtaDecision::AlreadyInFlight;
    }
    if capabilities & CAP_OTA == 0 {
        return OtaDecision::NoOtaCap;
    }
    let Some(bundled) = bundled_version else {
        return OtaDecision::NoBundled;
    };
    let Some(fw) = device_fw else {
        return OtaDecision::UpToDate;
    };
    if !semver_is_behind(fw, bundled) {
        return OtaDecision::UpToDate;
    }
    if !manual {
        if let Some(rec) = record {
            if rec.is_failed_for(bundled) {
                return OtaDecision::SuppressedByRecord;
            }
        }
    }
    OtaDecision::Offer
}

/// Per-session OTA state (live, not persisted).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OtaSessionState {
    pub state: String, // idle | offered | accepted | downloading | verifying | rebooting | failed
    pub target_version: Option<String>,
    pub progress: Option<i64>,
    pub error: Option<String>,
}

impl Default for OtaSessionState {
    fn default() -> Self {
        Self { state: "idle".to_string(), target_version: None, progress: None, error: None }
    }
}

impl OtaSessionState {
    pub fn in_flight(&self) -> bool {
        matches!(self.state.as_str(), "offered" | "accepted" | "downloading" | "verifying" | "rebooting")
    }
    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "state": self.state,
            "targetVersion": self.target_version,
            "progress": self.progress,
            "error": self.error,
            "inFlight": self.in_flight(),
        })
    }
}

// ---- device-port /firmware.bin peek handling ----

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PeekDecision {
    Firmware,
    Passthrough,
    NeedMore,
}

const FIRMWARE_GET: &[u8] = b"GET /firmware.bin";

/// Classify a peeked prefix of a device-port connection. A plain
/// `GET /firmware.bin` without a WebSocket Upgrade header is the OTA
/// download; everything else falls through to the WS handshake.
pub fn classify_device_peek(buf: &[u8]) -> PeekDecision {
    if buf.len() < FIRMWARE_GET.len() {
        if FIRMWARE_GET.starts_with(buf) {
            return PeekDecision::NeedMore;
        }
        return PeekDecision::Passthrough;
    }
    if !buf.starts_with(FIRMWARE_GET) {
        return PeekDecision::Passthrough;
    }
    // Must be a path boundary (space, '?', CR) so /firmware.binx is not matched.
    match buf.get(FIRMWARE_GET.len()) {
        Some(b' ') | Some(b'?') | Some(b'\r') => {}
        Some(_) => return PeekDecision::Passthrough,
        None => return PeekDecision::NeedMore,
    }
    let text = String::from_utf8_lossy(buf).to_lowercase();
    if text.contains("upgrade: websocket") || text.contains("upgrade:websocket") {
        return PeekDecision::Passthrough;
    }
    if text.contains("\r\n\r\n") || buf.len() >= 2048 {
        return PeekDecision::Firmware;
    }
    PeekDecision::NeedMore
}

pub fn firmware_http_header(status: &str, content_length: usize) -> Vec<u8> {
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/octet-stream\r\nContent-Length: {content_length}\r\nConnection: close\r\n\r\n"
    )
    .into_bytes()
}

pub fn build_firmware_response(bytes: Option<&[u8]>) -> Vec<u8> {
    match bytes {
        Some(b) => {
            let mut out = firmware_http_header("200 OK", b.len());
            out.extend_from_slice(b);
            out
        }
        None => firmware_http_header("404 Not Found", 0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{CAP_DEFLATE, CAP_FRAME_PATCH, CAP_RAW_BUNDLE};

    fn synthetic_image(version: &str) -> Vec<u8> {
        let mut buf = vec![0u8; 512];
        // esp_app_desc at offset 32 like the real image.
        buf[32..36].copy_from_slice(&ESP_APP_DESC_MAGIC.to_le_bytes());
        buf[48..48 + version.len()].copy_from_slice(version.as_bytes());
        buf
    }

    #[test]
    fn parse_version_synthetic() {
        assert_eq!(parse_app_desc_version(&synthetic_image("2.2.0")).as_deref(), Some("2.2.0"));
        assert_eq!(parse_app_desc_version(&synthetic_image("2.1.0")).as_deref(), Some("2.1.0"));
        assert_eq!(parse_app_desc_version(&[0u8; 128]), None);
        assert_eq!(parse_app_desc_version(&[]), None);
    }

    #[test]
    fn bundled_from_bytes_sha_and_size() {
        let img = synthetic_image("2.2.0");
        let info = bundled_firmware_from_bytes(PathBuf::from("yello_device.bin"), &img).unwrap();
        assert_eq!(info.version, "2.2.0");
        assert_eq!(info.size, 512);
        assert_eq!(info.sha256.len(), 64);
        assert!(info.sha256.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }

    #[test]
    fn semver_compare() {
        assert!(semver_is_behind("2.0.0", "2.2.0"));
        assert!(semver_is_behind("2.1", "2.2.0"));
        assert!(semver_is_behind("1.9.9", "2.0.0"));
        assert!(!semver_is_behind("2.2.0", "2.2.0"));
        assert!(!semver_is_behind("2.3.0", "2.2.0"));
        assert!(!semver_is_behind("banana", "2.2.0"));
        assert!(!semver_is_behind("2.2.0", "banana"));
        assert_eq!(parse_semver("2.1"), Some((2, 1, 0)));
        assert_eq!(parse_semver("v2.0.0-beta"), Some((2, 0, 0)));
    }

    fn record(outcome: &str, target: &str) -> OtaAttemptRecord {
        OtaAttemptRecord {
            target_version: target.into(),
            outcome: outcome.into(),
            updated_at: "2026-01-01T00:00:00Z".into(),
            from_version: Some("2.0.0".into()),
            rebooting: false,
        }
    }

    #[test]
    fn eligibility_gating() {
        let caps_ota = CAP_RAW_BUNDLE | CAP_DEFLATE | CAP_FRAME_PATCH | CAP_OTA;
        // Happy path
        assert_eq!(ota_eligibility(true, caps_ota, Some("2.0.0"), Some("2.2.0"), None, false, false), OtaDecision::Offer);
        // No CAP_OTA -> no offer
        assert_eq!(ota_eligibility(true, CAPS_CURRENT_FOR_TEST, Some("2.0.0"), Some("2.2.0"), None, false, false), OtaDecision::NoOtaCap);
        // Equal / ahead -> no offer
        assert_eq!(ota_eligibility(true, caps_ota, Some("2.2.0"), Some("2.2.0"), None, false, false), OtaDecision::UpToDate);
        assert_eq!(ota_eligibility(true, caps_ota, Some("2.3.0"), Some("2.2.0"), None, false, false), OtaDecision::UpToDate);
        // Failed record suppresses auto, manual bypasses
        let failed = record("failed", "2.2.0");
        assert_eq!(ota_eligibility(true, caps_ota, Some("2.0.0"), Some("2.2.0"), Some(&failed), false, false), OtaDecision::SuppressedByRecord);
        assert_eq!(ota_eligibility(true, caps_ota, Some("2.0.0"), Some("2.2.0"), Some(&failed), false, true), OtaDecision::Offer);
        let rolled = record("rolled-back", "2.2.0");
        assert_eq!(ota_eligibility(true, caps_ota, Some("2.0.0"), Some("2.2.0"), Some(&rolled), false, false), OtaDecision::SuppressedByRecord);
        // Failed record for a different target does not suppress
        let old_failed = record("failed", "2.1.0");
        assert_eq!(ota_eligibility(true, caps_ota, Some("2.0.0"), Some("2.2.0"), Some(&old_failed), false, false), OtaDecision::Offer);
        // In-flight, not connected, no bundled
        assert_eq!(ota_eligibility(true, caps_ota, Some("2.0.0"), Some("2.2.0"), None, true, false), OtaDecision::AlreadyInFlight);
        assert_eq!(ota_eligibility(false, caps_ota, Some("2.0.0"), Some("2.2.0"), None, false, false), OtaDecision::NotConnected);
        assert_eq!(ota_eligibility(true, caps_ota, Some("2.0.0"), None, None, false, false), OtaDecision::NoBundled);
    }

    const CAPS_CURRENT_FOR_TEST: u32 = CAP_RAW_BUNDLE | CAP_DEFLATE | CAP_FRAME_PATCH;

    #[test]
    fn peek_classify() {
        assert_eq!(classify_device_peek(b"GET /firmware.bin HTTP/1.1\r\nHost: x\r\n\r\n"), PeekDecision::Firmware);
        assert_eq!(classify_device_peek(b"GET /firmware.bin?x=1 HTTP/1.1\r\n\r\n"), PeekDecision::Firmware);
        assert_eq!(classify_device_peek(b"GET / HTTP/1.1\r\nUpgrade: websocket\r\n\r\n"), PeekDecision::Passthrough);
        assert_eq!(classify_device_peek(b"GET /firmware.bin HTTP/1.1\r\nUpgrade: websocket\r\n\r\n"), PeekDecision::Passthrough);
        assert_eq!(classify_device_peek(b"GET /firmware.binx HTTP/1.1\r\n\r\n"), PeekDecision::Passthrough);
        assert_eq!(classify_device_peek(b"GET /firm"), PeekDecision::NeedMore);
        assert_eq!(classify_device_peek(b"GET /firmware.bin HTTP/1.1\r\nHost: x"), PeekDecision::NeedMore);
    }

    #[test]
    fn firmware_response_builder() {
        let ok = build_firmware_response(Some(b"abc"));
        assert!(ok.starts_with(b"HTTP/1.1 200 OK\r\n"));
        assert!(String::from_utf8_lossy(&ok).contains("Content-Length: 3\r\n"));
        assert!(ok.ends_with(b"abc"));
        let nf = build_firmware_response(None);
        assert!(nf.starts_with(b"HTTP/1.1 404 Not Found\r\n"));
        assert!(String::from_utf8_lossy(&nf).contains("Content-Length: 0\r\n"));
    }
}

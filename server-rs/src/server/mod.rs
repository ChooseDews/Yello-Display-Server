//! Main server: WebSocket gateway for ESP32 devices + REST API for web UI.

use crate::frame_diff::dirty_regions;
use crate::home_assistant::HaClient;
use crate::layout_model::*;
use crate::protocol::*;
use crate::renderer;
use crate::safe_http::SafeHttp;
use crate::SCRIPT_EXAMPLES;

use chrono::Utc;
use futures_util::{SinkExt, StreamExt};
// use hyper::service::{make_service_fn, service_fn};
use hyper::{Body, Method, Request, Response, StatusCode};
use image::RgbImage;
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet, VecDeque};
use std::convert::Infallible;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, Mutex, RwLock};
use tokio_tungstenite::tungstenite::handshake::server::{Request as WsRequest, Response as WsResponse};
use tokio_tungstenite::tungstenite::Message as WsMessage;
use url::Url;

pub const MAX_ZONE_AREA_PX: u32 = 480;
pub const ZONE_HEADER_BYTES: usize = 16;
pub const STREAM_STATS_WINDOW_S: f64 = 10.0;
pub const ZONE_PACING_S: f64 = 0.002;
pub const MAX_EXTERNAL_BYTES: usize = 8192;
pub const MAX_IMAGE_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_IMAGE_PIXELS: u64 = 4_000_000;

pub fn wanted_image_sources(layout: &Value) -> HashSet<String> {
    layout
        .get("elements")
        .and_then(|v| v.as_array())
        .map(|elements| {
            elements
                .iter()
                .filter(|e| e.get("type").and_then(|v| v.as_str()) == Some("image"))
                .filter_map(|e| {
                    e.get("props")
                        .and_then(|p| p.get("src"))
                        .and_then(|v| v.as_str())
                        .filter(|s| !s.is_empty())
                        .map(|s| s.to_string())
                })
                .collect()
        })
        .unwrap_or_default()
}

fn in_flight_lock(set: &Arc<std::sync::Mutex<HashSet<String>>>) -> std::sync::MutexGuard<'_, HashSet<String>> {
    set.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}
pub const FULL_RESYNC_INTERVAL_S: f64 = 300.0;
pub const EXTERNAL_FETCH_TIMEOUT_S: u64 = 10;
pub const DOT_LIFETIME_S: f64 = 4.0;
pub const BUTTON_FLASH_S: f64 = 0.25;

lazy_static::lazy_static! {
    static ref DEVICE_ID_RE: Regex = Regex::new(r"^[A-Za-z0-9._:-]{1,64}$").unwrap();
}

#[derive(Debug, Clone)]
pub struct StreamSample {
    pub time: Instant,
    pub pixels: u64,
    pub wire_bytes: u64,
    pub zones: u32,
}

pub struct DeviceSession {
    pub device_id: String,
    pub remote: String,
    pub last_frame: Option<Arc<RgbImage>>,
    pub last_full_push: Instant,
    pub pressed: HashSet<String>,
    pub dots: Vec<(i32, i32, Instant)>,
    pub connected_at: Instant,
    pub connected_since: String,
    pub stream_samples: VecDeque<StreamSample>,
    pub total_pixels: u64,
    pub total_wire_bytes: u64,
    pub total_zone_messages: u64,
    pub frames_sent: u64,
    pub last_push_ms: f64,
    pub last_frame_at: String,
    pub orientation: String,
    pub last_ping_at: String,
    pub wifi_rssi_dbm: Option<i16>,
    pub uptime_ms: u64,
    pub active_design_id: String,
    pub ws_sender: Option<mpsc::UnboundedSender<WsMessage>>,
    /// Serializes frame pushes per device so the global device map is never
    /// held across the (paced) zone send loop.
    pub push_lock: Arc<Mutex<()>>,
    /// Last time the background render tick pushed for this device (gates the
    /// 30 s slow tick for designs without clock/script elements).
    pub last_tick: Instant,
}

/// Mirror of Python's renderer.has_clock: layouts containing a visible clock
/// or script element need a 1 s render tick; others only need 30 s.
fn layout_has_animation(layout: &Value) -> bool {
    layout
        .get("elements")
        .and_then(|v| v.as_array())
        .map(|els| {
            els.iter().any(|e| {
                let t = e.get("type").and_then(|v| v.as_str()).unwrap_or("");
                (t == "clock" || t == "script")
                    && e.get("visible").and_then(|v| v.as_bool()).unwrap_or(true)
            })
        })
        .unwrap_or(false)
}

#[derive(Debug, Clone, Default)]
pub struct LivePreviewState {
    pub pressed: HashSet<String>,
    pub dots: Vec<(i32, i32, Instant)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Design {
    pub id: String,
    pub name: String,
    pub revision: u64,
    pub layout: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Studio {
    pub schema_version: i32,
    pub designs: HashMap<String, Design>,
    pub screens: HashMap<String, Screen>,
}

impl Studio {
    pub fn new(initial_layout: Option<Value>) -> Self {
        let layout = validate_layout(initial_layout.unwrap_or_else(default_layout)).unwrap_or_else(|_| default_layout());
        let name = layout.get("name").and_then(|v| v.as_str()).unwrap_or("Default design").to_string();
        let mut designs = HashMap::new();
        designs.insert(
            DEFAULT_DESIGN_ID.to_string(),
            Design {
                id: DEFAULT_DESIGN_ID.to_string(),
                name,
                revision: 0,
                layout,
            },
        );
        Self {
            schema_version: 1,
            designs,
            screens: HashMap::new(),
        }
    }
}

pub struct AppState {
    pub studio: Arc<RwLock<Studio>>,
    pub devices: Arc<RwLock<HashMap<String, DeviceSession>>>,
    pub http_client: reqwest::Client,
    pub safe_http: Arc<SafeHttp>,
    pub external_values: Arc<RwLock<HashMap<(String, String), String>>>,
    pub external_last_fetch: Arc<RwLock<HashMap<(String, String), Instant>>>,
    pub images: Arc<RwLock<HashMap<String, std::sync::Arc<RgbImage>>>>,
    pub ha_client: Arc<RwLock<HaClient>>,
    pub ha_values: Arc<RwLock<HashMap<String, Value>>>,
    pub ha_last_fetch: Arc<RwLock<HashMap<String, Instant>>>,
    pub live_states: Arc<RwLock<HashMap<String, LivePreviewState>>>,
    pub studio_path: PathBuf,
    pub secrets_path: PathBuf,
    pub static_dir: PathBuf,
}

impl AppState {
    pub fn new(studio_path: PathBuf, secrets_path: PathBuf, static_dir: PathBuf) -> Self {
        Self {
            studio: Arc::new(RwLock::new(Studio::new(None))),
            devices: Arc::new(RwLock::new(HashMap::new())),
            http_client: reqwest::Client::new(),
            safe_http: Arc::new(SafeHttp::new()),
            external_values: Arc::new(RwLock::new(HashMap::new())),
            external_last_fetch: Arc::new(RwLock::new(HashMap::new())),
            images: Arc::new(RwLock::new(HashMap::new())),
            ha_client: Arc::new(RwLock::new(HaClient::new("", ""))),
            ha_values: Arc::new(RwLock::new(HashMap::new())),
            ha_last_fetch: Arc::new(RwLock::new(HashMap::new())),
            live_states: Arc::new(RwLock::new(HashMap::new())),
            studio_path,
            secrets_path,
            static_dir,
        }
    }

    pub async fn load_settings(&self) {
        if let Ok(content) = tokio::fs::read_to_string(&self.secrets_path).await {
            if let Ok(val) = serde_json::from_str::<Value>(&content) {
                if let Some(ha) = val.get("homeAssistant") {
                    let url = ha.get("url").and_then(|v| v.as_str()).unwrap_or("");
                    let token = ha.get("token").and_then(|v| v.as_str()).unwrap_or("");
                    let mut client = self.ha_client.write().await;
                    *client = HaClient::new(url, token);
                }
            }
        }
    }

    pub async fn save_settings(&self, url: &str, token: &str) -> Result<(), String> {
        let val = json!({
            "schemaVersion": 1,
            "homeAssistant": {
                "url": url,
                "token": token
            }
        });
        let content = serde_json::to_string_pretty(&val).map_err(|e| e.to_string())?;
        if let Some(parent) = self.secrets_path.parent() {
            let _ = tokio::fs::create_dir_all(parent).await;
        }
        tokio::fs::write(&self.secrets_path, content)
            .await
            .map_err(|e| e.to_string())?;
        let mut client = self.ha_client.write().await;
        *client = HaClient::new(url, token);
        Ok(())
    }

    pub async fn load_studio_file(&self) {
        match tokio::fs::read_to_string(&self.studio_path).await {
            Ok(content) => match serde_json::from_str::<Value>(&content) {
                Ok(val) if val.get("schemaVersion").and_then(|v| v.as_i64()) == Some(1) => {
                    if let Some(designs_map) = val.get("designs").and_then(|v| v.as_object()) {
                        let mut designs = HashMap::new();
                        for (d_id, d_rec) in designs_map {
                            if !DEVICE_ID_RE.is_match(d_id) {
                                continue;
                            }
                            match validate_layout(d_rec.get("layout").cloned().unwrap_or_else(|| json!({}))) {
                                Ok(layout) => {
                                    let name = d_rec.get("name").and_then(|v| v.as_str()).unwrap_or(d_id).to_string();
                                    let revision = d_rec.get("revision").and_then(|v| v.as_u64()).unwrap_or(0);
                                    designs.insert(d_id.clone(), Design { id: d_id.clone(), name, revision, layout });
                                }
                                Err(e) => {
                                    eprintln!("studio design {d_id}: failed to load layout: {:?}", e.errors);
                                }
                            }
                        }
                        let mut screens = HashMap::new();
                        if let Some(screens_map) = val.get("screens").and_then(|v| v.as_object()) {
                            for (s_id, s_rec) in screens_map {
                                if DEVICE_ID_RE.is_match(s_id) {
                                    let name = s_rec.get("name").and_then(|v| v.as_str()).unwrap_or(s_id).to_string();
                                    let design_id = s_rec.get("designId").and_then(|v| v.as_str()).unwrap_or(DEFAULT_DESIGN_ID).to_string();
                                    let last_seen = s_rec.get("lastSeen").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                    screens.insert(s_id.clone(), Screen { id: s_id.clone(), name, design_id, last_seen });
                                }
                            }
                        }

                        if !designs.is_empty() {
                            let mut st = self.studio.write().await;
                            st.designs = designs;
                            st.screens = screens;
                            return;
                        }
                        eprintln!(
                            "studio file {} contained no valid designs; keeping defaults (file not overwritten)",
                            self.studio_path.display()
                        );
                    }
                }
                Ok(_) => eprintln!("studio file {} has wrong/missing schemaVersion; ignoring", self.studio_path.display()),
                Err(e) => eprintln!("studio file {} is not valid JSON: {e}; ignoring", self.studio_path.display()),
            },
            Err(_) => {
                // File missing entirely — seed it with defaults.
                self.save_studio_file().await;
            }
        }
    }

    pub async fn save_studio_file(&self) {
        let studio = self.studio.read().await;
        if let Ok(content) = serde_json::to_string_pretty(&*studio) {
            if let Some(parent) = self.studio_path.parent() {
                let _ = tokio::fs::create_dir_all(parent).await;
            }
            let _ = tokio::fs::write(&self.studio_path, content).await;
        }
    }
}

pub fn design_id_for_session(studio: &Studio, session: &DeviceSession) -> String {
    if studio.designs.contains_key(&session.active_design_id) {
        return session.active_design_id.clone();
    }
    if let Some(screen) = studio.screens.get(&session.device_id) {
        if studio.designs.contains_key(&screen.design_id) {
            return screen.design_id.clone();
        }
    }
    DEFAULT_DESIGN_ID.to_string()
}

pub fn build_render_context(
    state_external: &HashMap<(String, String), String>,
    design_id: &str,
    state_ha: &HashMap<String, Value>,
    pressed: &HashSet<String>,
    dots: &[(i32, i32, Instant)],
) -> Value {
    let mut external_map = serde_json::Map::new();
    for ((d_id, s_id), val) in state_external {
        if d_id == design_id {
            external_map.insert(s_id.clone(), json!(val));
        }
    }

    let mut dots_arr = Vec::new();
    let now = Instant::now();
    for (x, y, expiry) in dots {
        if *expiry > now {
            dots_arr.push(json!([x, y]));
        }
    }

    let pressed_arr: Vec<Value> = pressed.iter().map(|s| json!(s)).collect();

    json!({
        "external": Value::Object(external_map),
        "homeAssistant": state_ha,
        "pressed": Value::Array(pressed_arr),
        "dots": Value::Array(dots_arr)
    })
}

pub async fn push_frame(state: &Arc<AppState>, device_id: &str, full: bool) -> Result<(), String> {
    // Phase 0: take the per-device push lock. Never wait for it while holding
    // a map lock — other tasks need short snapshot/update accesses.
    let push_lock = {
        let devices = state.devices.read().await;
        match devices.get(device_id) {
            Some(s) => s.push_lock.clone(),
            None => return Err("device not found".to_string()),
        }
    };
    let _guard = push_lock.lock().await;

    // Phase 1: snapshot session state. The global device map is locked only
    // for these clones — never across the render or the paced zone send.
    let (ws_sender, design_id, orientation, prev_frame, pressed, dots, last_full_push) = {
        let studio = state.studio.read().await;
        let mut devices = state.devices.write().await;
        let session = match devices.get_mut(device_id) {
            Some(s) => s,
            None => return Err("device not found".to_string()),
        };
        let design_id = design_id_for_session(&studio, session);
        let now = Instant::now();
        session.dots.retain(|(_, _, exp)| *exp > now);
        (
            session.ws_sender.clone(),
            design_id,
            session.orientation.clone(),
            session.last_frame.clone(),
            session.pressed.clone(),
            session.dots.clone(),
            session.last_full_push,
        )
    };

    let ws_sender = match ws_sender {
        Some(s) => s,
        None => return Ok(()),
    };

    let layout = {
        let studio = state.studio.read().await;
        match studio.designs.get(&design_id) {
            Some(d) => d.layout.clone(),
            None => return Err("design not found".to_string()),
        }
    };

    // Phase 2: lock-free render, region diff, encode and paced send.
    let mut full = full;
    let layout_orientation = layout.get("orientation").and_then(|v| v.as_str()).unwrap_or("portrait").to_string();
    if orientation != layout_orientation {
        let (w, h) = layout_dimensions(&layout);
        let config = DeviceConfig {
            orientation: if layout_orientation == "landscape" { crate::protocol::Orientation::Landscape } else { crate::protocol::Orientation::Portrait },
            width: w as u16,
            height: h as u16,
        };
        let _ = ws_sender.send(WsMessage::Binary(config.pack()));
        full = true;
    }

    let push_started = Instant::now();
    let ext_vals = state.external_values.read().await.clone();
    let ha_vals = state.ha_values.read().await.clone();
    let ctx = build_render_context(&ext_vals, &design_id, &ha_vals, &pressed, &dots);
    let images = state.images.read().await.clone();
    let frame = renderer::render(&layout, &ctx, &images);

    let now = Instant::now();
    let should_send_full = full
        || prev_frame.is_none()
        || now.duration_since(last_full_push).as_secs_f64() >= FULL_RESYNC_INTERVAL_S;

    let regions = if should_send_full {
        vec![(0u32, 0u32, frame.width(), frame.height())]
    } else if let Some(ref prev) = prev_frame {
        dirty_regions(&frame, prev, Some(MAX_ZONE_AREA_PX), None, None, None)
    } else {
        vec![(0u32, 0u32, frame.width(), frame.height())]
    };

    if regions.is_empty() {
        return Ok(());
    }

    let mut pixels_sent = 0u64;
    let mut zones_sent = 0u32;
    let raw = frame.as_raw();
    let frame_w = frame.width() as usize;

    for (x, y, w, h) in &regions {
        let rows_per_chunk = (MAX_ZONE_AREA_PX / w).max(1);
        let mut current_y = *y;
        while current_y < y + h {
            let chunk_height = rows_per_chunk.min(y + h - current_y);

            // Single pass over the zone's rows: RGB888 -> big-endian RGB565.
            let mut pixels_rgb565 = Vec::with_capacity((w * chunk_height * 2) as usize);
            pixels_rgb565.resize((w * chunk_height * 2) as usize, 0);
            for row in current_y..(current_y + chunk_height) {
                let src = &raw[(row as usize * frame_w + *x as usize) * 3..];
                let row_start = ((row - current_y) as usize) * (*w as usize) * 2;
                for col in 0..(*w as usize) {
                    let r = src[col * 3] as u16;
                    let g = src[col * 3 + 1] as u16;
                    let b = src[col * 3 + 2] as u16;
                    let val = ((r & 0xF8) << 8) | ((g & 0xFC) << 3) | (b >> 3);
                    pixels_rgb565[row_start + col * 2] = (val >> 8) as u8;
                    pixels_rgb565[row_start + col * 2 + 1] = (val & 0xFF) as u8;
                }
            }

            let zone = ZoneUpdate::new(*x as u16, current_y as u16, *w as u16, chunk_height as u16, pixels_rgb565);
            let packet = zone.pack();
            let _ = ws_sender.send(WsMessage::Binary(packet));

            pixels_sent += (*w as u64) * (chunk_height as u64);
            zones_sent += 1;
            current_y += chunk_height;

            tokio::time::sleep(Duration::from_secs_f64(ZONE_PACING_S)).await;
        }
    }

    // Phase 3: short-lived write-back of frame and stream stats.
    let now = Instant::now();
    let wire_bytes = pixels_sent * 2 + (zones_sent as u64) * (ZONE_HEADER_BYTES as u64);
    let push_ms = push_started.elapsed().as_secs_f64() * 1000.0;
    {
        let mut devices = state.devices.write().await;
        if let Some(session) = devices.get_mut(device_id) {
            session.last_frame = Some(Arc::new(frame));
            if should_send_full {
                session.last_full_push = now;
            }
            session.orientation = layout_orientation;
            session.stream_samples.push_back(StreamSample {
                time: now,
                pixels: pixels_sent,
                wire_bytes,
                zones: zones_sent,
            });
            while let Some(front) = session.stream_samples.front() {
                if now.duration_since(front.time).as_secs_f64() > STREAM_STATS_WINDOW_S {
                    session.stream_samples.pop_front();
                } else {
                    break;
                }
            }
            session.total_pixels += pixels_sent;
            session.total_wire_bytes += wire_bytes;
            session.total_zone_messages += zones_sent as u64;
            session.frames_sent += 1;
            session.last_push_ms = push_ms;
            session.last_frame_at = Utc::now().to_rfc3339();
        }
    }

    Ok(())
}


pub async fn push_design(state: &Arc<AppState>, design_id: &str, full: bool) {
    let device_ids: Vec<String> = {
        let studio = state.studio.read().await;
        let devices = state.devices.read().await;
        devices
            .iter()
            .filter(|(_, session)| design_id_for_session(&studio, session) == design_id)
            .map(|(id, _)| id.clone())
            .collect()
    };

    for dev_id in device_ids {
        let st = state.clone();
        tokio::spawn(async move {
            let _ = push_frame(&st, &dev_id, full).await;
        });
    }
}

// Background loops
pub fn start_background_tasks(state: Arc<AppState>) {
    // 1-second render tick loop
    let st_render = state.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let device_ids: Vec<String> = {
                let devices = st_render.devices.read().await;
                devices.keys().cloned().collect()
            };
            let now = Instant::now();
            for dev_id in device_ids {
                // Designs with a visible clock/script tick every second;
                // static designs only need a 30 s tick (Python parity).
                let due = {
                    let studio = st_render.studio.read().await;
                    let devices = st_render.devices.read().await;
                    match devices.get(&dev_id) {
                        Some(session) => {
                            let d_id = design_id_for_session(&studio, session);
                            let animated = studio.designs.get(&d_id).map(|d| layout_has_animation(&d.layout)).unwrap_or(true);
                            animated || now.duration_since(session.last_tick).as_secs_f64() >= 30.0
                        }
                        None => false,
                    }
                };
                if !due {
                    continue;
                }
                if let Some(session) = st_render.devices.write().await.get_mut(&dev_id) {
                    session.last_tick = now;
                }
                let st = st_render.clone();
                tokio::spawn(async move {
                    let _ = push_frame(&st, &dev_id, false).await;
                });
            }
        }
    });

    // External data sources loop
    let st_ext = state.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            let studio = st_ext.studio.read().await.clone();
            for (design_id, design) in studio.designs {
                if let Some(sources) = design.layout.get("dataSources").and_then(|v| v.as_array()) {
                    for src in sources {
                        let id = src.get("id").and_then(|v| v.as_str()).unwrap_or("");
                        let url = src.get("url").and_then(|v| v.as_str()).unwrap_or("");
                        let interval = src.get("interval").and_then(|v| v.as_u64()).unwrap_or(60);
                        if url.is_empty() || id.is_empty() { continue; }

                        let key = (design_id.clone(), id.to_string());
                        let should_fetch = {
                            let last = st_ext.external_last_fetch.read().await;
                            match last.get(&key) {
                                None => true,
                                Some(t) => t.elapsed().as_secs() >= interval,
                            }
                        };

                        if should_fetch {
                            st_ext.external_last_fetch.write().await.insert(key.clone(), Instant::now());
                            let client = st_ext.safe_http.clone();
                            let url_s = url.to_string();
                            let ext_map = st_ext.external_values.clone();
                            let ext_state = st_ext.clone();
                            let ext_design_id = key.0.clone();
                            tokio::spawn(async move {
                                if let Ok((bytes, _)) = client.fetch_bytes(&url_s, MAX_EXTERNAL_BYTES).await {
                                    if let Ok(text) = String::from_utf8(bytes.to_vec()) {
                                        let is_new = {
                                            let mut vals = ext_map.write().await;
                                            vals.get(&key) != Some(&text.trim().to_string()) && {
                                                vals.insert(key, text.trim().to_string());
                                                true
                                            }
                                        };
                                        // Python parity: new external values re-push immediately.
                                        if is_new {
                                            push_design(&ext_state, &ext_design_id, false).await;
                                        }
                                    }
                                }
                            });
                        }
                    }
                }
            }
        }
    });

    // Image fetch loop (Python parity: ensure_images + fetch_image)
    let st_img = state.clone();
    tokio::spawn(async move {
        let in_flight: Arc<std::sync::Mutex<HashSet<String>>> = Arc::new(std::sync::Mutex::new(HashSet::new()));
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            let studio = st_img.studio.read().await.clone();
            let mut wanted: HashSet<String> = HashSet::new();
            for (_, design) in &studio.designs {
                for src in wanted_image_sources(&design.layout) {
                    wanted.insert(src);
                }
            }

            // Prune cached images no longer referenced by any design.
            {
                let mut imgs = st_img.images.write().await;
                for url in imgs.keys().cloned().collect::<Vec<_>>() {
                    if !wanted.contains(&url) {
                        imgs.remove(&url);
                    }
                }
            }

            for url in wanted {
                let skip = {
                    let cached = st_img.images.read().await.contains_key(&url);
                    let inflight = in_flight.lock().unwrap().contains(&url);
                    cached || inflight
                };
                if skip { continue; }
                in_flight.lock().unwrap().insert(url.clone());
                let client = st_img.safe_http.clone();
                let img_map = st_img.images.clone();
                let img_state = st_img.clone();
                let inflight = in_flight.clone();
                tokio::spawn(async move {
                    let result = async {
                        let (bytes, _) = client.fetch_bytes(&url, MAX_IMAGE_BYTES).await.map_err(|e| e.to_string())?;
                        let decoded = image::load_from_memory(&bytes).map_err(|e| e.to_string())?;
                        if (decoded.width() as u64) * (decoded.height() as u64) > MAX_IMAGE_PIXELS {
                            return Err("image dimensions are too large".to_string());
                        }
                        Ok(std::sync::Arc::new(decoded.to_rgb8()))
                    }.await;
                    match result {
                        Ok(img) => {
                            img_map.write().await.insert(url.clone(), img);
                            // Python parity: re-push every design using this image.
                            let studio = img_state.studio.read().await;
                            let design_ids: Vec<String> = studio.designs.iter()
                                .filter(|(_, d)| wanted_image_sources(&d.layout).contains(&url))
                                .map(|(id, _)| id.clone())
                                .collect();
                            drop(studio);
                            for design_id in design_ids {
                                push_design(&img_state, &design_id, false).await;
                            }
                        }
                        Err(e) => {
                            eprintln!("image fetch failed for {url}: {e}");
                            in_flight_lock(&inflight).remove(&url);
                        }
                    }
                });
            }
        }
    });

    // Home assistant loop
    let st_ha = state.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            let client = st_ha.ha_client.read().await.clone();
            if !client.configured() {
                continue;
            }

            let entity_specs = {
                let studio = st_ha.studio.read().await;
                let mut specs = HashMap::new();
                for (_, design) in &studio.designs {
                    if let Some(elements) = design.layout.get("elements").and_then(|v| v.as_array()) {
                        for elem in elements {
                            let etype = elem.get("type").and_then(|v| v.as_str()).unwrap_or("");
                            let props = elem.get("props").unwrap_or(&Value::Null);
                            if etype == "ha-state" || etype == "ha-toggle" {
                                let eid = props.get("entityId").and_then(|v| v.as_str()).unwrap_or("");
                                let refresh = props.get("refreshInterval").and_then(|v| v.as_u64()).unwrap_or(5);
                                if !eid.is_empty() {
                                    specs.insert(eid.to_string(), refresh);
                                }
                            }
                        }
                    }
                }
                specs
            };

            for (eid, interval) in entity_specs {
                let should_fetch = {
                    let last = st_ha.ha_last_fetch.read().await;
                    match last.get(&eid) {
                        None => true,
                        Some(t) => t.elapsed().as_secs() >= interval,
                    }
                };

                if should_fetch {
                    st_ha.ha_last_fetch.write().await.insert(eid.clone(), Instant::now());
                    let ha = client.clone();
                    let ha_map = st_ha.ha_values.clone();
                    tokio::spawn(async move {
                        if let Ok(state_val) = ha.get_state(&eid).await {
                            ha_map.write().await.insert(eid, state_val);
                        }
                    });
                }
            }
        }
    });
}

// WebSocket device connection handler
pub async fn handle_device_ws(
    stream: tokio::net::TcpStream,
    addr: SocketAddr,
    state: Arc<AppState>,
) {
    let mut device_id = String::new();
    let callback = |req: &WsRequest, resp: WsResponse| {
        let uri = req.uri();
        let _query = uri.query().unwrap_or("");
        let query_pairs: HashMap<String, String> = Url::parse(&format!("http://localhost{}", uri))
            .map(|u| u.query_pairs().into_owned().collect())
            .unwrap_or_default();

        let id = query_pairs.get("mac").or_else(|| query_pairs.get("id")).cloned().unwrap_or_else(|| {
            format!("yello-{}", addr.ip().to_string().replace('.', "-"))
        });
        device_id = id;
        Ok(resp)
    };

    let ws_stream = match tokio_tungstenite::accept_hdr_async(stream, callback).await {
        Ok(ws) => ws,
        Err(e) => {
            eprintln!("WS handshake failed: {}", e);
            return;
        }
    };

    if device_id.is_empty() {
        device_id = format!("yello-{}", addr.ip().to_string().replace('.', "-"));
    }

    let (mut ws_sink, mut ws_stream_rx) = ws_stream.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<WsMessage>();

    // Forward outbound channel messages to WebSocket sink
    tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if ws_sink.send(msg).await.is_err() {
                break;
            }
        }
    });

    // Register session
    {
        let mut studio = state.studio.write().await;
        if !studio.screens.contains_key(&device_id) {
            studio.screens.insert(
                device_id.clone(),
                Screen {
                    id: device_id.clone(),
                    name: format!("Yello {}", &device_id[device_id.len().saturating_sub(6)..]),
                    design_id: DEFAULT_DESIGN_ID.to_string(),
                    last_seen: Utc::now().to_rfc3339(),
                },
            );
        }
    }

    let initial_design_id = {
        let studio = state.studio.read().await;
        studio.screens.get(&device_id).map(|s| s.design_id.clone()).unwrap_or_else(|| DEFAULT_DESIGN_ID.to_string())
    };

    {
        let mut devices = state.devices.write().await;
        devices.insert(
            device_id.clone(),
            DeviceSession {
                device_id: device_id.clone(),
                remote: addr.to_string(),
                last_frame: None,
                last_full_push: Instant::now() - Duration::from_secs(400),
                pressed: HashSet::new(),
                dots: Vec::new(),
                connected_at: Instant::now(),
                connected_since: Utc::now().to_rfc3339(),
                stream_samples: VecDeque::new(),
                total_pixels: 0,
                total_wire_bytes: 0,
                total_zone_messages: 0,
                frames_sent: 0,
                last_push_ms: 0.0,
                last_frame_at: Utc::now().to_rfc3339(),
                orientation: "portrait".to_string(),
                last_ping_at: Utc::now().to_rfc3339(),
                wifi_rssi_dbm: None,
                uptime_ms: 0,
                active_design_id: initial_design_id,
                ws_sender: Some(tx.clone()),
                push_lock: Arc::new(Mutex::new(())),
                last_tick: Instant::now(),
            },
        );
    }

    // Push full initial frame
    let _ = push_frame(&state, &device_id, true).await;

    // Listen for incoming messages from device
    while let Some(Ok(msg)) = ws_stream_rx.next().await {
        match msg {
            WsMessage::Binary(data) => {
                if data.len() >= 3 && data[0] == PROTO_MAGIC && data[1] == PROTO_VERSION {
                    match data[2] {
                        2 => {
                            // TouchEvent
                            if let Ok(touch) = TouchEvent::unpack(&data) {
                                handle_device_touch(&state, &device_id, touch.x as i32, touch.y as i32).await;
                            }
                        }
                        4 => {
                            // DeviceStatus
                            if let Ok(status) = DeviceStatus::unpack(&data) {
                                let mut devices = state.devices.write().await;
                                if let Some(session) = devices.get_mut(&device_id) {
                                    session.uptime_ms = status.uptime_ms as u64;
                                    session.wifi_rssi_dbm = Some(status.wifi_rssi_dbm);
                                    session.last_ping_at = Utc::now().to_rfc3339();
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            WsMessage::Ping(payload) => {
                let _ = tx.send(WsMessage::Pong(payload));
            }
            WsMessage::Close(_) => break,
            _ => {}
        }
    }

    // Cleanup session on disconnect
    {
        let mut devices = state.devices.write().await;
        devices.remove(&device_id);
    }
}

async fn handle_device_touch(state: &Arc<AppState>, device_id: &str, x: i32, y: i32) {
    let (_design_id, layout) = {
        let studio = state.studio.read().await;
        let devices = state.devices.read().await;
        let session = match devices.get(device_id) {
            Some(s) => s,
            None => return,
        };
        let d_id = design_id_for_session(&studio, session);
        let lay = studio.designs.get(&d_id).map(|d| d.layout.clone()).unwrap_or_else(default_layout);
        (d_id, lay)
    };

    // Add dot feedback
    {
        let mut devices = state.devices.write().await;
        if let Some(session) = devices.get_mut(device_id) {
            session.dots.push((x, y, Instant::now() + Duration::from_secs_f64(DOT_LIFETIME_S)));
        }
    }

    if let Some(hit) = renderer::hit_test_buttons(&layout, x, y) {
        let elem_id = hit.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let elem_type = hit.get("type").and_then(|v| v.as_str()).unwrap_or("");
        let props = hit.get("props").unwrap_or(&Value::Null);

        // Visual flash feedback
        {
            let mut devices = state.devices.write().await;
            if let Some(session) = devices.get_mut(device_id) {
                session.pressed.insert(elem_id.clone());
            }
        }
        let _ = push_frame(state, device_id, false).await;

        let st = state.clone();
        let d_id = device_id.to_string();
        let el_id = elem_id.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs_f64(BUTTON_FLASH_S)).await;
            let mut devices = st.devices.write().await;
            if let Some(session) = devices.get_mut(&d_id) {
                session.pressed.remove(&el_id);
            }
            drop(devices);
            let _ = push_frame(&st, &d_id, false).await;
        });

        match elem_type {
            "button" => {
                if let Some(action) = renderer::resolve_button_action(&layout, hit) {
                    let url = action.get("url").and_then(|v| v.as_str()).unwrap_or("");
                    let method = action.get("method").and_then(|v| v.as_str()).unwrap_or("GET");
                    let body = action.get("body").and_then(|v| v.as_str()).unwrap_or("");
                    if !url.is_empty() {
                        let client = state.safe_http.clone();
                        let url_s = url.to_string();
                        let method_s = method.to_string();
                        let body_s = body.to_string();
                        tokio::spawn(async move {
                            let _ = client.fetch_bytes_with_method(&url_s, MAX_EXTERNAL_BYTES, &method_s, Some(&body_s)).await;
                        });
                    }
                }
            }
            "design-link" => {
                let target = props.get("targetDesignId").and_then(|v| v.as_str()).unwrap_or(DEFAULT_DESIGN_ID);
                let mut devices = state.devices.write().await;
                if let Some(session) = devices.get_mut(device_id) {
                    session.active_design_id = target.to_string();
                }
            }
            "ha-toggle" => {
                let entity_id = props.get("entityId").and_then(|v| v.as_str()).unwrap_or("");
                if !entity_id.is_empty() {
                    let client = state.ha_client.read().await.clone();
                    let eid = entity_id.to_string();
                    let st = state.clone();
                    tokio::spawn(async move {
                        if client.toggle(&eid).await.is_ok() {
                            if let Ok(new_state) = client.get_state(&eid).await {
                                st.ha_values.write().await.insert(eid, new_state);
                            }
                        }
                    });
                }
            }
            _ => {}
        }
    }

    let _ = push_frame(state, device_id, false).await;
}

// REST API Request Router
pub async fn handle_http_request(
    req: Request<Body>,
    state: Arc<AppState>,
) -> Result<Response<Body>, Infallible> {
    let method = req.method().clone();
    let path = req.uri().path().to_string();

    let resp = match (method, path.as_str()) {
        (Method::GET, "/") | (Method::GET, "/designer") | (Method::GET, "/devices") | (Method::GET, "/designs") | (Method::GET, "/settings") => {
            serve_static_file(&state.static_dir.join("index.html"), "text/html").await
        }
        (Method::GET, p) if p.starts_with("/static/") => {
            let rel = p.trim_start_matches("/static/");
            let file_path = state.static_dir.join(rel);
            let content_type = if rel.ends_with(".js") {
                "application/javascript"
            } else if rel.ends_with(".css") {
                "text/css"
            } else if rel.ends_with(".png") {
                "image/png"
            } else if rel.ends_with(".html") {
                "text/html"
            } else {
                "application/octet-stream"
            };
            serve_static_file(&file_path, content_type).await
        }
        (Method::GET, "/api/layout") => {
            let studio = state.studio.read().await;
            let query = req.uri().query().unwrap_or_default();
            let design_id = query
                .split('&')
                .find_map(|pair| {
                    let (name, value) = pair.split_once('=')?;
                    (name == "design").then(|| value.to_string())
                })
                .unwrap_or_else(|| DEFAULT_DESIGN_ID.to_string());
            match studio.designs.get(&design_id) {
                Some(design) => json_response(StatusCode::OK, &json!({
                    "designId": design_id,
                    "layout": design.layout,
                    "revision": design.revision,
                })),
                None => json_response(StatusCode::NOT_FOUND, &json!({"error": "design not found"})),
            }
        }
        (Method::POST, "/api/layout") => {
            let design_id = query_param(&req, "design").unwrap_or_else(|| DEFAULT_DESIGN_ID.to_string());
            let body_bytes = hyper::body::to_bytes(req.into_body()).await.unwrap_or_default();
            match serde_json::from_slice::<Value>(&body_bytes) {
                Ok(raw) => match validate_layout(raw) {
                    Ok(layout) => {
                        let exists = {
                            let studio = state.studio.read().await;
                            studio.designs.contains_key(&design_id)
                        };
                        if !exists {
                            return Ok(json_response(StatusCode::NOT_FOUND, &json!({"error": "design not found"})));
                        }
                        let mut studio = state.studio.write().await;
                        let design = studio.designs.get_mut(&design_id).unwrap();
                        design.layout = layout.clone();
                        design.name = layout.get("name").and_then(|v| v.as_str()).unwrap_or(&design.name).to_string();
                        design.revision += 1;
                        let rev = design.revision;
                        drop(studio);
                        state.save_studio_file().await;
                        push_design(&state, &design_id, true).await;
                        let applied = {
                            let studio = state.studio.read().await;
                            let devices = state.devices.read().await;
                            devices.values()
                                .filter(|session| design_id_for_session(&studio, session) == design_id)
                                .count()
                        };
                        json_response(StatusCode::OK, &json!({"status": "ok", "layout": layout, "revision": rev, "ok": true, "applied": applied > 0, "appliedScreens": applied}))
                    }
                    Err(e) => json_response(StatusCode::UNPROCESSABLE_ENTITY, &json!({"error": "invalid layout", "details": e.errors})),
                },
                Err(_) => json_response(StatusCode::BAD_REQUEST, &json!({"error": "invalid JSON"})),
            }
        }
        (Method::POST, "/api/layout/validate") => {
            let body_bytes = hyper::body::to_bytes(req.into_body()).await.unwrap_or_default();
            match serde_json::from_slice::<Value>(&body_bytes) {
                Ok(raw) => match validate_layout(raw) {
                    Ok(layout) => json_response(StatusCode::OK, &json!({"status": "ok", "layout": layout})),
                    Err(e) => json_response(StatusCode::UNPROCESSABLE_ENTITY, &json!({"error": "invalid layout", "details": e.errors})),
                },
                Err(_) => json_response(StatusCode::BAD_REQUEST, &json!({"error": "invalid JSON"})),
            }
        }
        (Method::POST, "/api/preview") => {
            let body_bytes = hyper::body::to_bytes(req.into_body()).await.unwrap_or_default();
            match serde_json::from_slice::<Value>(&body_bytes) {
                Ok(raw) => match validate_layout(raw) {
                    Ok(layout) => {
                        let ext_vals = state.external_values.read().await;
                        let ha_vals = state.ha_values.read().await;
                        let empty_set = HashSet::new();
                        let empty_dots = Vec::new();
                        let ctx = build_render_context(&ext_vals, DEFAULT_DESIGN_ID, &ha_vals, &empty_set, &empty_dots);
                        let images = state.images.read().await.clone();
                        let img = renderer::render(&layout, &ctx, &images);
                        png_response(&img)
                    }
                    Err(e) => json_response(StatusCode::UNPROCESSABLE_ENTITY, &json!({"error": "invalid layout", "details": e.errors})),
                },
                Err(_) => json_response(StatusCode::BAD_REQUEST, &json!({"error": "invalid JSON"})),
            }
        }
        (Method::GET, "/api/preview") => {
            let query = req.uri().query().unwrap_or_default();
            let design_id = query
                .split('&')
                .find_map(|pair| {
                    let (name, value) = pair.split_once('=')?;
                    (name == "design").then(|| value.to_string())
                })
                .unwrap_or_else(|| DEFAULT_DESIGN_ID.to_string());
            let studio = state.studio.read().await;
            match studio.designs.get(&design_id) {
                Some(design) => {
                    let ext_vals = state.external_values.read().await;
                    let ha_vals = state.ha_values.read().await;
                    let empty_set = HashSet::new();
                    let empty_dots = Vec::new();
                    let ctx = build_render_context(&ext_vals, &design_id, &ha_vals, &empty_set, &empty_dots);
                    let images = state.images.read().await.clone();
                    let img = renderer::render(&design.layout, &ctx, &images);
                    png_response(&img)
                }
                None => json_response(StatusCode::NOT_FOUND, &json!({"error": "design not found"})),
            }
        }
        (Method::GET, p) if p.starts_with("/api/designs/") && p.ends_with("/preview") => {
            let design_id = p.trim_start_matches("/api/designs/").trim_end_matches("/preview");
            let studio = state.studio.read().await;
            if let Some(design) = studio.designs.get(design_id) {
                let ext_vals = state.external_values.read().await;
                let ha_vals = state.ha_values.read().await;
                let empty_set = HashSet::new();
                let empty_dots = Vec::new();
                let ctx = build_render_context(&ext_vals, design_id, &ha_vals, &empty_set, &empty_dots);
                let images = state.images.read().await.clone();
                let img = renderer::render(&design.layout, &ctx, &images);
                png_response(&img)
            } else {
                json_response(StatusCode::NOT_FOUND, &json!({"error": "design not found"}))
            }
        }
        (Method::GET, "/api/catalog") => {
            let types = [
                "text", "clock", "external-text", "color-block", "button",
                "design-link", "image", "ha-state", "ha-toggle", "script",
            ];
            let mut element_defaults = serde_json::Map::new();
            for element_type in types {
                element_defaults.insert(
                    element_type.to_string(),
                    new_element(element_type, "example"),
                );
            }
            let script_examples: Vec<Value> = SCRIPT_EXAMPLES.iter().map(|example| {
                json!({
                    "id": example.id,
                    "name": example.name,
                    "description": example.description,
                    "code": example.code,
                })
            }).collect();
            json_response(StatusCode::OK, &json!({
                "elementDefaults": Value::Object(element_defaults),
                "scriptExamples": script_examples,
            }))
        }
        (Method::GET, "/api/studio") => {
            let studio = state.studio.read().await;
            let devices = state.devices.read().await;
            let ha_configured = state.ha_client.read().await.configured();

            let mut designs_arr = Vec::new();
            for design in studio.designs.values() {
                designs_arr.push(json!({
                    "id": design.id,
                    "name": design.name,
                    "revision": design.revision,
                    "orientation": design.layout.get("orientation").and_then(|v| v.as_str()).unwrap_or("portrait"),
                }));
            }

            let mut screens_arr = Vec::new();
            for screen in studio.screens.values() {
                let session = devices.get(&screen.id);
                let active_design_id = match session {
                    Some(sess) => design_id_for_session(&studio, sess),
                    None => screen.design_id.clone(),
                };
                screens_arr.push(json!({
                    "id": screen.id,
                    "name": screen.name,
                    "designId": screen.design_id,
                    "lastSeen": screen.last_seen,
                    "connected": session.is_some(),
                    "activeDesignId": active_design_id,
                    "remote": session.map(|s| s.remote.clone()).unwrap_or_default(),
                    "stream": session.map(|s| json!({
                        "framesSent": s.frames_sent,
                        "totalPixels": s.total_pixels,
                        "totalWireBytes": s.total_wire_bytes,
                        "totalZoneMessages": s.total_zone_messages,
                        "lastPushMs": s.last_push_ms,
                        "lastFrameAt": s.last_frame_at,
                    })),
                    "heartbeat": session.map(|s| json!({
                        "lastPingAt": s.last_ping_at,
                        "wifiRssiDbm": s.wifi_rssi_dbm,
                        "uptimeMs": s.uptime_ms,
                    })),
                }));
            }

            let mut devices_arr = Vec::new();
            for (id, session) in devices.iter() {
                let active_d = design_id_for_session(&studio, session);
                devices_arr.push(json!({
                    "deviceId": id,
                    "remote": session.remote,
                    "connectedSince": session.connected_since,
                    "activeDesignId": active_d,
                    "framesSent": session.frames_sent,
                    "totalPixels": session.total_pixels,
                    "totalWireBytes": session.total_wire_bytes,
                    "totalZoneMessages": session.total_zone_messages,
                    "lastPushMs": session.last_push_ms,
                    "lastFrameAt": session.last_frame_at,
                    "wifiRssiDbm": session.wifi_rssi_dbm,
                    "uptimeMs": session.uptime_ms
                }));
            }

            json_response(StatusCode::OK, &json!({
                "designs": designs_arr,
                "screens": screens_arr,
                "integrations": {"homeAssistant": {"configured": ha_configured}},
                "devices": devices_arr
            }))
        }
        (Method::POST, "/api/designs") => {
            let body_bytes = hyper::body::to_bytes(req.into_body()).await.unwrap_or_default();
            if let Ok(raw) = serde_json::from_slice::<Value>(&body_bytes) {
                let name = raw.get("name").and_then(|v| v.as_str()).unwrap_or("New Design").to_string();
                let clone_from = raw.get("cloneFrom").and_then(|v| v.as_str()).unwrap_or(DEFAULT_DESIGN_ID).to_string();
                let mut layout = {
                    let studio = state.studio.read().await;
                    studio.designs.get(&clone_from).map(|d| d.layout.clone()).unwrap_or_else(default_layout)
                };
                layout["name"] = json!(name);
                if let Some(orientation) = raw.get("orientation").and_then(|v| v.as_str()) {
                    if orientation == "portrait" || orientation == "landscape" {
                        layout["orientation"] = json!(orientation);
                    }
                }
                let normalized = validate_layout(layout).unwrap_or_else(|_| default_layout());

                let id = format!("design_{:x}", Utc::now().timestamp_micros());
                let mut studio = state.studio.write().await;
                studio.designs.insert(id.clone(), Design {
                    id: id.clone(),
                    name: name.clone(),
                    revision: 0,
                    layout: normalized.clone(),
                });
                drop(studio);
                state.save_studio_file().await;

                json_response(StatusCode::CREATED, &json!({
                    "ok": true,
                    "design": {"id": id, "name": name, "revision": 0},
                    "id": id, "name": name, "revision": 0, "layout": normalized
                }))
            } else {
                json_response(StatusCode::BAD_REQUEST, &json!({"error": "invalid JSON"}))
            }
        }
        (Method::POST, p) if p.starts_with("/api/designs/") && p.ends_with("/assign") => {
            let design_id = p.trim_start_matches("/api/designs/").trim_end_matches("/assign");
            let body_bytes = hyper::body::to_bytes(req.into_body()).await.unwrap_or_default();
            if let Ok(raw) = serde_json::from_slice::<Value>(&body_bytes) {
                if let Some(screen_ids) = raw.get("screenIds").and_then(|v| v.as_array()) {
                    let mut studio = state.studio.write().await;
                    for sid in screen_ids {
                        if let Some(s) = sid.as_str() {
                            if let Some(scr) = studio.screens.get_mut(s) {
                                scr.design_id = design_id.to_string();
                            }
                        }
                    }
                    drop(studio);
                    state.save_studio_file().await;
                    push_design(&state, design_id, false).await;
                    json_response(StatusCode::OK, &json!({"status": "ok"}))
                } else {
                    json_response(StatusCode::BAD_REQUEST, &json!({"error": "screenIds array required"}))
                }
            } else {
                json_response(StatusCode::BAD_REQUEST, &json!({"error": "invalid JSON"}))
            }
        }
        (Method::PUT, p) if p.starts_with("/api/screens/") => {
            let screen_id = p.trim_start_matches("/api/screens/");
            let body_bytes = hyper::body::to_bytes(req.into_body()).await.unwrap_or_default();
            if let Ok(raw) = serde_json::from_slice::<Value>(&body_bytes) {
                let mut studio = state.studio.write().await;
                if let Some(screen) = studio.screens.get_mut(screen_id) {
                    if let Some(name) = raw.get("name").and_then(|v| v.as_str()) {
                        screen.name = name.to_string();
                    }
                    if let Some(d_id) = raw.get("designId").and_then(|v| v.as_str()) {
                        screen.design_id = d_id.to_string();
                    }
                    let res = screen.clone();
                    drop(studio);
                    state.save_studio_file().await;
                    push_design(&state, &res.design_id, false).await;
                    json_response(StatusCode::OK, &serde_json::to_value(&res).unwrap_or_default())
                } else {
                    json_response(StatusCode::NOT_FOUND, &json!({"error": "screen not found"}))
                }
            } else {
                json_response(StatusCode::BAD_REQUEST, &json!({"error": "invalid JSON"}))
            }
        }
        (Method::POST, p) if p.starts_with("/api/screens/") && p.ends_with("/restart") => {
            let screen_id = p.trim_start_matches("/api/screens/").trim_end_matches("/restart");
            let devices = state.devices.read().await;
            if let Some(session) = devices.get(screen_id) {
                if let Some(ref ws) = session.ws_sender {
                    let cmd = DeviceCommand { command: DeviceCommandType::Restart };
                    let _ = ws.send(WsMessage::Binary(cmd.pack()));
                }
                json_response(StatusCode::OK, &json!({"status": "restart command sent"}))
            } else {
                json_response(StatusCode::NOT_FOUND, &json!({"error": "device not connected"}))
            }
        }
        (Method::GET, "/api/settings/home-assistant") => {
            let ha = state.ha_client.read().await;
            json_response(StatusCode::OK, &json!({
                "configured": ha.configured(),
                "url": ha.configured()
            }))
        }
        (Method::GET, "/api/integrations/home-assistant") => {
            let ha = state.ha_client.read().await;
            if !ha.configured() {
                json_response(StatusCode::OK, &json!({"configured": false, "entities": []}))
            } else {
                match ha.list_states().await {
                    Ok(states) => {
                        let mut entities: Vec<Value> = states.iter().filter_map(|item| {
                            let entity_id = item.entity_id.as_str();
                            (!entity_id.is_empty()).then(|| json!({
                                "entityId": entity_id,
                                "domain": entity_id.split('.').next().unwrap_or(""),
                                "name": item.attributes.get("friendly_name").and_then(|v| v.as_str()).unwrap_or(entity_id),
                                "state": item.state,
                                "unit": item.attributes.get("unit_of_measurement").and_then(|v| v.as_str()).unwrap_or(""),
                            }))
                        }).collect();
                        entities.sort_by(|a, b| {
                            (a["domain"].as_str().unwrap_or(""), a["name"].as_str().unwrap_or("").to_lowercase())
                                .cmp(&(b["domain"].as_str().unwrap_or(""), b["name"].as_str().unwrap_or("").to_lowercase()))
                        });
                        json_response(StatusCode::OK, &json!({"configured": true, "entities": entities}))
                    }
                    Err(e) => json_response(StatusCode::OK, &json!({"configured": true, "entities": [], "error": e.to_string()})),
                }
            }
        }
        (Method::PUT, "/api/settings/home-assistant") => {
            let body_bytes = hyper::body::to_bytes(req.into_body()).await.unwrap_or_default();
            if let Ok(raw) = serde_json::from_slice::<Value>(&body_bytes) {
                let url = raw.get("url").and_then(|v| v.as_str()).unwrap_or("");
                let token = raw.get("token").and_then(|v| v.as_str()).unwrap_or("");
                if let Err(e) = state.save_settings(url, token).await {
                    json_response(StatusCode::INTERNAL_SERVER_ERROR, &json!({"error": e}))
                } else {
                    json_response(StatusCode::OK, &json!({"status": "ok"}))
                }
            } else {
                json_response(StatusCode::BAD_REQUEST, &json!({"error": "invalid JSON"}))
            }
        }
        (Method::GET, "/api/status") => {
            let devices = state.devices.read().await;
            let studio = state.studio.read().await;
            json_response(StatusCode::OK, &json!({
                "deviceConnected": !devices.is_empty(),
                "connectedScreens": devices.len(),
                "knownScreens": studio.screens.len(),
                "homeAssistantConfigured": state.ha_client.read().await.configured(),
                "devicesCount": devices.len(),
                "designsCount": studio.designs.len(),
                "screensCount": studio.screens.len()
            }))
        }
        _ => json_response(StatusCode::NOT_FOUND, &json!({"error": "not found"})),
    };

    Ok(resp)
}

fn query_param(req: &Request<Body>, name: &str) -> Option<String> {
    let query = req.uri().query()?;
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key == name).then(|| value.to_string())
    })
}

fn json_response(status: StatusCode, val: &Value) -> Response<Body> {
    let body_str = serde_json::to_string(val).unwrap_or_else(|_| "{}".to_string());
    Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .header("Access-Control-Allow-Origin", "*")
        .body(Body::from(body_str))
        .unwrap()
}

fn png_response(img: &RgbImage) -> Response<Body> {
    let mut buf = Vec::new();
    let mut cursor = std::io::Cursor::new(&mut buf);
    let _ = img.write_to(&mut cursor, image::ImageFormat::Png);
    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "image/png")
        .header("Access-Control-Allow-Origin", "*")
        .body(Body::from(buf))
        .unwrap()
}

async fn serve_static_file(path: &Path, content_type: &str) -> Response<Body> {
    match tokio::fs::read(path).await {
        Ok(data) => Response::builder()
            .status(StatusCode::OK)
            .header("Content-Type", content_type)
            .body(Body::from(data))
            .unwrap(),
        Err(_) => Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Body::from("File not found"))
            .unwrap(),
    }
}

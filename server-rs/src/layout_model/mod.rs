//! Versioned layout validation and migration.

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::HashSet;

pub const SCHEMA_VERSION: i32 = 1;
pub const DISPLAY_WIDTH: u16 = 240;
pub const DISPLAY_HEIGHT: u16 = 320;
pub const LANDSCAPE_WIDTH: u16 = 320;
pub const LANDSCAPE_HEIGHT: u16 = 240;
pub const MAX_ELEMENTS: usize = 100;
pub const MAX_DATA_SOURCES: usize = 20;
pub const MAX_ACTIONS: usize = 20;

pub const DEFAULT_DESIGN_ID: &str = "default";

lazy_static::lazy_static! {
    static ref ID_RE: Regex = Regex::new(r"^[A-Za-z][A-Za-z0-9_-]{0,63}$").unwrap();
    static ref COLOR_RE: Regex = Regex::new(r"^#[0-9a-fA-F]{6}$").unwrap();
    static ref HA_ENTITY_RE: Regex = Regex::new(r"^[a-z0-9_]+\.[a-z0-9_]+$").unwrap();
    static ref TARGET_DESIGN_RE: Regex = Regex::new(r"^[A-Za-z0-9._:-]{1,64}$").unwrap();
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Orientation {
    Portrait,
    Landscape,
}

impl std::fmt::Display for Orientation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Orientation::Portrait => write!(f, "portrait"),
            Orientation::Landscape => write!(f, "landscape"),
        }
    }
}

pub type DisplayOrientation = Orientation;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayoutError {
    pub path: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayoutValidationError {
    pub errors: Vec<LayoutError>,
}

impl std::fmt::Display for LayoutValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "invalid layout: {} errors", self.errors.len())
    }
}

impl std::error::Error for LayoutValidationError {}

impl LayoutValidationError {
    pub fn new(message: &str) -> Self {
        Self {
            errors: vec![LayoutError {
                path: "$".to_string(),
                message: message.to_string(),
            }],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Frame {
    pub x: i32,
    pub y: i32,
    pub w: u16,
    pub h: u16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ElementStyle {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub off_color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub font_size: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub radius: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub align: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vertical_align: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataSource {
    pub id: String,
    pub name: String,
    pub r#type: String,
    pub url: String,
    pub interval: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Action {
    pub id: String,
    pub name: String,
    pub r#type: String,
    pub method: String,
    pub url: String,
    #[serde(default)]
    pub body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Element {
    pub id: String,
    pub r#type: String,
    pub name: String,
    pub visible: bool,
    pub locked: bool,
    pub frame: Frame,
    pub style: Value,
    pub props: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Layout {
    pub schema_version: i32,
    pub name: String,
    pub profile: String,
    pub orientation: String,
    pub background: String,
    pub data_sources: Vec<DataSource>,
    pub actions: Vec<Action>,
    pub elements: Vec<Element>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Screen {
    pub id: String,
    pub name: String,
    pub design_id: String,
    pub last_seen: String,
}

pub fn default_element(element_type: &str, element_id: &str) -> Result<Value, String> {
    let mut defaults: Map<String, Value> = Map::new();
    defaults.insert("id".to_string(), json!(element_id));
    defaults.insert("type".to_string(), json!(element_type));
    let name = element_type.replace('-', " ");
    let title_name = name
        .split_whitespace()
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    defaults.insert("name".to_string(), json!(title_name));
    defaults.insert("visible".to_string(), json!(true));
    defaults.insert("locked".to_string(), json!(false));

    match element_type {
        "text" => {
            defaults.insert("frame".to_string(), json!({"x": 20, "y": 140, "w": 200, "h": 40}));
            defaults.insert(
                "style".to_string(),
                json!({"color": "#ffffff", "fontSize": 18, "align": "center", "verticalAlign": "middle"}),
            );
            defaults.insert("props".to_string(), json!({"text": "New text", "wrap": true}));
        }
        "clock" => {
            defaults.insert("frame".to_string(), json!({"x": 20, "y": 130, "w": 200, "h": 60}));
            defaults.insert(
                "style".to_string(),
                json!({"color": "#ffffff", "fontSize": 32, "align": "center", "verticalAlign": "middle"}),
            );
            defaults.insert("props".to_string(), json!({"format": "%H:%M:%S", "timezone": ""}));
        }
        "external-text" => {
            defaults.insert("frame".to_string(), json!({"x": 20, "y": 140, "w": 200, "h": 40}));
            defaults.insert(
                "style".to_string(),
                json!({"color": "#88ccff", "fontSize": 14, "align": "center", "verticalAlign": "middle"}),
            );
            defaults.insert("props".to_string(), json!({"sourceId": "", "wrap": true}));
        }
        "color-block" => {
            defaults.insert("frame".to_string(), json!({"x": 80, "y": 140, "w": 80, "h": 40}));
            defaults.insert("style".to_string(), json!({"color": "#3355aa", "radius": 0}));
            defaults.insert("props".to_string(), json!({}));
        }
        "button" => {
            defaults.insert("frame".to_string(), json!({"x": 60, "y": 140, "w": 120, "h": 44}));
            defaults.insert("style".to_string(), json!({"color": "#2255aa", "fontSize": 16, "radius": 8}));
            defaults.insert("props".to_string(), json!({"label": "Press me", "actionId": ""}));
        }
        "design-link" => {
            defaults.insert("frame".to_string(), json!({"x": 60, "y": 140, "w": 120, "h": 44}));
            defaults.insert("style".to_string(), json!({"color": "#6b9cff", "fontSize": 16, "radius": 8}));
            defaults.insert(
                "props".to_string(),
                json!({"label": "Open screen", "targetDesignId": "default"}),
            );
        }
        "image" => {
            defaults.insert("frame".to_string(), json!({"x": 80, "y": 120, "w": 80, "h": 80}));
            defaults.insert("style".to_string(), json!({}));
            defaults.insert("props".to_string(), json!({"src": "", "fit": "contain"}));
        }
        "ha-state" => {
            defaults.insert("frame".to_string(), json!({"x": 20, "y": 140, "w": 200, "h": 40}));
            defaults.insert(
                "style".to_string(),
                json!({"color": "#88ccff", "fontSize": 18, "align": "center", "verticalAlign": "middle"}),
            );
            defaults.insert(
                "props".to_string(),
                json!({
                    "entityId": "sensor.example", "attribute": "", "prefix": "", "suffix": "",
                    "decimals": 1, "showUnit": true, "refreshInterval": 5, "wrap": true
                }),
            );
        }
        "ha-toggle" => {
            defaults.insert("frame".to_string(), json!({"x": 60, "y": 140, "w": 120, "h": 44}));
            defaults.insert(
                "style".to_string(),
                json!({"color": "#f2c94c", "offColor": "#353b48", "fontSize": 16, "radius": 8}),
            );
            defaults.insert(
                "props".to_string(),
                json!({"entityId": "light.example", "label": "Light", "refreshInterval": 2}),
            );
        }
        "script" => {
            defaults.insert("frame".to_string(), json!({"x": 20, "y": 130, "w": 200, "h": 60}));
            defaults.insert(
                "style".to_string(),
                json!({"color": "#ffffff", "fontSize": 16, "align": "center", "verticalAlign": "middle"}),
            );
            defaults.insert(
                "props".to_string(),
                json!({
                    "code": "text(\"Hello from a script\")",
                    "refreshInterval": 5,
                    "wrap": true
                }),
            );
        }
        _ => return Err(format!("unknown element type: {}", element_type)),
    }

    Ok(Value::Object(defaults))
}

pub fn new_element(element_type: &str, element_id: &str) -> Value {
    default_element(element_type, element_id).unwrap_or_else(|_| json!({}))
}

pub fn default_layout() -> Value {
    let mut clock = default_element("clock", "clock").unwrap();
    clock["frame"] = json!({"x": 0, "y": 56, "w": 240, "h": 68});
    clock["style"]["fontSize"] = json!(48);

    let mut date = default_element("clock", "date").unwrap();
    date["name"] = json!("Date");
    date["frame"] = json!({"x": 0, "y": 124, "w": 240, "h": 32});
    date["style"]["fontSize"] = json!(20);
    date["style"]["color"] = json!("#78c8ff");
    date["props"]["format"] = json!("%a %b %d, %Y");

    let mut divider = default_element("color-block", "divider").unwrap();
    divider["name"] = json!("Divider");
    divider["frame"] = json!({"x": 20, "y": 179, "w": 200, "h": 2});
    divider["style"]["color"] = json!("#505078");

    let mut greeting = default_element("text", "greeting").unwrap();
    greeting["name"] = json!("Greeting");
    greeting["frame"] = json!({"x": 0, "y": 188, "w": 240, "h": 44});
    greeting["style"]["fontSize"] = json!(18);
    greeting["style"]["color"] = json!("#ffdc78");
    greeting["props"]["text"] = json!("Hello from the server!");

    json!({
        "schemaVersion": SCHEMA_VERSION,
        "name": "Yello screen",
        "profile": "ili9341-240x320",
        "orientation": "portrait",
        "background": "#0a0a1e",
        "dataSources": [],
        "actions": [],
        "elements": [clock, date, divider, greeting]
    })
}

pub fn layout_dimensions(layout: &Value) -> (u16, u16) {
    if layout.get("orientation").and_then(|v| v.as_str()) == Some("landscape") {
        (LANDSCAPE_WIDTH, LANDSCAPE_HEIGHT)
    } else {
        (DISPLAY_WIDTH, DISPLAY_HEIGHT)
    }
}

pub fn migrate_layout(mut raw: Value) -> Value {
    if !raw.is_object() || raw.get("schemaVersion").is_some() {
        return raw;
    }

    let mut migrated = json!({
        "schemaVersion": SCHEMA_VERSION,
        "name": raw.get("name").and_then(|v| v.as_str()).unwrap_or("Yello screen"),
        "profile": "ili9341-240x320",
        "orientation": "portrait",
        "background": raw.get("background").and_then(|v| v.as_str()).unwrap_or("#0a0a1e"),
        "dataSources": [],
        "actions": [],
        "elements": []
    });

    let mut source_by_url: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut action_by_url: std::collections::HashMap<String, String> = std::collections::HashMap::new();

    if let Some(elements) = raw.get_mut("elements").and_then(|v| v.as_array_mut()) {
        for (index, old) in elements.iter_mut().enumerate() {
            if !old.is_object() {
                continue;
            }
            let raw_type = old.get("type").and_then(|v| v.as_str()).unwrap_or("");
            let element_type = match raw_type {
                "time" => "clock",
                "external" => "external-text",
                "rect" => "color-block",
                other => other,
            };

            let supported = [
                "text", "clock", "external-text", "color-block", "button",
                "design-link", "image", "ha-state", "ha-toggle", "script",
            ];
            if !supported.contains(&element_type) {
                continue;
            }

            let element_id = old
                .get("id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| format!("element{}", index + 1));

            let mut el = match default_element(element_type, &element_id) {
                Ok(v) => v,
                Err(_) => continue,
            };

            if let Some(name) = old.get("name").and_then(|v| v.as_str()) {
                el["name"] = json!(name);
            }

            let size = old
                .get("size")
                .and_then(|v| v.as_i64())
                .unwrap_or_else(|| el["style"]["fontSize"].as_i64().unwrap_or(18)) as i32;

            if ["text", "clock", "external-text"].contains(&element_type) {
                let width = old.get("w").and_then(|v| v.as_i64()).unwrap_or(DISPLAY_WIDTH as i64) as i32;
                let height = old
                    .get("h")
                    .and_then(|v| v.as_i64())
                    .unwrap_or(24.max((size as f64 * 1.45) as i64)) as i32;
                let center_x = old.get("x").and_then(|v| v.as_i64()).unwrap_or((DISPLAY_WIDTH / 2) as i64) as i32;
                let center_y = old.get("y").and_then(|v| v.as_i64()).unwrap_or((DISPLAY_HEIGHT / 2) as i64) as i32;
                el["frame"] = json!({
                    "x": center_x - width / 2,
                    "y": center_y - height / 2,
                    "w": width,
                    "h": height
                });
            } else {
                let x = old.get("x").and_then(|v| v.as_i64()).unwrap_or_else(|| el["frame"]["x"].as_i64().unwrap_or(0));
                let y = old.get("y").and_then(|v| v.as_i64()).unwrap_or_else(|| el["frame"]["y"].as_i64().unwrap_or(0));
                let w = old.get("w").and_then(|v| v.as_i64()).unwrap_or_else(|| el["frame"]["w"].as_i64().unwrap_or(0));
                let h = old.get("h").and_then(|v| v.as_i64()).unwrap_or_else(|| el["frame"]["h"].as_i64().unwrap_or(0));
                el["frame"] = json!({"x": x, "y": y, "w": w, "h": h});
            }

            if let Some(color) = old.get("color").and_then(|v| v.as_str()) {
                el["style"]["color"] = json!(color);
            }
            if el["style"].get("fontSize").is_some() {
                el["style"]["fontSize"] = json!(size);
            }

            if element_type == "text" {
                if let Some(text) = old.get("text").and_then(|v| v.as_str()) {
                    el["props"]["text"] = json!(text);
                }
            } else if element_type == "clock" {
                if let Some(format) = old.get("format").and_then(|v| v.as_str()) {
                    el["props"]["format"] = json!(format);
                }
            } else if element_type == "external-text" {
                let url = old.get("url").and_then(|v| v.as_str()).unwrap_or("");
                if !url.is_empty() {
                    let source_id = if let Some(id) = source_by_url.get(url) {
                        id.clone()
                    } else {
                        let id = format!("source{}", source_by_url.len() + 1);
                        source_by_url.insert(url.to_string(), id.clone());
                        let interval = old.get("interval").and_then(|v| v.as_i64()).unwrap_or(60);
                        migrated["dataSources"].as_array_mut().unwrap().push(json!({
                            "id": id,
                            "name": format!("External source {}", source_by_url.len()),
                            "type": "http-text",
                            "url": url,
                            "interval": interval
                        }));
                        id
                    };
                    el["props"]["sourceId"] = json!(source_id);
                }
            } else if element_type == "button" {
                if let Some(label) = old.get("label").and_then(|v| v.as_str()) {
                    el["props"]["label"] = json!(label);
                }
                let url = old.get("action").and_then(|v| v.as_str()).unwrap_or("");
                if !url.is_empty() {
                    let action_id = if let Some(id) = action_by_url.get(url) {
                        id.clone()
                    } else {
                        let id = format!("action{}", action_by_url.len() + 1);
                        action_by_url.insert(url.to_string(), id.clone());
                        migrated["actions"].as_array_mut().unwrap().push(json!({
                            "id": id,
                            "name": format!("Button action {}", action_by_url.len()),
                            "type": "http",
                            "method": "GET",
                            "url": url
                        }));
                        id
                    };
                    el["props"]["actionId"] = json!(action_id);
                }
            } else if element_type == "image" {
                if let Some(src) = old.get("src").and_then(|v| v.as_str()) {
                    el["props"]["src"] = json!(src);
                }
            }

            migrated["elements"].as_array_mut().unwrap().push(el);
        }
    }

    migrated
}

struct Validator {
    errors: Vec<LayoutError>,
}

impl Validator {
    fn new() -> Self {
        Self { errors: Vec::new() }
    }

    fn error(&mut self, path: &str, message: &str) {
        self.errors.push(LayoutError {
            path: path.to_string(),
            message: message.to_string(),
        });
    }

    fn string(&mut self, val: Option<&Value>, path: &str, default: &str, max_len: usize) -> String {
        match val {
            None => default.to_string(),
            Some(Value::String(s)) => {
                if s.len() > max_len {
                    self.error(path, &format!("must contain at most {max_len} characters"));
                    s[..max_len].to_string()
                } else {
                    s.clone()
                }
            }
            Some(_) => {
                self.error(path, "must be a string");
                default.to_string()
            }
        }
    }

    fn integer(&mut self, val: Option<&Value>, path: &str, default: i64, min: i64, max: i64) -> i64 {
        match val {
            None => default,
            Some(Value::Bool(_)) => {
                self.error(path, "must be an integer");
                default
            }
            Some(Value::Number(n)) => {
                if let Some(i) = n.as_i64() {
                    if i < min || i > max {
                        self.error(path, &format!("must be between {min} and {max}"));
                    }
                    i.clamp(min, max)
                } else {
                    self.error(path, "must be an integer");
                    default
                }
            }
            Some(_) => {
                self.error(path, "must be an integer");
                default
            }
        }
    }

    fn color(&mut self, val: Option<&Value>, path: &str, default: &str) -> String {
        let s = self.string(val, path, default, 7);
        if !COLOR_RE.is_match(&s) {
            self.error(path, "must be a #rrggbb color");
            default.to_string()
        } else {
            s.to_lowercase()
        }
    }

    fn identifier(&mut self, val: Option<&Value>, path: &str, fallback: &str) -> String {
        let s = self.string(val, path, fallback, 64);
        if !ID_RE.is_match(&s) {
            self.error(path, "must start with a letter and contain only letters, numbers, _ or -");
            fallback.to_string()
        } else {
            s
        }
    }
}

pub fn validate_layout(raw: Value) -> Result<Value, LayoutValidationError> {
    let raw = migrate_layout(raw);
    if !raw.is_object() {
        return Err(LayoutValidationError::new("must be an object"));
    }

    let mut v = Validator::new();

    if raw.get("schemaVersion").and_then(|x| x.as_i64()) != Some(SCHEMA_VERSION as i64) {
        v.error("schemaVersion", &format!("must equal {SCHEMA_VERSION}"));
    }

    let name = v.string(raw.get("name"), "name", "Yello screen", 100);
    let profile = v.string(raw.get("profile"), "profile", "ili9341-240x320", 64);
    if profile != "ili9341-240x320" {
        v.error("profile", "unsupported display profile");
    }

    let mut orientation = v.string(raw.get("orientation"), "orientation", "portrait", 12);
    if orientation != "portrait" && orientation != "landscape" {
        v.error("orientation", "must be portrait or landscape");
        orientation = "portrait".to_string();
    }

    let background = v.color(raw.get("background"), "background", "#0a0a1e");

    let (display_width, display_height) = if orientation == "landscape" {
        (LANDSCAPE_WIDTH as i64, LANDSCAPE_HEIGHT as i64)
    } else {
        (DISPLAY_WIDTH as i64, DISPLAY_HEIGHT as i64)
    };

    let mut result = json!({
        "schemaVersion": SCHEMA_VERSION,
        "name": name,
        "profile": profile,
        "orientation": orientation,
        "background": background,
        "dataSources": [],
        "actions": [],
        "elements": []
    });

    let mut source_ids = HashSet::new();
    if let Some(sources) = raw.get("dataSources") {
        if let Some(arr) = sources.as_array() {
            if arr.len() > MAX_DATA_SOURCES {
                v.error("dataSources", &format!("must contain at most {MAX_DATA_SOURCES} items"));
            }
            for (index, source) in arr.iter().take(MAX_DATA_SOURCES).enumerate() {
                let path = format!("dataSources[{index}]");
                if !source.is_object() {
                    v.error(&path, "must be an object");
                    continue;
                }
                let fallback = format!("source{}", index + 1);
                let id = v.identifier(source.get("id"), &format!("{path}.id"), &fallback);
                if source_ids.contains(&id) {
                    v.error(&format!("{path}.id"), "must be unique");
                }
                source_ids.insert(id.clone());

                let stype = v.string(source.get("type"), &format!("{path}.type"), "http-text", 32);
                if stype != "http-text" {
                    v.error(&format!("{path}.type"), "unsupported data source type");
                }

                result["dataSources"].as_array_mut().unwrap().push(json!({
                    "id": id,
                    "name": v.string(source.get("name"), &format!("{path}.name"), &id, 100),
                    "type": "http-text",
                    "url": v.string(source.get("url"), &format!("{path}.url"), "", 2048),
                    "interval": v.integer(source.get("interval"), &format!("{path}.interval"), 60, 5, 86400)
                }));
            }
        } else {
            v.error("dataSources", "must be an array");
        }
    }

    let mut action_ids = HashSet::new();
    if let Some(actions) = raw.get("actions") {
        if let Some(arr) = actions.as_array() {
            if arr.len() > MAX_ACTIONS {
                v.error("actions", &format!("must contain at most {MAX_ACTIONS} items"));
            }
            for (index, action) in arr.iter().take(MAX_ACTIONS).enumerate() {
                let path = format!("actions[{index}]");
                if !action.is_object() {
                    v.error(&path, "must be an object");
                    continue;
                }
                let fallback = format!("action{}", index + 1);
                let id = v.identifier(action.get("id"), &format!("{path}.id"), &fallback);
                if action_ids.contains(&id) {
                    v.error(&format!("{path}.id"), "must be unique");
                }
                action_ids.insert(id.clone());

                let mut method = v.string(action.get("method"), &format!("{path}.method"), "GET", 4).to_uppercase();
                if method != "GET" && method != "POST" {
                    v.error(&format!("{path}.method"), "must be GET or POST");
                    method = "GET".to_string();
                }

                result["actions"].as_array_mut().unwrap().push(json!({
                    "id": id,
                    "name": v.string(action.get("name"), &format!("{path}.name"), &id, 100),
                    "type": "http",
                    "method": method,
                    "url": v.string(action.get("url"), &format!("{path}.url"), "", 2048),
                    "body": v.string(action.get("body"), &format!("{path}.body"), "", 4096)
                }));
            }
        } else {
            v.error("actions", "must be an array");
        }
    }

    let supported_types = [
        "text", "clock", "external-text", "color-block", "button",
        "design-link", "image", "ha-state", "ha-toggle", "script",
    ];

    let mut element_ids = HashSet::new();
    if let Some(elements) = raw.get("elements") {
        if let Some(arr) = elements.as_array() {
            if arr.len() > MAX_ELEMENTS {
                v.error("elements", &format!("must contain at most {MAX_ELEMENTS} items"));
            }
            for (index, element) in arr.iter().take(MAX_ELEMENTS).enumerate() {
                let path = format!("elements[{index}]");
                if !element.is_object() {
                    v.error(&path, "must be an object");
                    continue;
                }

                let etype = v.string(element.get("type"), &format!("{path}.type"), "", 32);
                if !supported_types.contains(&etype.as_str()) {
                    v.error(&format!("{path}.type"), "unsupported element type");
                    continue;
                }

                let fallback = format!("element{}", index + 1);
                let id = v.identifier(element.get("id"), &format!("{path}.id"), &fallback);
                if element_ids.contains(&id) {
                    v.error(&format!("{path}.id"), "must be unique");
                }
                element_ids.insert(id.clone());

                let defaults = match default_element(&etype, &id) {
                    Ok(d) => d,
                    Err(_) => continue,
                };

                let frame_raw = element.get("frame").and_then(|v| v.as_object());
                let style_raw = element.get("style").and_then(|v| v.as_object());
                let props_raw = element.get("props").and_then(|v| v.as_object());

                if element.get("frame").is_some() && frame_raw.is_none() {
                    v.error(&format!("{path}.frame"), "must be an object");
                }
                if element.get("style").is_some() && style_raw.is_none() {
                    v.error(&format!("{path}.style"), "must be an object");
                }
                if element.get("props").is_some() && props_raw.is_none() {
                    v.error(&format!("{path}.props"), "must be an object");
                }

                let def_frame = defaults.get("frame").unwrap();
                let frame = json!({
                    "x": v.integer(frame_raw.and_then(|m| m.get("x")), &format!("{path}.frame.x"), def_frame["x"].as_i64().unwrap(), -display_width, display_width - 1),
                    "y": v.integer(frame_raw.and_then(|m| m.get("y")), &format!("{path}.frame.y"), def_frame["y"].as_i64().unwrap(), -display_height, display_height - 1),
                    "w": v.integer(frame_raw.and_then(|m| m.get("w")), &format!("{path}.frame.w"), def_frame["w"].as_i64().unwrap(), 1, display_width),
                    "h": v.integer(frame_raw.and_then(|m| m.get("h")), &format!("{path}.frame.h"), def_frame["h"].as_i64().unwrap(), 1, display_height)
                });

                let mut style = Map::new();
                let def_style = defaults.get("style").and_then(|v| v.as_object()).unwrap();

                if let Some(def_color) = def_style.get("color").and_then(|v| v.as_str()) {
                    let color = v.color(style_raw.and_then(|m| m.get("color")), &format!("{path}.style.color"), def_color);
                    style.insert("color".to_string(), json!(color));
                }

                if let Some(def_fs) = def_style.get("fontSize").and_then(|v| v.as_i64()) {
                    let fs = v.integer(style_raw.and_then(|m| m.get("fontSize")), &format!("{path}.style.fontSize"), def_fs, 6, 120);
                    style.insert("fontSize".to_string(), json!(fs));
                }

                if ["text", "clock", "external-text", "ha-state", "script"].contains(&etype.as_str()) {
                    let mut align = v.string(style_raw.and_then(|m| m.get("align")), &format!("{path}.style.align"), "center", 8);
                    let mut vertical = v.string(style_raw.and_then(|m| m.get("verticalAlign")), &format!("{path}.style.verticalAlign"), "middle", 8);
                    if !["left", "center", "right"].contains(&align.as_str()) {
                        v.error(&format!("{path}.style.align"), "must be left, center or right");
                        align = "center".to_string();
                    }
                    if !["top", "middle", "bottom"].contains(&vertical.as_str()) {
                        v.error(&format!("{path}.style.verticalAlign"), "must be top, middle or bottom");
                        vertical = "middle".to_string();
                    }
                    style.insert("align".to_string(), json!(align));
                    style.insert("verticalAlign".to_string(), json!(vertical));
                }

                if ["color-block", "button", "design-link", "ha-toggle"].contains(&etype.as_str()) {
                    let def_rad = def_style.get("radius").and_then(|v| v.as_i64()).unwrap_or(0);
                    let rad = v.integer(style_raw.and_then(|m| m.get("radius")), &format!("{path}.style.radius"), def_rad, 0, 50);
                    style.insert("radius".to_string(), json!(rad));
                }

                if etype == "ha-toggle" {
                    let def_off = def_style.get("offColor").and_then(|v| v.as_str()).unwrap_or("#353b48");
                    let off_color = v.color(style_raw.and_then(|m| m.get("offColor")), &format!("{path}.style.offColor"), def_off);
                    style.insert("offColor".to_string(), json!(off_color));
                }

                let props = match etype.as_str() {
                    "text" => {
                        let text = v.string(props_raw.and_then(|m| m.get("text")), &format!("{path}.props.text"), "", 1000);
                        let wrap = props_raw.and_then(|m| m.get("wrap")).and_then(|v| v.as_bool()).unwrap_or(true);
                        json!({"text": text, "wrap": wrap})
                    }
                    "clock" => {
                        let format = v.string(props_raw.and_then(|m| m.get("format")), &format!("{path}.props.format"), "%H:%M:%S", 100);
                        let tz = v.string(props_raw.and_then(|m| m.get("timezone")), &format!("{path}.props.timezone"), "", 64);
                        json!({"format": format, "timezone": tz})
                    }
                    "external-text" => {
                        let source_id = v.string(props_raw.and_then(|m| m.get("sourceId")), &format!("{path}.props.sourceId"), "", 64);
                        if !source_id.is_empty() && !source_ids.contains(&source_id) {
                            v.error(&format!("{path}.props.sourceId"), "references an unknown data source");
                        }
                        let wrap = props_raw.and_then(|m| m.get("wrap")).and_then(|v| v.as_bool()).unwrap_or(true);
                        json!({"sourceId": source_id, "wrap": wrap})
                    }
                    "button" => {
                        let action_id = v.string(props_raw.and_then(|m| m.get("actionId")), &format!("{path}.props.actionId"), "", 64);
                        if !action_id.is_empty() && !action_ids.contains(&action_id) {
                            v.error(&format!("{path}.props.actionId"), "references an unknown action");
                        }
                        let label = v.string(props_raw.and_then(|m| m.get("label")), &format!("{path}.props.label"), "Button", 100);
                        json!({"label": label, "actionId": action_id})
                    }
                    "design-link" => {
                        let mut target = v.string(props_raw.and_then(|m| m.get("targetDesignId")), &format!("{path}.props.targetDesignId"), "default", 64);
                        if !TARGET_DESIGN_RE.is_match(&target) {
                            v.error(&format!("{path}.props.targetDesignId"), "must be a valid design ID");
                            target = "default".to_string();
                        }
                        let label = v.string(props_raw.and_then(|m| m.get("label")), &format!("{path}.props.label"), "Open screen", 100);
                        json!({"label": label, "targetDesignId": target})
                    }
                    "image" => {
                        let mut fit = v.string(props_raw.and_then(|m| m.get("fit")), &format!("{path}.props.fit"), "contain", 8);
                        if !["contain", "cover", "stretch"].contains(&fit.as_str()) {
                            v.error(&format!("{path}.props.fit"), "must be contain, cover or stretch");
                            fit = "contain".to_string();
                        }
                        let src = v.string(props_raw.and_then(|m| m.get("src")), &format!("{path}.props.src"), "", 2048);
                        json!({"src": src, "fit": fit})
                    }
                    "ha-state" => {
                        let entity_id = v.string(props_raw.and_then(|m| m.get("entityId")), &format!("{path}.props.entityId"), "sensor.example", 255);
                        if !HA_ENTITY_RE.is_match(&entity_id) {
                            v.error(&format!("{path}.props.entityId"), "must be a Home Assistant entity ID such as sensor.temperature");
                        }
                        let attribute = v.string(props_raw.and_then(|m| m.get("attribute")), &format!("{path}.props.attribute"), "", 100);
                        let prefix = v.string(props_raw.and_then(|m| m.get("prefix")), &format!("{path}.props.prefix"), "", 100);
                        let suffix = v.string(props_raw.and_then(|m| m.get("suffix")), &format!("{path}.props.suffix"), "", 100);
                        let decimals = v.integer(props_raw.and_then(|m| m.get("decimals")), &format!("{path}.props.decimals"), 1, 0, 6);
                        let show_unit = props_raw.and_then(|m| m.get("showUnit")).and_then(|v| v.as_bool()).unwrap_or(true);
                        let refresh_interval = v.integer(props_raw.and_then(|m| m.get("refreshInterval")), &format!("{path}.props.refreshInterval"), 5, 1, 3600);
                        let wrap = props_raw.and_then(|m| m.get("wrap")).and_then(|v| v.as_bool()).unwrap_or(true);
                        json!({
                            "entityId": entity_id, "attribute": attribute, "prefix": prefix,
                            "suffix": suffix, "decimals": decimals, "showUnit": show_unit,
                            "refreshInterval": refresh_interval, "wrap": wrap
                        })
                    }
                    "ha-toggle" => {
                        let entity_id = v.string(props_raw.and_then(|m| m.get("entityId")), &format!("{path}.props.entityId"), "light.example", 255);
                        if !HA_ENTITY_RE.is_match(&entity_id) {
                            v.error(&format!("{path}.props.entityId"), "must be a Home Assistant entity ID such as light.kitchen");
                        } else {
                            let domain = entity_id.split('.').next().unwrap_or("");
                            if !["light", "switch", "input_boolean", "fan"].contains(&domain) {
                                v.error(&format!("{path}.props.entityId"), "must be a light, switch, input_boolean, or fan entity");
                            }
                        }
                        let label = v.string(props_raw.and_then(|m| m.get("label")), &format!("{path}.props.label"), "Light", 100);
                        let refresh_interval = v.integer(props_raw.and_then(|m| m.get("refreshInterval")), &format!("{path}.props.refreshInterval"), 2, 1, 3600);
                        json!({
                            "entityId": entity_id, "label": label, "refreshInterval": refresh_interval
                        })
                    }
                    "script" => {
                        let code = v.string(props_raw.and_then(|m| m.get("code")), &format!("{path}.props.code"), "text(\"Hello from a script\")", 8000);
                        let refresh_interval = v.integer(props_raw.and_then(|m| m.get("refreshInterval")), &format!("{path}.props.refreshInterval"), 5, 1, 3600);
                        let wrap = props_raw.and_then(|m| m.get("wrap")).and_then(|v| v.as_bool()).unwrap_or(true);
                        json!({
                            "code": code, "refreshInterval": refresh_interval, "wrap": wrap
                        })
                    }
                    _ => json!({}),
                };

                let def_name = defaults.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let el_name = v.string(element.get("name"), &format!("{path}.name"), def_name, 100);
                let visible = element.get("visible").and_then(|v| v.as_bool()).unwrap_or(true);
                let locked = element.get("locked").and_then(|v| v.as_bool()).unwrap_or(false);

                result["elements"].as_array_mut().unwrap().push(json!({
                    "id": id,
                    "type": etype,
                    "name": el_name,
                    "visible": visible,
                    "locked": locked,
                    "frame": frame,
                    "style": Value::Object(style),
                    "props": props
                }));
            }
        } else {
            v.error("elements", "must be an array");
        }
    }

    if !v.errors.is_empty() {
        Err(LayoutValidationError { errors: v.errors })
    } else {
        Ok(result)
    }
}

pub fn find_by_id<'a>(items: &'a [Value], item_id: &str) -> Option<&'a Value> {
    items.iter().find(|item| item.get("id").and_then(|v| v.as_str()) == Some(item_id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_layout_is_valid() {
        let layout = default_layout();
        let validated = validate_layout(layout).unwrap();
        assert_eq!(validated["schemaVersion"], SCHEMA_VERSION);
    }
}

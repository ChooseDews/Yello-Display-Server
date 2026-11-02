//! Versioned layout validation and migration.

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::HashSet;

/// The design schema: a design document (schemaVersion 2) contains
/// pages + shared elements. Page bodies carry no version of their own,
/// and no other document format is accepted.
pub const DESIGN_SCHEMA_VERSION: i32 = 2;
pub const MAX_PAGES: usize = 50;
pub const DEFAULT_PAGE_ID: &str = "main";
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
    static ref TARGET_DESIGN_RE: Regex = Regex::new(r"^[A-Za-z0-9._:/-]{1,64}$").unwrap();
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
        "screen" => {
            defaults.insert("frame".to_string(), json!({"x": 20, "y": 90, "w": 120, "h": 160}));
            defaults.insert("style".to_string(), json!({}));
            defaults.insert("props".to_string(), json!({"designId": "", "pageId": ""}));
        }
        "gauge" => {
            defaults.insert("frame".to_string(), json!({"x": 20, "y": 150, "w": 200, "h": 36}));
            defaults.insert(
                "style".to_string(),
                json!({"color": "#57c785", "trackColor": "#2a2f3a", "fontSize": 14}),
            );
            defaults.insert(
                "props".to_string(),
                json!({
                    "sourceKind": "static", "value": 65,
                    "sourceId": "", "entityId": "sensor.example",
                    "min": 0, "max": 100, "decimals": 0, "unit": "",
                    "showValue": true, "refreshInterval": 5
                }),
            );
        }
        "qr" => {
            defaults.insert("frame".to_string(), json!({"x": 70, "y": 110, "w": 100, "h": 100}));
            defaults.insert(
                "style".to_string(),
                json!({"color": "#111111", "bgColor": "#ffffff"}),
            );
            defaults.insert("props".to_string(), json!({"text": "https://example.com"}));
        }
        "shape" => {
            defaults.insert("frame".to_string(), json!({"x": 80, "y": 130, "w": 80, "h": 60}));
            defaults.insert(
                "style".to_string(),
                json!({"color": "#8e6bc8", "radius": 0, "outlineColor": "", "outlineWidth": 0}),
            );
            defaults.insert("props".to_string(), json!({"shape": "rect"}));
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
        "name": "Yello screen",
        "profile": "ili9341-240x320",
        "orientation": "portrait",
        "background": "#0a0a1e",
        "dataSources": [],
        "actions": [],
        "elements": [clock, date, divider, greeting]
    })
}

/// The canonical default design: a schema-v2 document whose single
/// "Main" page holds the default layout's elements. New studio files
/// are seeded with this; designs have no other format.
pub fn default_design() -> Value {
    let page = default_layout();
    let background = page.get("background").cloned().unwrap_or_else(|| json!("#0a0a1e"));
    json!({
        "schemaVersion": DESIGN_SCHEMA_VERSION,
        "name": page.get("name").cloned().unwrap_or_else(|| json!("Yello screen")),
        "profile": page.get("profile").cloned().unwrap_or_else(|| json!("ili9341-240x320")),
        "orientation": page.get("orientation").cloned().unwrap_or_else(|| json!("portrait")),
        "background": background,
        "idleHomeSeconds": 0,
        "dataSources": page.get("dataSources").cloned().unwrap_or_else(|| json!([])),
        "actions": page.get("actions").cloned().unwrap_or_else(|| json!([])),
        "homePageId": DEFAULT_PAGE_ID,
        "pages": [{
            "id": DEFAULT_PAGE_ID,
            "name": "Main",
            "background": background,
            "elements": page.get("elements").cloned().unwrap_or_else(|| json!([]))
        }],
        "sharedElements": []
    })
}

pub fn layout_dimensions(layout: &Value) -> (u16, u16) {
    if layout.get("orientation").and_then(|v| v.as_str()) == Some("landscape") {
        (LANDSCAPE_WIDTH, LANDSCAPE_HEIGHT)
    } else {
        (DISPLAY_WIDTH, DISPLAY_HEIGHT)
    }
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
    if !raw.is_object() {
        return Err(LayoutValidationError::new("must be an object"));
    }

    let mut v = Validator::new();

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
        "screen", "gauge", "qr", "shape",
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

                if ["color-block", "button", "design-link", "ha-toggle", "shape"].contains(&etype.as_str()) {
                    let def_rad = def_style.get("radius").and_then(|v| v.as_i64()).unwrap_or(0);
                    let rad = v.integer(style_raw.and_then(|m| m.get("radius")), &format!("{path}.style.radius"), def_rad, 0, 50);
                    style.insert("radius".to_string(), json!(rad));
                }

                if etype == "ha-toggle" {
                    let def_off = def_style.get("offColor").and_then(|v| v.as_str()).unwrap_or("#353b48");
                    let off_color = v.color(style_raw.and_then(|m| m.get("offColor")), &format!("{path}.style.offColor"), def_off);
                    style.insert("offColor".to_string(), json!(off_color));
                }

                if etype == "gauge" {
                    let def_track = def_style.get("trackColor").and_then(|v| v.as_str()).unwrap_or("#2a2f3a");
                    let track = v.color(style_raw.and_then(|m| m.get("trackColor")), &format!("{path}.style.trackColor"), def_track);
                    style.insert("trackColor".to_string(), json!(track));
                }

                if etype == "qr" {
                    let def_bg = def_style.get("bgColor").and_then(|v| v.as_str()).unwrap_or("#ffffff");
                    let bg = v.color(style_raw.and_then(|m| m.get("bgColor")), &format!("{path}.style.bgColor"), def_bg);
                    style.insert("bgColor".to_string(), json!(bg));
                }

                if etype == "shape" {
                    // Outline color may be empty (= no outline).
                    let raw_oc = style_raw.and_then(|m| m.get("outlineColor")).and_then(|x| x.as_str()).unwrap_or("");
                    let oc = if raw_oc.is_empty() {
                        String::new()
                    } else {
                        v.color(style_raw.and_then(|m| m.get("outlineColor")), &format!("{path}.style.outlineColor"), "#ffffff")
                    };
                    style.insert("outlineColor".to_string(), json!(oc));
                    let ow = v.integer(style_raw.and_then(|m| m.get("outlineWidth")), &format!("{path}.style.outlineWidth"), 0, 0, 20);
                    style.insert("outlineWidth".to_string(), json!(ow));
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
                    "screen" => {
                        let design_id = v.string(props_raw.and_then(|m| m.get("designId")), &format!("{path}.props.designId"), "", 64);
                        if !design_id.is_empty() && !design_id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
                            v.error(&format!("{path}.props.designId"), "must be a design ID or empty (this design)");
                        }
                        let page_id = v.string(props_raw.and_then(|m| m.get("pageId")), &format!("{path}.props.pageId"), "", 64);
                        if !page_id.is_empty() && !page_id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
                            v.error(&format!("{path}.props.pageId"), "must be a page ID or empty (home page)");
                        }
                        json!({"designId": design_id, "pageId": page_id})
                    }
                    "gauge" => {
                        let mut kind = v.string(props_raw.and_then(|m| m.get("sourceKind")), &format!("{path}.props.sourceKind"), "static", 16);
                        if !["static", "external", "ha"].contains(&kind.as_str()) {
                            v.error(&format!("{path}.props.sourceKind"), "must be static, external or ha");
                            kind = "static".to_string();
                        }
                        let num = |key: &str, def: f64| -> f64 {
                            props_raw.and_then(|m| m.get(key)).and_then(|x| x.as_f64()).unwrap_or(def)
                        };
                        let value = num("value", 0.0);
                        let min = num("min", 0.0);
                        let max = num("max", 100.0);
                        if max <= min {
                            v.error(&format!("{path}.props.max"), "must be greater than min");
                        }
                        let source_id = v.string(props_raw.and_then(|m| m.get("sourceId")), &format!("{path}.props.sourceId"), "", 64);
                        if kind == "external" && !source_id.is_empty() && !source_ids.contains(&source_id) {
                            v.error(&format!("{path}.props.sourceId"), "references an unknown data source");
                        }
                        let entity_id = v.string(props_raw.and_then(|m| m.get("entityId")), &format!("{path}.props.entityId"), "sensor.example", 255);
                        if kind == "ha" && !HA_ENTITY_RE.is_match(&entity_id) {
                            v.error(&format!("{path}.props.entityId"), "must be a Home Assistant entity ID such as sensor.temperature");
                        }
                        let decimals = v.integer(props_raw.and_then(|m| m.get("decimals")), &format!("{path}.props.decimals"), 0, 0, 6);
                        let unit = v.string(props_raw.and_then(|m| m.get("unit")), &format!("{path}.props.unit"), "", 16);
                        let show_value = props_raw.and_then(|m| m.get("showValue")).and_then(|x| x.as_bool()).unwrap_or(true);
                        let refresh_interval = v.integer(props_raw.and_then(|m| m.get("refreshInterval")), &format!("{path}.props.refreshInterval"), 5, 1, 3600);
                        json!({
                            "sourceKind": kind, "value": value, "sourceId": source_id,
                            "entityId": entity_id, "min": min, "max": max,
                            "decimals": decimals, "unit": unit, "showValue": show_value,
                            "refreshInterval": refresh_interval
                        })
                    }
                    "qr" => {
                        let text = v.string(props_raw.and_then(|m| m.get("text")), &format!("{path}.props.text"), "", 1024);
                        json!({"text": text})
                    }
                    "shape" => {
                        let mut shape = v.string(props_raw.and_then(|m| m.get("shape")), &format!("{path}.props.shape"), "rect", 16);
                        if !["rect", "circle", "ellipse"].contains(&shape.as_str()) {
                            v.error(&format!("{path}.props.shape"), "must be rect, circle or ellipse");
                            shape = "rect".to_string();
                        }
                        json!({"shape": shape})
                    }
                    _ => json!({}),
                };

                let def_name = defaults.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let el_name = v.string(element.get("name"), &format!("{path}.name"), def_name, 100);
                let visible = element.get("visible").and_then(|v| v.as_bool()).unwrap_or(true);
                let locked = element.get("locked").and_then(|v| v.as_bool()).unwrap_or(false);
                // Editor-only organization: the Layers panel groups
                // elements by this label. Rendering ignores it; the key
                // is omitted entirely when empty so ungrouped designs
                // stay byte-identical.
                let group = v.string(element.get("group"), &format!("{path}.group"), "", 40);

                let mut el_obj = json!({
                    "id": id,
                    "type": etype,
                    "name": el_name,
                    "visible": visible,
                    "locked": locked,
                    "frame": frame,
                    "style": Value::Object(style),
                    "props": props
                });
                if !group.is_empty() {
                    el_obj["group"] = json!(group);
                }
                result["elements"].as_array_mut().unwrap().push(el_obj);
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

/// Navigation target of a design-link element (Tier 2):
/// `__back`, `__home`, `page:<pageId>` (same design),
/// `<designId>` (that design's home page), or `<designId>/<pageId>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NavTarget {
    Back,
    Home,
    Page(String),
    Design(String),
    DesignPage(String, String),
}

pub fn parse_nav_target(target: &str) -> NavTarget {
    match target {
        "__back" => return NavTarget::Back,
        "__home" => return NavTarget::Home,
        _ => {}
    }
    if let Some(page_id) = target.strip_prefix("page:") {
        return NavTarget::Page(page_id.to_string());
    }
    if let Some((design_id, page_id)) = target.split_once('/') {
        return NavTarget::DesignPage(design_id.to_string(), page_id.to_string());
    }
    NavTarget::Design(target.to_string())
}

/// The design's home page id, resolved against its actual pages.
pub fn design_home_page_id(design: &Value) -> String {
    if let Some(pages) = design.get("pages").and_then(|p| p.as_array()) {
        if let Some(home) = design.get("homePageId").and_then(|v| v.as_str()) {
            if pages.iter().any(|p| p.get("id").and_then(|v| v.as_str()) == Some(home)) {
                return home.to_string();
            }
        }
        if let Some(first) = pages.first().and_then(|p| p.get("id")).and_then(|v| v.as_str()) {
            return first.to_string();
        }
    }
    DEFAULT_PAGE_ID.to_string()
}

/// Build the effective v1-shaped layout for one page of a v2 design: the
/// page's own elements plus the shared elements that explicitly opted
/// into this page (shared elements render below page elements). Shared
/// elements are NEVER included implicitly. Returns None when the design
/// has no pages at all.
pub fn effective_page_layout(design: &Value, page_id: &str) -> Option<Value> {
    let pages = design.get("pages").and_then(|p| p.as_array())?;
    let page = pages
        .iter()
        .find(|p| p.get("id").and_then(|v| v.as_str()) == Some(page_id))
        .or_else(|| {
            let home = design_home_page_id(design);
            pages.iter().find(|p| p.get("id").and_then(|v| v.as_str()) == Some(home.as_str()))
        })
        .or_else(|| pages.first())?;
    let resolved_page_id = page.get("id").and_then(|v| v.as_str()).unwrap_or(page_id);

    let mut elements: Vec<Value> = Vec::new();
    if let Some(shared) = design.get("sharedElements").and_then(|s| s.as_array()) {
        for el in shared {
            let included = el
                .get("pages")
                .and_then(|p| p.as_array())
                .map(|arr| arr.iter().any(|x| x.as_str() == Some(resolved_page_id)))
                .unwrap_or(false);
            if included {
                let mut el = el.clone();
                if let Some(obj) = el.as_object_mut() {
                    obj.remove("pages");
                }
                elements.push(el);
            }
        }
    }
    if let Some(els) = page.get("elements").and_then(|e| e.as_array()) {
        elements.extend(els.iter().cloned());
    }

    Some(json!({
        "name": page.get("name").cloned().unwrap_or_else(|| design.get("name").cloned().unwrap_or_else(|| json!("Yello screen"))),
        "profile": design.get("profile").cloned().unwrap_or_else(|| json!("ili9341-240x320")),
        "orientation": design.get("orientation").cloned().unwrap_or_else(|| json!("portrait")),
        "background": page.get("background").cloned().unwrap_or_else(|| design.get("background").cloned().unwrap_or_else(|| json!("#0a0a1e"))),
        "dataSources": design.get("dataSources").cloned().unwrap_or_else(|| json!([])),
        "actions": design.get("actions").cloned().unwrap_or_else(|| json!([])),
        "elements": Value::Array(elements)
    }))
}

/// Validate a schema-v2 design document (pages + opt-in shared
/// elements). The v1 layout format is not accepted. Page element
/// validation reuses the page validator on a synthetic layout carrying
/// the design's data sources and actions, so cross-references
/// (sourceId / actionId) check exactly as before.
pub fn validate_design(raw: Value) -> Result<Value, LayoutValidationError> {
    if !raw.is_object() {
        return Err(LayoutValidationError::new("must be an object"));
    }
    let is_v2 = raw.get("schemaVersion").and_then(|x| x.as_i64()) == Some(DESIGN_SCHEMA_VERSION as i64)
        || raw.get("pages").is_some();
    if !is_v2 {
        return Err(LayoutValidationError::new(
            "must be a schema v2 design document (schemaVersion 2 with pages[]); the v1 layout format is no longer accepted",
        ));
    }

    let mut v = Validator::new();
    let mut errors: Vec<LayoutError> = Vec::new();

    if raw.get("schemaVersion").and_then(|x| x.as_i64()) != Some(DESIGN_SCHEMA_VERSION as i64) {
        v.error("schemaVersion", &format!("must equal {DESIGN_SCHEMA_VERSION}"));
    }

    // Header + data sources + actions, normalized by the page validator.
    // Only keys actually present are copied over: a JSON null would fail
    // the v1 string checks, while an absent key takes the default.
    let mut skeleton = json!({
        "dataSources": raw.get("dataSources").cloned().unwrap_or_else(|| json!([])),
        "actions": raw.get("actions").cloned().unwrap_or_else(|| json!([])),
        "elements": []
    });
    for key in ["name", "profile", "orientation", "background"] {
        if let Some(val) = raw.get(key) {
            skeleton[key] = val.clone();
        }
    }
    let head = match validate_layout(skeleton) {
        Ok(h) => h,
        Err(e) => {
            errors.extend(e.errors);
            json!({
                "name": "Yello screen", "profile": "ili9341-240x320",
                "orientation": "portrait", "background": "#0a0a1e",
                "dataSources": [], "actions": []
            })
        }
    };
    let orientation = head["orientation"].as_str().unwrap_or("portrait").to_string();
    let design_background = head["background"].as_str().unwrap_or("#0a0a1e").to_string();

    let idle_home_seconds = v.integer(raw.get("idleHomeSeconds"), "idleHomeSeconds", 0, 0, 86400);

    // ---- Pages ----
    let mut pages_out: Vec<Value> = Vec::new();
    let mut page_ids: Vec<String> = Vec::new();
    match raw.get("pages") {
        Some(Value::Array(raw_pages)) => {
            if raw_pages.is_empty() {
                v.error("pages", "must contain at least one page");
            }
            if raw_pages.len() > MAX_PAGES {
                v.error("pages", &format!("must contain at most {MAX_PAGES} pages"));
            }
            for (index, raw_page) in raw_pages.iter().take(MAX_PAGES).enumerate() {
                let path = format!("pages[{index}]");
                if !raw_page.is_object() {
                    v.error(&path, "must be an object");
                    continue;
                }
                let fallback = format!("page{}", index + 1);
                let id = v.identifier(raw_page.get("id"), &format!("{path}.id"), &fallback);
                if page_ids.contains(&id) {
                    v.error(&format!("{path}.id"), "must be unique");
                }
                page_ids.push(id.clone());
                let page_name = v.string(raw_page.get("name"), &format!("{path}.name"), "Page", 100);

                let page_skeleton = json!({
                    "name": head["name"],
                    "profile": head["profile"],
                    "orientation": orientation,
                    "background": raw_page.get("background").cloned().unwrap_or_else(|| json!(design_background.clone())),
                    "dataSources": raw.get("dataSources").cloned().unwrap_or_else(|| json!([])),
                    "actions": raw.get("actions").cloned().unwrap_or_else(|| json!([])),
                    "elements": raw_page.get("elements").cloned().unwrap_or_else(|| json!([]))
                });
                let (page_background, page_elements) = match validate_layout(page_skeleton) {
                    Ok(norm) => (
                        norm["background"].as_str().unwrap_or(&design_background).to_string(),
                        norm["elements"].clone(),
                    ),
                    Err(e) => {
                        for err in e.errors {
                            if err.path.starts_with("elements") {
                                errors.push(LayoutError {
                                    path: format!("{path}.{}", err.path),
                                    message: err.message,
                                });
                            } else if err.path == "background" {
                                errors.push(LayoutError {
                                    path: format!("{path}.background"),
                                    message: err.message,
                                });
                            }
                        }
                        (design_background.clone(), json!([]))
                    }
                };

                pages_out.push(json!({
                    "id": id,
                    "name": page_name,
                    "background": page_background,
                    "elements": page_elements
                }));
            }
        }
        Some(_) => v.error("pages", "must be an array"),
        None => v.error("pages", "must be an array"),
    }

    // ---- Home page ----
    let home_default = page_ids.first().cloned().unwrap_or_else(|| DEFAULT_PAGE_ID.to_string());
    let home_page_id = v.string(raw.get("homePageId"), "homePageId", &home_default, 64);
    let home_page_id = if page_ids.contains(&home_page_id) {
        home_page_id
    } else {
        if raw.get("homePageId").is_some() {
            v.error("homePageId", "must be the id of a page in this design");
        }
        home_default.clone()
    };

    // ---- Shared elements (opt-in per page; default: no pages) ----
    let mut shared_out: Vec<Value> = Vec::new();
    if let Some(shared_raw) = raw.get("sharedElements") {
        match shared_raw.as_array() {
            Some(shared_arr) => {
                let shared_skeleton = json!({
                    "name": head["name"],
                    "profile": head["profile"],
                    "orientation": orientation,
                    "background": design_background,
                    "dataSources": raw.get("dataSources").cloned().unwrap_or_else(|| json!([])),
                    "actions": raw.get("actions").cloned().unwrap_or_else(|| json!([])),
                    "elements": Value::Array(shared_arr.clone())
                });
                match validate_layout(shared_skeleton) {
                    Ok(norm) => {
                        let norm_elements = norm["elements"].as_array().cloned().unwrap_or_default();
                        for (index, mut el) in norm_elements.into_iter().enumerate() {
                            let mut member_pages: Vec<String> = Vec::new();
                            if let Some(pages_val) = shared_arr.get(index).and_then(|e| e.get("pages")) {
                                match pages_val.as_array() {
                                    Some(arr) => {
                                        for entry in arr {
                                            match entry.as_str() {
                                                Some(pid) if page_ids.iter().any(|p| p == pid) => {
                                                    if !member_pages.contains(&pid.to_string()) {
                                                        member_pages.push(pid.to_string());
                                                    }
                                                }
                                                _ => v.error(
                                                    &format!("sharedElements[{index}].pages"),
                                                    "must list only page ids in this design",
                                                ),
                                            }
                                        }
                                    }
                                    None => v.error(
                                        &format!("sharedElements[{index}].pages"),
                                        "must be an array of page ids",
                                    ),
                                }
                            }
                            if let Some(obj) = el.as_object_mut() {
                                obj.insert("pages".to_string(), json!(member_pages));
                            }
                            shared_out.push(el);
                        }
                    }
                    Err(e) => {
                        for err in e.errors {
                            if err.path.starts_with("elements") {
                                errors.push(LayoutError {
                                    path: format!("sharedElements{}", &err.path["elements".len()..]),
                                    message: err.message,
                                });
                            }
                        }
                    }
                }
            }
            None => v.error("sharedElements", "must be an array"),
        }
    }

    // ---- Internal references must point at real pages: design-link
    // page: targets, and embedded screens pointing at this design. ----
    let check_targets = |elements: &Value, root: &str, v: &mut Validator| {
        if let Some(arr) = elements.as_array() {
            for (index, el) in arr.iter().enumerate() {
                let etype = el.get("type").and_then(|t| t.as_str()).unwrap_or("");
                if etype == "design-link" {
                    let target = el
                        .get("props")
                        .and_then(|p| p.get("targetDesignId"))
                        .and_then(|t| t.as_str())
                        .unwrap_or("");
                    if let NavTarget::Page(pid) = parse_nav_target(target) {
                        if !page_ids.contains(&pid) {
                            v.error(
                                &format!("{root}[{index}].props.targetDesignId"),
                                "references an unknown page in this design",
                            );
                        }
                    }
                } else if etype == "screen" {
                    let props = el.get("props").unwrap_or(&Value::Null);
                    let target_design = props.get("designId").and_then(|t| t.as_str()).unwrap_or("");
                    let target_page = props.get("pageId").and_then(|t| t.as_str()).unwrap_or("");
                    if target_design.is_empty() && !target_page.is_empty() && !page_ids.iter().any(|p| p.as_str() == target_page) {
                        v.error(
                            &format!("{root}[{index}].props.pageId"),
                            "references an unknown page in this design",
                        );
                    }
                }
            }
        }
    };
    for (index, page) in pages_out.iter().enumerate() {
        check_targets(&page["elements"], &format!("pages[{index}].elements"), &mut v);
    }
    check_targets(&Value::Array(shared_out.clone()), "sharedElements", &mut v);

    // ---- Shared element ids must not collide with a page's own elements
    // on any page the shared element is included on ----
    for (s_index, shared_el) in shared_out.iter().enumerate() {
        let shared_id = shared_el.get("id").and_then(|i| i.as_str()).unwrap_or("");
        let member_pages: Vec<&str> = shared_el
            .get("pages")
            .and_then(|p| p.as_array())
            .map(|arr| arr.iter().filter_map(|x| x.as_str()).collect())
            .unwrap_or_default();
        for page in &pages_out {
            let pid = page.get("id").and_then(|i| i.as_str()).unwrap_or("");
            if !member_pages.contains(&pid) {
                continue;
            }
            if let Some(elements) = page["elements"].as_array() {
                if elements.iter().any(|e| e.get("id").and_then(|i| i.as_str()) == Some(shared_id)) {
                    v.error(
                        &format!("sharedElements[{s_index}].id"),
                        &format!("duplicates an element id on page '{pid}' where it is included"),
                    );
                }
            }
        }
    }

    errors.extend(v.errors);
    if !errors.is_empty() {
        return Err(LayoutValidationError { errors });
    }

    Ok(json!({
        "schemaVersion": DESIGN_SCHEMA_VERSION,
        "name": head["name"],
        "profile": head["profile"],
        "orientation": orientation,
        "background": design_background,
        "idleHomeSeconds": idle_home_seconds,
        "dataSources": head["dataSources"],
        "actions": head["actions"],
        "homePageId": home_page_id,
        "pages": pages_out,
        "sharedElements": shared_out
    }))
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
        // Page bodies carry no schema version; the version lives on the
        // design document (DESIGN_SCHEMA_VERSION).
        assert!(validated.get("schemaVersion").is_none());
    }

    #[test]
    fn test_default_design_is_valid_v2() {
        let design = default_design();
        let validated = validate_design(design).unwrap();
        assert_eq!(validated["schemaVersion"], DESIGN_SCHEMA_VERSION);
        assert_eq!(validated["pages"].as_array().unwrap().len(), 1);
        assert_eq!(validated["homePageId"], DEFAULT_PAGE_ID);
    }

    #[test]
    fn test_validate_design_rejects_v1_layout() {
        let err = validate_design(default_layout()).unwrap_err();
        assert!(
            format!("{err:?}").contains("no longer accepted"),
            "unexpected error: {err:?}"
        );
    }
}

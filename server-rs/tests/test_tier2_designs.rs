//! Tier 2 tests: v2 designs with pages, opt-in shared elements,
//! navigation stack, and cross-design link validation.

use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;

use hyper::{Body, Method, Request, StatusCode};
use serde_json::{json, Value};
use tokio::sync::Mutex;

use yello_server::layout_model::{
    design_home_page_id, effective_page_layout, parse_nav_target, validate_design, NavTarget,
    DEFAULT_PAGE_ID,
};
use yello_server::server::{apply_navigation, AppState, DeviceSession};
use yello_server::{default_design, default_layout, handle_http_request, DEFAULT_DESIGN_ID};

fn static_dir() -> PathBuf {
    let mut d = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    d.push("static");
    d
}

fn make_state() -> Arc<AppState> {
    Arc::new(AppState::new(
        PathBuf::from("/tmp/test_studio_tier2.json"),
        PathBuf::from("/tmp/test_secrets_tier2.json"),
        static_dir(),
    ))
}

/// A v2 design with two pages and one shared element opted into page 2 only.
fn two_page_design() -> Value {
    json!({
        "schemaVersion": 2,
        "name": "Two Page",
        "orientation": "portrait",
        "background": "#0a0a1e",
        "homePageId": "main",
        "idleHomeSeconds": 0,
        "pages": [
            {"id": "main", "name": "Main", "background": "#0a0a1e", "elements": [
                {"type": "design-link", "id": "go2", "frame": {"x": 0, "y": 0, "w": 240, "h": 60},
                 "style": {}, "props": {"label": "Go", "targetDesignId": "page:second"}}
            ]},
            {"id": "second", "name": "Second", "background": "#101040", "elements": [
                {"type": "text", "id": "t2", "frame": {"x": 0, "y": 0, "w": 240, "h": 40},
                 "style": {}, "props": {"text": "Page two"}}
            ]}
        ],
        "sharedElements": [
            {"type": "text", "id": "hdr", "frame": {"x": 0, "y": 280, "w": 240, "h": 40},
             "style": {}, "props": {"text": "Header"}, "pages": ["second"]}
        ]
    })
}

#[test]
fn v1_layout_is_rejected() {
    let err = validate_design(default_layout()).unwrap_err();
    assert!(format!("{err:?}").contains("no longer accepted"), "unexpected: {err:?}");
}

#[test]
fn default_design_is_a_valid_single_page_design() {
    let out = validate_design(default_design()).expect("valid");
    assert_eq!(out["schemaVersion"], 2);
    assert_eq!(out["homePageId"], DEFAULT_PAGE_ID);
    let pages = out["pages"].as_array().unwrap();
    assert_eq!(pages.len(), 1);
    assert_eq!(pages[0]["id"], DEFAULT_PAGE_ID);
    let default_elements = default_design()["pages"][0]["elements"].as_array().unwrap().len();
    assert_eq!(pages[0]["elements"].as_array().unwrap().len(), default_elements);
    assert!(out["sharedElements"].as_array().unwrap().is_empty());
    assert_eq!(design_home_page_id(&out), DEFAULT_PAGE_ID);
}

#[test]
fn shared_elements_are_opt_in_per_page() {
    let design = validate_design(two_page_design()).expect("valid design");
    assert_eq!(design["pages"].as_array().unwrap().len(), 2);

    let main = effective_page_layout(&design, "main").unwrap();
    let main_ids: Vec<&str> = main["elements"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_str().unwrap())
        .collect();
    assert!(main_ids.contains(&"go2"));
    assert!(!main_ids.contains(&"hdr"), "shared element must not leak onto pages that did not opt in");

    let second = effective_page_layout(&design, "second").unwrap();
    let second_ids: Vec<&str> = second["elements"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_str().unwrap())
        .collect();
    assert!(second_ids.contains(&"hdr"), "shared element must render on its opted-in page");
    assert!(second_ids.contains(&"t2"));
    // The shared element's membership list must not survive into the
    // effective layout handed to the renderer.
    assert!(second["elements"]
        .as_array()
        .unwrap()
        .iter()
        .all(|e| e.get("pages").is_none()));
    // Unknown page falls back to the home page.
    let fallback = effective_page_layout(&design, "nope").unwrap();
    assert_eq!(fallback["background"], "#0a0a1e");
}

#[test]
fn validate_design_rejects_bad_internal_references() {
    // page: link to a page that does not exist
    let mut bad = two_page_design();
    bad["pages"][0]["elements"][0]["props"]["targetDesignId"] = json!("page:ghost");
    assert!(validate_design(bad).is_err());

    // shared element opted into an unknown page
    let mut bad = two_page_design();
    bad["sharedElements"][0]["pages"] = json!(["ghost"]);
    assert!(validate_design(bad).is_err());

    // home page that does not exist
    let mut bad = two_page_design();
    bad["homePageId"] = json!("ghost");
    assert!(validate_design(bad).is_err());

    // no pages at all
    let mut bad = two_page_design();
    bad["pages"] = json!([]);
    assert!(validate_design(bad).is_err());
}

#[test]
fn nav_target_parsing() {
    assert_eq!(parse_nav_target("__back"), NavTarget::Back);
    assert_eq!(parse_nav_target("__home"), NavTarget::Home);
    assert_eq!(parse_nav_target("page:second"), NavTarget::Page("second".to_string()));
    assert_eq!(parse_nav_target("other"), NavTarget::Design("other".to_string()));
    assert_eq!(
        parse_nav_target("other/pagex"),
        NavTarget::DesignPage("other".to_string(), "pagex".to_string())
    );
}

fn make_session(device_id: &str) -> DeviceSession {
    DeviceSession {
        device_id: device_id.to_string(),
        remote: "127.0.0.1:9999".to_string(),
        last_frame: None,
        last_full_push: std::time::Instant::now(),
        pressed: HashSet::new(),
        dots: Vec::new(),
        connected_at: std::time::Instant::now(),
        connected_since: chrono::Utc::now().to_rfc3339(),
        stream_samples: VecDeque::new(),
        total_pixels: 0,
        total_wire_bytes: 0,
        total_zone_messages: 0,
        frames_sent: 0,
        last_push_ms: 0.0,
        last_frame_at: chrono::Utc::now().to_rfc3339(),
        orientation: "portrait".to_string(),
        last_ping_at: chrono::Utc::now().to_rfc3339(),
        wifi_rssi_dbm: None,
        uptime_ms: 0,
        active_design_id: DEFAULT_DESIGN_ID.to_string(),
        active_page_id: DEFAULT_PAGE_ID.to_string(),
        nav_stack: Vec::new(),
        last_interaction: std::time::Instant::now(),
        ws_sender: None,
        frame_seq: 0,
        firmware_version: None,
        device_proto_version: None,
        capabilities: 0,
        supports_compression: false,
        hello_received: false,
        push_lock: Arc::new(Mutex::new(())),
        last_tick: std::time::Instant::now(),
    }
}

async fn post(state: &Arc<AppState>, uri: &str, body: &Value) -> (StatusCode, Value) {
    let req = Request::builder()
        .method(Method::POST)
        .uri(uri)
        .body(Body::from(serde_json::to_vec(body).unwrap()))
        .unwrap();
    let resp = handle_http_request(req, state.clone()).await.unwrap();
    let status = resp.status();
    let bytes = hyper::body::to_bytes(resp.into_body()).await.unwrap();
    let val: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, val)
}

async fn session_pos(state: &Arc<AppState>, device_id: &str) -> (String, String, usize) {
    let devices = state.devices.read().await;
    let s = devices.get(device_id).unwrap();
    (s.active_design_id.clone(), s.active_page_id.clone(), s.nav_stack.len())
}

#[tokio::test]
async fn navigation_stack_page_back_home() {
    let state = make_state();
    let (status, _) = post(&state, "/api/layout?design=default", &two_page_design()).await;
    assert_eq!(status, StatusCode::OK);

    state
        .devices
        .write()
        .await
        .insert("dev-nav".to_string(), make_session("dev-nav"));

    apply_navigation(&state, "dev-nav", "page:second").await;
    assert_eq!(session_pos(&state, "dev-nav").await, (DEFAULT_DESIGN_ID.to_string(), "second".to_string(), 1));

    apply_navigation(&state, "dev-nav", "__back").await;
    assert_eq!(session_pos(&state, "dev-nav").await, (DEFAULT_DESIGN_ID.to_string(), "main".to_string(), 0));

    // Back with an empty stack stays home; unknown pages are ignored.
    apply_navigation(&state, "dev-nav", "__back").await;
    assert_eq!(session_pos(&state, "dev-nav").await.1, "main");
    apply_navigation(&state, "dev-nav", "page:ghost").await;
    assert_eq!(session_pos(&state, "dev-nav").await.1, "main");

    apply_navigation(&state, "dev-nav", "page:second").await;
    apply_navigation(&state, "dev-nav", "__home").await;
    assert_eq!(session_pos(&state, "dev-nav").await, (DEFAULT_DESIGN_ID.to_string(), "main".to_string(), 0));
}

#[tokio::test]
async fn cross_design_links_checked_at_save() {
    let state = make_state();

    // Link to a design that does not exist -> 422.
    let mut bad = two_page_design();
    bad["pages"][0]["elements"][0]["props"]["targetDesignId"] = json!("ghostdesign");
    let (status, _) = post(&state, "/api/layout?design=default", &bad).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    // Link to an unknown page of an existing design -> 422.
    let mut bad = two_page_design();
    bad["pages"][0]["elements"][0]["props"]["targetDesignId"] = json!("default/ghost");
    let (status, _) = post(&state, "/api/layout?design=default", &bad).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    // Valid in-design page link + valid cross-design home link -> 200.
    let (status, _) = post(&state, "/api/layout?design=default", &two_page_design()).await;
    assert_eq!(status, StatusCode::OK);
    let mut ok = two_page_design();
    ok["pages"][0]["elements"][0]["props"]["targetDesignId"] = json!("default/second");
    let (status, _) = post(&state, "/api/layout?design=default", &ok).await;
    assert_eq!(status, StatusCode::OK);

    // Preview of a specific page renders.
    let req = Request::builder()
        .method(Method::POST)
        .uri("/api/preview?page=second")
        .body(Body::from(serde_json::to_vec(&two_page_design()).unwrap()))
        .unwrap();
    let resp = handle_http_request(req, state.clone()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = hyper::body::to_bytes(resp.into_body()).await.unwrap();
    assert_eq!(&bytes[0..4], &[0x89, 0x50, 0x4E, 0x47]);
}

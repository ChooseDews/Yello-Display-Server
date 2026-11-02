//! Widget round: embedded screens, gauge, QR, and shape elements —
//! validation, cross-design embed checks, and pixel-level rendering.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use hyper::{Body, Method, Request, StatusCode};
use serde_json::{json, Value};

use yello_server::layout_model::{default_design, effective_page_layout, validate_design};
use yello_server::renderer;
use yello_server::server::AppState;
use yello_server::{handle_http_request, with_embed_images};

fn static_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("static")
}

fn test_state() -> Arc<AppState> {
    let unique = format!(
        "yello-widgets-{}-{:?}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let dir = std::env::temp_dir().join(unique);
    std::fs::create_dir_all(&dir).unwrap();
    Arc::new(AppState::new(
        dir.join("studio.json"),
        dir.join("secrets.json"),
        static_dir(),
    ))
}

async fn post(state: &Arc<AppState>, uri: &str, body: &Value) -> (StatusCode, Value) {
    let req = Request::builder()
        .method(Method::POST)
        .uri(uri)
        .header("Content-Type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = handle_http_request(req, state.clone()).await.unwrap();
    let status = resp.status();
    let bytes = hyper::body::to_bytes(resp.into_body()).await.unwrap();
    let val: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, val)
}

async fn get(state: &Arc<AppState>, uri: &str) -> (StatusCode, Vec<u8>) {
    let req = Request::builder()
        .method(Method::GET)
        .uri(uri)
        .body(Body::empty())
        .unwrap();
    let resp = handle_http_request(req, state.clone()).await.unwrap();
    let status = resp.status();
    let bytes = hyper::body::to_bytes(resp.into_body()).await.unwrap();
    (status, bytes.to_vec())
}

fn design_with(elements: Vec<Value>) -> Value {
    let mut design = default_design();
    design["pages"][0]["elements"] = json!(elements);
    design
}

fn element(etype: &str, props: Value, style: Value) -> Value {
    json!({
        "id": format!("el_{etype}"),
        "type": etype,
        "frame": {"x": 20, "y": 20, "w": 200, "h": 100},
        "style": style,
        "props": props
    })
}

#[test]
fn new_widget_types_validate_with_defaults() {
    let design = design_with(vec![
        element("screen", json!({}), json!({})),
        element("gauge", json!({}), json!({})),
        element("qr", json!({}), json!({})),
        element("shape", json!({}), json!({})),
    ]);
    let out = validate_design(design).expect("valid design");
    let els = out["pages"][0]["elements"].as_array().unwrap();
    assert_eq!(els[0]["props"]["designId"], "");
    assert_eq!(els[0]["props"]["pageId"], "");
    assert_eq!(els[1]["props"]["sourceKind"], "static");
    assert_eq!(els[1]["style"]["trackColor"], "#2a2f3a");
    assert_eq!(els[2]["style"]["bgColor"], "#ffffff");
    assert_eq!(els[3]["props"]["shape"], "rect");
    assert_eq!(els[3]["style"]["outlineWidth"], 0);
}

#[test]
fn gauge_rejects_bad_source_and_range() {
    let design = design_with(vec![element(
        "gauge",
        json!({"sourceKind": "bogus", "min": 50, "max": 10}),
        json!({}),
    )]);
    let err = validate_design(design).unwrap_err();
    let text = format!("{err:?}");
    assert!(text.contains("sourceKind"), "{text}");
    assert!(text.contains("greater than min"), "{text}");

    let design = design_with(vec![element(
        "gauge",
        json!({"sourceKind": "ha", "entityId": "not an entity"}),
        json!({}),
    )]);
    assert!(validate_design(design).is_err());

    let design = design_with(vec![element(
        "gauge",
        json!({"sourceKind": "external", "sourceId": "missing"}),
        json!({}),
    )]);
    assert!(validate_design(design).is_err());
}

#[test]
fn screen_self_page_reference_is_checked() {
    let design = design_with(vec![element(
        "screen",
        json!({"designId": "", "pageId": "nope"}),
        json!({}),
    )]);
    let err = validate_design(design).unwrap_err();
    assert!(format!("{err:?}").contains("unknown page in this design"));
}

#[tokio::test]
async fn screen_embed_cross_design_save_checks() {
    let state = test_state();

    // A design embedding an unknown design is refused at save.
    let bad = design_with(vec![element(
        "screen",
        json!({"designId": "ghost", "pageId": ""}),
        json!({}),
    )]);
    let (status, _) = post(&state, "/api/layout?design=default", &bad).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);

    // Embedding this design's own home page is fine.
    let good = design_with(vec![element(
        "screen",
        json!({"designId": "", "pageId": ""}),
        json!({}),
    )]);
    let (status, _) = post(&state, "/api/layout?design=default", &good).await;
    assert_eq!(status, StatusCode::OK);
}

#[test]
fn gauge_renders_fill_against_track() {
    let design = design_with(vec![element(
        "gauge",
        json!({"sourceKind": "static", "value": 25, "min": 0, "max": 100, "showValue": false}),
        json!({"color": "#00ff00", "trackColor": "#101010", "fontSize": 14}),
    )]);
    let validated = validate_design(design).unwrap();
    let layout = effective_page_layout(&validated, "main").unwrap();
    let img = renderer::render(&layout, &json!({}), &HashMap::new());
    // Element frame is (20,20,200,100) — wait, element() uses h=100.
    // Fill covers the left quarter, track the rest (sampled mid-height).
    let fill_px = img.get_pixel(20 + 30, 20 + 50);
    let track_px = img.get_pixel(20 + 180, 20 + 50);
    assert_eq!(fill_px.0, [0, 255, 0], "fill pixel");
    assert_eq!(track_px.0, [16, 16, 16], "track pixel");
}

#[test]
fn qr_renders_dark_modules_on_light_background() {
    let design = design_with(vec![element(
        "qr",
        json!({"text": "https://example.com"}),
        json!({"color": "#000000", "bgColor": "#ffffff"}),
    )]);
    let validated = validate_design(design).unwrap();
    let layout = effective_page_layout(&validated, "main").unwrap();
    let img = renderer::render(&layout, &json!({}), &HashMap::new());
    // Frame is (20,20,200,100). The frame corner keeps the page
    // background; inside the frame the QR paints a light square with
    // hundreds of dark modules.
    assert_eq!(img.get_pixel(21, 21).0, [10, 10, 30], "frame corner");
    let mut dark = 0;
    let mut light = 0;
    for y in 20..120 {
        for x in 20..220 {
            match img.get_pixel(x, y).0 {
                [0, 0, 0] => dark += 1,
                [255, 255, 255] => light += 1,
                _ => {}
            }
        }
    }
    assert!(dark > 200 && light > 500, "dark {dark} light {light}");
}

#[test]
fn shape_circle_fills_center_not_corners() {
    let design = design_with(vec![element(
        "shape",
        json!({"shape": "circle"}),
        json!({"color": "#ff0000"}),
    )]);
    let validated = validate_design(design).unwrap();
    let layout = effective_page_layout(&validated, "main").unwrap();
    let img = renderer::render(&layout, &json!({}), &HashMap::new());
    // Frame (20,20,200,100): circle center (120,70) is red, the frame
    // corner keeps the design background (#0a0a1e).
    assert_eq!(img.get_pixel(120, 70).0, [255, 0, 0]);
    assert_eq!(img.get_pixel(21, 21).0, [10, 10, 30]);
}

#[tokio::test]
async fn embedded_screen_renders_target_page() {
    let state = test_state();

    // Design B: created via the designs API, then given one big
    // unmistakable color block.
    let (status, created) = post(
        &state,
        "/api/designs",
        &json!({"name": "Embed Target", "cloneFrom": "default"}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let target_id = created["id"].as_str().unwrap().to_string();
    let mut design_b = default_design();
    design_b["name"] = json!("Embed Target");
    design_b["pages"][0]["elements"] = json!([{
        "id": "block1", "type": "color-block",
        "frame": {"x": 0, "y": 0, "w": 240, "h": 320},
        "style": {"color": "#123456", "radius": 0},
        "props": {}
    }]);
    let (status, _) = post(&state, &format!("/api/layout?design={target_id}"), &design_b).await;
    assert_eq!(status, StatusCode::OK);

    // Design A embeds B's home page.
    let design_a = design_with(vec![element(
        "screen",
        json!({"designId": target_id, "pageId": ""}),
        json!({}),
    )]);
    let (status, _) = post(&state, "/api/layout?design=default", &design_a).await;
    assert_eq!(status, StatusCode::OK);

    // The embed map carries the pre-rendered target page.
    let layout = {
        let studio = state.studio.read().await;
        let design = &studio.designs["default"].layout;
        effective_page_layout(design, "main").unwrap()
    };
    let images = with_embed_images(&state, "default", &layout, HashMap::new()).await;
    let embed = images.get("embed:el_screen").expect("embed pre-rendered");
    assert_eq!(embed.get_pixel(120, 160).0, [0x12, 0x34, 0x56]);

    // And the composed preview shows it inside A's frame.
    let (status, png) = get(&state, "/api/designs/default/preview").await;
    assert_eq!(status, StatusCode::OK);
    let img = image::load_from_memory(&png).unwrap().to_rgb8();
    assert_eq!(img.get_pixel(120, 70).0, [0x12, 0x34, 0x56]);
}

#[tokio::test]
async fn self_embed_terminates_with_placeholder() {
    let state = test_state();
    // A page embedding itself must still render (depth-1 rule).
    let design = design_with(vec![element(
        "screen",
        json!({"designId": "", "pageId": ""}),
        json!({}),
    )]);
    let validated = validate_design(design).unwrap();
    let layout = effective_page_layout(&validated, "main").unwrap();
    let images = with_embed_images(&state, "default", &layout, HashMap::new()).await;
    // The seeded studio has a "default" design; the embed resolves to
    // it and renders something (not a hang, not a panic).
    let img = renderer::render(&layout, &json!({}), &images);
    assert_eq!((img.width(), img.height()), (240, 320));
}

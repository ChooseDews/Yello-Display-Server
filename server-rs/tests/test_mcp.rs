use hyper::{Body, Method, Request, StatusCode};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use yello_server::server::{handle_http_request, AppState};
use yello_server::*;

fn static_dir() -> PathBuf {
    if PathBuf::from("server-rs/static").exists() {
        PathBuf::from("server-rs/static")
    } else if PathBuf::from("../server-rs/static").exists() {
        PathBuf::from("../server-rs/static")
    } else {
        PathBuf::from("static")
    }
}

fn test_state() -> Arc<AppState> {
    Arc::new(AppState::new(
        PathBuf::from("/tmp/test_mcp_studio.json"),
        PathBuf::from("/tmp/test_mcp_secrets.json"),
        static_dir(),
    ))
}

async fn rpc(state: &Arc<AppState>, body: Value) -> (StatusCode, Value) {
    let req = Request::builder()
        .method(Method::POST)
        .uri("/mcp")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let resp = handle_http_request(req, state.clone()).await.unwrap();
    let status = resp.status();
    let bytes = hyper::body::to_bytes(resp.into_body()).await.unwrap();
    let val: Value = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    (status, val)
}

async fn call_tool(state: &Arc<AppState>, name: &str, arguments: Value) -> Value {
    let (status, resp) = rpc(
        state,
        json!({
            "jsonrpc": "2.0",
            "id": 99,
            "method": "tools/call",
            "params": {"name": name, "arguments": arguments}
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    resp["result"].clone()
}

#[tokio::test]
async fn test_mcp_handshake_and_tool_list() {
    let state = test_state();

    let (status, resp) = rpc(
        &state,
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-03-26"}}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(resp["result"]["protocolVersion"], "2025-03-26");
    assert_eq!(resp["result"]["serverInfo"]["name"], "yello-studio");

    let (_, resp) = rpc(&state, json!({"jsonrpc": "2.0", "id": 2, "method": "ping"})).await;
    assert_eq!(resp["result"], json!({}));

    let (_, resp) = rpc(&state, json!({"jsonrpc": "2.0", "id": 3, "method": "tools/list"})).await;
    let tools = resp["result"]["tools"].as_array().unwrap();
    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    for expected in [
        "yello_status",
        "yello_list_designs",
        "yello_get_design",
        "yello_save_design",
        "yello_preview_design",
        "yello_list_screens",
        "yello_assign_design",
    ] {
        assert!(names.contains(&expected), "missing tool {}", expected);
    }
    assert_eq!(names.len(), 7, "exactly seven tools: {:?}", names);
    assert!(
        !names.contains(&"yello_validate_design"),
        "yello_validate_design was removed (save validates)"
    );

    // Unknown method is a JSON-RPC error, not a tool error.
    let (_, resp) = rpc(&state, json!({"jsonrpc": "2.0", "id": 4, "method": "nope"})).await;
    assert_eq!(resp["error"]["code"], -32601);

    // Notifications get a bare 202 with no body.
    let (status, _) = rpc(&state, json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).await;
    assert_eq!(status, StatusCode::ACCEPTED);

    // GET is refused: the transport is POST-only.
    let req = Request::builder().method(Method::GET).uri("/mcp").body(Body::empty()).unwrap();
    let resp = handle_http_request(req, state.clone()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn test_mcp_design_round_trip() {
    let state = test_state();

    // The seeded default design is listed and fetchable.
    let result = call_tool(&state, "yello_list_designs", json!({})).await;
    assert_eq!(result["isError"], false);
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("\"default\""), "list text: {}", text);

    let result = call_tool(&state, "yello_get_design", json!({"designId": "default"})).await;
    assert_eq!(result["isError"], false);

    // Save a new design.
    let mut layout = default_design();
    layout["name"] = json!("Agent made");
    let result = call_tool(
        &state,
        "yello_save_design",
        json!({"designId": "agent-made", "layout": layout}),
    )
    .await;
    assert_eq!(result["isError"], false, "save result: {}", result);
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("\"revision\": 1"), "save text: {}", text);

    // It round-trips as a v2 document with pages.
    let result = call_tool(&state, "yello_get_design", json!({"designId": "agent-made"})).await;
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("\"pages\""), "get text: {}", text);
    assert!(text.contains("Agent made"), "get text: {}", text);

    // Saving again bumps the revision.
    let mut layout = default_design();
    layout["name"] = json!("Agent made v2");
    let result = call_tool(
        &state,
        "yello_save_design",
        json!({"designId": "agent-made", "layout": layout}),
    )
    .await;
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("\"revision\": 2"), "save2 text: {}", text);
}

#[tokio::test]
async fn test_mcp_preview_returns_png() {
    let state = test_state();

    let result = call_tool(&state, "yello_preview_design", json!({"designId": "default"})).await;
    assert_eq!(result["isError"], false, "preview result: {}", result);
    let image = &result["content"][0];
    assert_eq!(image["type"], "image");
    assert_eq!(image["mimeType"], "image/png");
    let data = image["data"].as_str().unwrap();
    // Base64 of the PNG magic bytes always starts with iVBOR.
    assert!(data.starts_with("iVBOR"), "preview data prefix: {}", &data[..16.min(data.len())]);

    // Inline (unsaved) layout preview works too.
    let result = call_tool(
        &state,
        "yello_preview_design",
        json!({"layout": default_design()}),
    )
    .await;
    assert_eq!(result["isError"], false);
    assert_eq!(result["content"][0]["type"], "image");

    // A missing design is a tool error.
    let result = call_tool(&state, "yello_preview_design", json!({"designId": "ghost"})).await;
    assert_eq!(result["isError"], true);
}

#[tokio::test]
async fn test_mcp_validation_errors_are_tool_errors() {
    let state = test_state();

    // A page: link to a page that does not exist must be rejected.
    let mut bad = default_design();
    bad["pages"][0]["elements"] = json!([{
        "id": "b1", "type": "design-link",
        "frame": {"x": 10, "y": 10, "w": 100, "h": 40},
        "style": {},
        "props": {"label": "Go", "targetDesignId": "page:missing"}
    }]);
    // Validation happens inside yello_save_design (there is no separate
    // validate tool): the same bad layout is rejected with the same text.
    let bad2 = bad.clone();
    let result = call_tool(
        &state,
        "yello_save_design",
        json!({"designId": "broken", "layout": bad2}),
    )
    .await;
    assert_eq!(result["isError"], true);
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("unknown page"), "save validation text: {}", text);
    // And it was not saved.
    let result = call_tool(&state, "yello_get_design", json!({"designId": "broken"})).await;
    assert_eq!(result["isError"], true);

    // Unknown tools and missing arguments are clean tool errors.
    let result = call_tool(&state, "yello_nope", json!({})).await;
    assert_eq!(result["isError"], true);
    let result = call_tool(&state, "yello_get_design", json!({})).await;
    assert_eq!(result["isError"], true);
}

//! MCP (Model Context Protocol) endpoint: POST /mcp
//!
//! Stateless Streamable HTTP transport carrying JSON-RPC 2.0, the same
//! shape as the BaseAuth/ConnectBase MCP server: initialize / ping /
//! tools/list / tools/call, notifications answer 202, GET answers 405
//! (wired in the router). Tools let an agent manage, update, and preview
//! screen designs without driving the Studio UI. Design writes go through
//! the exact same validation as POST /api/layout (validate_design +
//! cross-design link check) and push to assigned screens the same way.
//! Like the rest of the Yello API, the endpoint is unauthenticated and
//! intended for the LAN / tailnet.

use super::*;
use crate::layout_model::LayoutError;
use base64::Engine as _;
use std::collections::HashSet;
use std::time::Instant;

const MCP_PROTOCOL_VERSION: &str = "2025-03-26";

fn rpc_ok(id: &Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn rpc_err(id: &Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn tool_text(text: String, is_error: bool) -> Value {
    json!({"content": [{"type": "text", "text": text}], "isError": is_error})
}

fn tool_json(value: &Value) -> Value {
    tool_text(
        serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_string()),
        false,
    )
}

fn tool_error(message: String) -> Value {
    tool_text(message, true)
}

fn validation_error_text(context: &str, errors: &[LayoutError]) -> String {
    let lines: Vec<String> = errors
        .iter()
        .map(|e| format!("- {}: {}", e.path, e.message))
        .collect();
    format!("{}:\n{}", context, lines.join("\n"))
}

pub fn tool_definitions() -> Value {
    json!([
        {
            "name": "yello_status",
            "description": "Server status: design/screen counts and how many screens are connected.",
            "inputSchema": {"type": "object", "properties": {}}
        },
        {
            "name": "yello_list_designs",
            "description": "List all saved designs with id, name, revision, orientation, page count, and home page id.",
            "inputSchema": {"type": "object", "properties": {}}
        },
        {
            "name": "yello_get_design",
            "description": "Fetch one design's full schema-v2 JSON document (pages, shared elements, data sources, actions).",
            "inputSchema": {
                "type": "object",
                "properties": {"designId": {"type": "string"}},
                "required": ["designId"]
            }
        },
        {
            "name": "yello_save_design",
            "description": "Create or replace a design. The layout is validated exactly like a Studio save (schema-v2 pages design, or a legacy v1 layout which is migrated); bad link targets are rejected. The design is saved to disk and pushed to screens assigned to it.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "designId": {"type": "string"},
                    "layout": {"type": "object", "description": "The full design document."}
                },
                "required": ["designId", "layout"]
            }
        },
        {
            "name": "yello_validate_design",
            "description": "Validate a design document without saving it. Returns the normalized schema-v2 document when valid.",
            "inputSchema": {
                "type": "object",
                "properties": {"layout": {"type": "object"}},
                "required": ["layout"]
            }
        },
        {
            "name": "yello_preview_design",
            "description": "Render a design page to a PNG image, exactly as the device would show it. Give designId for a saved design, or layout for an unsaved document; pageId defaults to the design's home page.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "designId": {"type": "string"},
                    "pageId": {"type": "string"},
                    "layout": {"type": "object"}
                }
            }
        },
        {
            "name": "yello_list_screens",
            "description": "List known screens with their assigned design and connection state.",
            "inputSchema": {"type": "object", "properties": {}}
        },
        {
            "name": "yello_assign_design",
            "description": "Assign a design to one or more screens and push it to connected devices.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "designId": {"type": "string"},
                    "screenIds": {"type": "array", "items": {"type": "string"}}
                },
                "required": ["designId", "screenIds"]
            }
        }
    ])
}

async fn tool_status(state: &Arc<AppState>) -> Value {
    let studio = state.studio.read().await;
    let devices = state.devices.read().await;
    tool_json(&json!({
        "designsCount": studio.designs.len(),
        "screensCount": studio.screens.len(),
        "knownScreens": studio.screens.len(),
        "connectedScreens": devices.len(),
        "deviceConnected": !devices.is_empty(),
    }))
}

async fn tool_list_designs(state: &Arc<AppState>) -> Value {
    let studio = state.studio.read().await;
    let mut designs: Vec<Value> = studio
        .designs
        .values()
        .map(|design| {
            json!({
                "id": design.id,
                "name": design.name,
                "revision": design.revision,
                "orientation": design.layout.get("orientation").and_then(|v| v.as_str()).unwrap_or("portrait"),
                "pageCount": design.layout.get("pages").and_then(|p| p.as_array()).map(|a| a.len()).unwrap_or(1),
                "homePageId": design_home_page_id(&design.layout),
            })
        })
        .collect();
    designs.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
    tool_json(&json!({"designs": designs}))
}

async fn tool_get_design(state: &Arc<AppState>, args: &Value) -> Value {
    let design_id = match args.get("designId").and_then(|v| v.as_str()) {
        Some(id) => id,
        None => return tool_error("missing required argument: designId".to_string()),
    };
    let studio = state.studio.read().await;
    match studio.designs.get(design_id) {
        Some(design) => tool_json(&json!({
            "designId": design.id,
            "revision": design.revision,
            "layout": design.layout,
        })),
        None => tool_error(format!("design not found: {}", design_id)),
    }
}

/// Shared by yello_save_design: validates, checks cross-design links,
/// writes the design, persists the studio file, and pushes to screens.
/// Returns (revision, applied screen count).
async fn save_design(
    state: &Arc<AppState>,
    design_id: &str,
    raw_layout: Value,
) -> Result<(u64, usize), String> {
    let layout = match validate_design(raw_layout) {
        Ok(layout) => layout,
        Err(e) => return Err(validation_error_text("invalid layout", &e.errors)),
    };
    let link_errors = {
        let studio = state.studio.read().await;
        cross_design_link_errors(&studio, design_id, &layout)
    };
    if !link_errors.is_empty() {
        return Err(validation_error_text("invalid link targets", &link_errors));
    }
    let revision = {
        let mut studio = state.studio.write().await;
        let name = layout
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or(design_id)
            .to_string();
        match studio.designs.get_mut(design_id) {
            Some(design) => {
                design.layout = layout;
                design.name = name;
                design.revision += 1;
                design.revision
            }
            None => {
                studio.designs.insert(
                    design_id.to_string(),
                    Design {
                        id: design_id.to_string(),
                        name,
                        revision: 1,
                        layout,
                    },
                );
                1
            }
        }
    };
    state.save_studio_file().await;
    push_design(state, design_id, true).await;
    let applied = {
        let studio = state.studio.read().await;
        let devices = state.devices.read().await;
        devices
            .values()
            .filter(|session| design_id_for_session(&studio, session) == design_id)
            .count()
    };
    Ok((revision, applied))
}

async fn tool_save_design(state: &Arc<AppState>, args: &Value) -> Value {
    let design_id = match args.get("designId").and_then(|v| v.as_str()) {
        Some(id) if !id.is_empty() => id.to_string(),
        _ => return tool_error("missing required argument: designId".to_string()),
    };
    let layout = match args.get("layout") {
        Some(layout) if layout.is_object() => layout.clone(),
        _ => return tool_error("missing required argument: layout (object)".to_string()),
    };
    match save_design(state, &design_id, layout).await {
        Ok((revision, applied)) => tool_json(&json!({
            "status": "ok",
            "designId": design_id,
            "revision": revision,
            "appliedScreens": applied,
        })),
        Err(message) => tool_error(message),
    }
}

fn tool_validate_design(args: &Value) -> Value {
    match args.get("layout") {
        Some(layout) if layout.is_object() => match validate_design(layout.clone()) {
            Ok(normalized) => tool_json(&json!({"valid": true, "layout": normalized})),
            Err(e) => tool_error(validation_error_text("invalid layout", &e.errors)),
        },
        _ => tool_error("missing required argument: layout (object)".to_string()),
    }
}

async fn tool_preview_design(state: &Arc<AppState>, args: &Value) -> Value {
    // Resolve the design document: an inline layout wins, else the saved
    // design named by designId.
    let (design_id, design) = if let Some(layout) = args.get("layout") {
        if !layout.is_object() {
            return tool_error("layout must be an object".to_string());
        }
        match validate_design(layout.clone()) {
            Ok(design) => (
                args.get("designId")
                    .and_then(|v| v.as_str())
                    .unwrap_or(DEFAULT_DESIGN_ID)
                    .to_string(),
                design,
            ),
            Err(e) => return tool_error(validation_error_text("invalid layout", &e.errors)),
        }
    } else if let Some(id) = args.get("designId").and_then(|v| v.as_str()) {
        let studio = state.studio.read().await;
        match studio.designs.get(id) {
            Some(design) => (id.to_string(), design.layout.clone()),
            None => return tool_error(format!("design not found: {}", id)),
        }
    } else {
        return tool_error("give designId or layout".to_string());
    };
    let page_id = args
        .get("pageId")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| design_home_page_id(&design));
    let layout = match effective_page_layout(&design, &page_id) {
        Some(layout) => layout,
        None => return tool_error(format!("page not found: {}", page_id)),
    };
    let ext_vals = state.external_values.read().await;
    let ha_vals = state.ha_values.read().await;
    let pressed: HashSet<String> = HashSet::new();
    let dots: Vec<(i32, i32, Instant)> = Vec::new();
    let ctx = build_render_context(&ext_vals, &design_id, &ha_vals, &pressed, &dots);
    let images = state.images.read().await.clone();
    let img = renderer::render(&layout, &ctx, &images);
    let (width, height) = (img.width(), img.height());
    let mut buf = Vec::new();
    let mut cursor = std::io::Cursor::new(&mut buf);
    if img.write_to(&mut cursor, image::ImageFormat::Png).is_err() {
        return tool_error("failed to encode preview PNG".to_string());
    }
    let b64 = base64::engine::general_purpose::STANDARD.encode(&buf);
    json!({
        "content": [
            {"type": "image", "data": b64, "mimeType": "image/png"},
            {"type": "text", "text": format!("Preview of design '{}', page '{}' ({}x{}).", design_id, page_id, width, height)}
        ],
        "isError": false
    })
}

async fn tool_list_screens(state: &Arc<AppState>) -> Value {
    let studio = state.studio.read().await;
    let devices = state.devices.read().await;
    let mut screens: Vec<Value> = studio
        .screens
        .values()
        .map(|screen| {
            json!({
                "id": screen.id,
                "name": screen.name,
                "designId": screen.design_id,
                "lastSeen": screen.last_seen,
                "connected": devices.contains_key(&screen.id),
            })
        })
        .collect();
    screens.sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
    tool_json(&json!({"screens": screens}))
}

async fn tool_assign_design(state: &Arc<AppState>, args: &Value) -> Value {
    let design_id = match args.get("designId").and_then(|v| v.as_str()) {
        Some(id) => id.to_string(),
        None => return tool_error("missing required argument: designId".to_string()),
    };
    let screen_ids: Vec<String> = match args.get("screenIds").and_then(|v| v.as_array()) {
        Some(arr) => arr
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect(),
        None => return tool_error("missing required argument: screenIds (array)".to_string()),
    };
    let assigned = {
        let mut studio = state.studio.write().await;
        if !studio.designs.contains_key(&design_id) {
            return tool_error(format!("design not found: {}", design_id));
        }
        let mut count = 0usize;
        for sid in &screen_ids {
            if let Some(screen) = studio.screens.get_mut(sid) {
                screen.design_id = design_id.clone();
                count += 1;
            }
        }
        count
    };
    if assigned == 0 {
        return tool_error("no matching screens".to_string());
    }
    state.save_studio_file().await;
    push_design(state, &design_id, false).await;
    tool_json(&json!({"status": "ok", "designId": design_id, "assigned": assigned}))
}

async fn call_tool(state: &Arc<AppState>, params: &Value) -> Value {
    let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
    let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
    match name {
        "yello_status" => tool_status(state).await,
        "yello_list_designs" => tool_list_designs(state).await,
        "yello_get_design" => tool_get_design(state, &args).await,
        "yello_save_design" => tool_save_design(state, &args).await,
        "yello_validate_design" => tool_validate_design(&args),
        "yello_preview_design" => tool_preview_design(state, &args).await,
        "yello_list_screens" => tool_list_screens(state).await,
        "yello_assign_design" => tool_assign_design(state, &args).await,
        _ => tool_error(format!("unknown tool: {}", name)),
    }
}

async fn handle_rpc(state: &Arc<AppState>, request: &Value) -> Option<Value> {
    let id = request.get("id").cloned();
    let method = request.get("method").and_then(|v| v.as_str()).unwrap_or("");
    // Notifications carry no id and never get a response.
    if id.is_none() {
        return None;
    }
    let id = id.unwrap();
    let params = request.get("params").cloned().unwrap_or(Value::Null);
    let result = match method {
        "initialize" => {
            let client_version = params
                .get("protocolVersion")
                .and_then(|v| v.as_str())
                .unwrap_or(MCP_PROTOCOL_VERSION);
            json!({
                "protocolVersion": client_version,
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "yello-studio", "version": env!("CARGO_PKG_VERSION")},
            })
        }
        "ping" => json!({}),
        "tools/list" => json!({"tools": tool_definitions()}),
        "tools/call" => return Some(rpc_ok(&id, call_tool(state, &params).await)),
        _ => return Some(rpc_err(&id, -32601, "method not found")),
    };
    Some(rpc_ok(&id, result))
}

fn mcp_json_response(status: StatusCode, value: &Value) -> Response<Body> {
    Response::builder()
        .status(status)
        .header("Content-Type", "application/json")
        .header("Access-Control-Allow-Origin", "*")
        .body(Body::from(value.to_string()))
        .unwrap()
}

pub async fn handle_mcp_request(state: Arc<AppState>, req: Request<Body>) -> Response<Body> {
    let body_bytes = hyper::body::to_bytes(req.into_body()).await.unwrap_or_default();
    let parsed: Value = match serde_json::from_slice(&body_bytes) {
        Ok(value) => value,
        Err(_) => {
            return mcp_json_response(
                StatusCode::OK,
                &rpc_err(&Value::Null, -32700, "parse error"),
            );
        }
    };
    // Batch: answer every non-notification, or 202 when there is nothing
    // to answer.
    if let Some(batch) = parsed.as_array() {
        let mut responses = Vec::new();
        for item in batch {
            if let Some(resp) = handle_rpc(&state, item).await {
                responses.push(resp);
            }
        }
        if responses.is_empty() {
            return Response::builder().status(StatusCode::ACCEPTED).body(Body::empty()).unwrap();
        }
        return mcp_json_response(StatusCode::OK, &Value::Array(responses));
    }
    match handle_rpc(&state, &parsed).await {
        Some(resp) => mcp_json_response(StatusCode::OK, &resp),
        None => Response::builder().status(StatusCode::ACCEPTED).body(Body::empty()).unwrap(),
    }
}

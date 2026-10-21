use yello_server::render;
use std::collections::HashMap;

#[test]
fn render_current_layout_debug() {
    let raw = std::fs::read_to_string("../studio.json").expect("studio.json");
    let doc: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let layout = doc
        .get("designs")
        .and_then(|d| d.get("default"))
        .and_then(|d| d.get("layout"))
        .cloned()
        .expect("default design layout");
    println!("layout bg: {}", layout.get("background").unwrap());
    let ctx = serde_json::json!({"pressed": [], "homeAssistant": {}, "external": {}});
    let img = render(&layout, &ctx, &HashMap::new());
    img.save("/tmp/opencode/current_layout.png").unwrap();

    // Also render a text-heavy probe: descenders at several sizes in tight frames
    let probe = serde_json::json!({
        "background": "#000000",
        "elements": [
            {"type": "text", "id": "a", "frame": {"x": 0, "y": 0, "w": 240, "h": 40},
             "style": {"fontSize": 40, "color": "#ffffff", "verticalAlign": "middle", "align": "center"},
             "props": {"text": "Ygy"}},
            {"type": "text", "id": "b", "frame": {"x": 0, "y": 44, "w": 240, "h": 32},
             "style": {"fontSize": 32, "color": "#ffffff", "verticalAlign": "middle", "align": "center"},
             "props": {"text": "Ygy"}},
            {"type": "text", "id": "c", "frame": {"x": 0, "y": 80, "w": 240, "h": 20},
             "style": {"fontSize": 20, "color": "#ffffff", "verticalAlign": "middle", "align": "center"},
             "props": {"text": "Ygy"}}
        ]
    });
    let img2 = render(&probe, &ctx, &HashMap::new());
    img2.save("/tmp/opencode/probe.png").unwrap();
}

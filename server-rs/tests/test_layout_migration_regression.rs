//! Regression tests for migrate_layout: a modern (schema-v1) layout posted
//! without schemaVersion must not be rebuilt from legacy defaults.

use yello_server::layout_model::validate_layout;
use serde_json::json;

#[test]
fn modern_layout_without_schema_version_keeps_props() {
    let raw = json!({
        "background": "#101040",
        "elements": [
            {"type": "text", "id": "t1", "frame": {"x": 10, "y": 10, "w": 220, "h": 60},
             "style": {"fontSize": 28, "color": "#ff0000", "align": "center", "verticalAlign": "middle"},
             "props": {"text": "Hello"}},
            {"type": "image", "id": "img1", "frame": {"x": 20, "y": 80, "w": 200, "h": 160},
             "style": {}, "props": {"src": "https://example.com/photo.jpg", "fit": "cover"}}
        ]
    });
    let out = validate_layout(raw).expect("valid layout");
    assert_eq!(out["elements"][0]["props"]["text"], "Hello", "text prop reset to default");
    assert_eq!(out["elements"][0]["frame"]["x"], 10, "frame reset to default");
    assert_eq!(out["elements"][0]["style"]["fontSize"], 28, "fontSize reset to default");
    assert_eq!(out["elements"][1]["props"]["src"], "https://example.com/photo.jpg", "image src reset to default");
    assert_eq!(out["elements"][1]["props"]["fit"], "cover", "image fit reset to default");
}

#[test]
fn legacy_flat_elements_are_still_migrated() {
    let raw = json!({
        "elements": [
            {"type": "time", "id": "clk", "x": 120, "y": 80, "size": 40},
            {"type": "text", "id": "t1", "x": 120, "y": 160, "size": 16, "text": "old text", "color": "#ff0000"}
        ]
    });
    let out = validate_layout(raw).expect("valid layout");
    assert_eq!(out["elements"][0]["type"], "clock", "legacy type rename lost");
    assert_eq!(out["elements"][1]["props"]["text"], "old text", "legacy text prop lost");
    assert_eq!(out["elements"][1]["style"]["color"], "#ff0000", "legacy color lost");
}

use serde_json::json;
use std::collections::HashMap;
use yello_server::*;

#[test]
fn test_default_layout_is_valid_and_normalized() {
    let layout = validate_layout(default_layout()).unwrap();
    assert_eq!(layout["schemaVersion"], 1);
    let elements = layout["elements"].as_array().unwrap();
    let types: Vec<&str> = elements.iter().map(|e| e["type"].as_str().unwrap()).collect();
    assert_eq!(types, vec!["clock", "clock", "color-block", "text"]);
}

#[test]
fn test_legacy_layout_migration() {
    let legacy = json!({
        "background": "#010203",
        "elements": [
            {"id": "weather", "type": "external", "x": 120, "y": 40, "url": "https://example.com/value", "interval": 30},
            {"id": "go", "type": "button", "x": 10, "y": 20, "w": 90, "h": 40, "label": "Go", "action": "https://example.com/go"}
        ]
    });
    let layout = validate_layout(legacy).unwrap();
    let elements = layout["elements"].as_array().unwrap();
    assert_eq!(elements[0]["type"], "external-text");
    assert_eq!(elements[0]["frame"]["x"], 0);
    assert_eq!(elements[0]["props"]["sourceId"], layout["dataSources"][0]["id"]);
    assert_eq!(elements[1]["props"]["actionId"], layout["actions"][0]["id"]);
}

#[test]
fn test_color_block_rendering() {
    let mut layout = default_layout();
    let first = json!({
        "id": "first",
        "type": "color-block",
        "name": "First",
        "visible": true,
        "locked": false,
        "frame": {"x": 10, "y": 10, "w": 30, "h": 30},
        "style": {"color": "#ff0000", "radius": 0},
        "props": {}
    });
    let second = json!({
        "id": "second",
        "type": "color-block",
        "name": "Second",
        "visible": true,
        "locked": false,
        "frame": {"x": 20, "y": 20, "w": 30, "h": 30},
        "style": {"color": "#0000ff", "radius": 0},
        "props": {}
    });
    layout["elements"] = json!([first, second]);
    let norm = validate_layout(layout).unwrap();
    let img = render(&norm, &json!({}), &HashMap::new());
    assert_eq!(img.get_pixel(12, 12).0, [255, 0, 0]);
    assert_eq!(img.get_pixel(25, 25).0, [0, 0, 255]);
}

#[test]
fn test_hit_test_buttons() {
    let mut layout = default_layout();
    let lower = json!({
        "id": "lower", "type": "button", "name": "Lower", "visible": true, "locked": false,
        "frame": {"x": 10, "y": 10, "w": 100, "h": 50}, "style": {"color": "#2255aa"}, "props": {"label": "Lower"}
    });
    let upper = json!({
        "id": "upper", "type": "button", "name": "Upper", "visible": true, "locked": false,
        "frame": {"x": 10, "y": 10, "w": 100, "h": 50}, "style": {"color": "#2255aa"}, "props": {"label": "Upper"}
    });
    layout["elements"] = json!([lower, upper]);
    let norm = validate_layout(layout).unwrap();
    assert_eq!(hit_test_buttons(&norm, 20, 20).unwrap()["id"], "upper");
}

#[test]
fn test_rgb565_be_conversion() {
    let rgb888 = vec![255, 0, 0, 0, 255, 0, 0, 0, 255];
    let rgb565 = rgb888_to_rgb565_be(&rgb888, 3, 1);
    assert_eq!(rgb565.len(), 6);
    // Red: 0xF800 -> big-endian: 0xF8, 0x00
    assert_eq!(rgb565[0], 0xF8);
    assert_eq!(rgb565[1], 0x00);
    // Green: 0x07E0 -> big-endian: 0x07, 0xE0
    assert_eq!(rgb565[2], 0x07);
    assert_eq!(rgb565[3], 0xE0);
    // Blue: 0x001F -> big-endian: 0x00, 0x1F
    assert_eq!(rgb565[4], 0x00);
    assert_eq!(rgb565[5], 0x1F);
}

#[test]
fn test_fit_image_modes() {
    // 4x2 image: left half white, right half black
    let mut src = image::RgbImage::new(4, 2);
    for y in 0..2 {
        for x in 0..4 {
            src.put_pixel(x, y, if x < 2 { image::Rgb([255, 255, 255]) } else { image::Rgb([0, 0, 0]) });
        }
    }
    // contain in 8x8: 8x4 centered vertically on black
    let contained = fit_image(&src, 8, 8, "contain");
    assert_eq!(contained.dimensions(), (8, 8));
    assert_eq!(contained.get_pixel(1, 2).0, [255, 255, 255]);
    assert_eq!(contained.get_pixel(6, 2).0, [0, 0, 0]);
    assert_eq!(contained.get_pixel(1, 0).0, [0, 0, 0], "letterbox bar expected");
    // stretch to 8x8 distorts aspect: white fills left half everywhere
    let stretched = fit_image(&src, 8, 8, "stretch");
    assert_eq!(stretched.dimensions(), (8, 8));
    assert_eq!(stretched.get_pixel(1, 7).0, [255, 255, 255]);
    // cover crops to target aspect (source is wider than 1:1 target -> crop width)
    let covered = fit_image(&src, 2, 2, "cover");
    assert_eq!(covered.dimensions(), (2, 2));
}

#[test]
fn test_wanted_image_sources() {
    let layout = json!({
        "elements": [
            {"type": "image", "props": {"src": "http://a/b.png"}},
            {"type": "image", "props": {}},
            {"type": "text", "props": {"src": "http://c/d.png"}},
            {"type": "image", "props": {"src": "http://a/b.png"}}
        ]
    });
    let sources = wanted_image_sources(&layout);
    assert_eq!(sources.len(), 1);
    assert!(sources.contains("http://a/b.png"));
}

#[test]
fn test_small_text_is_antialiased_not_binary() {
    // Regression: glyph coverage was composited as binary (any alpha > 0 ->
    // full text color), bloating small text into merged blobs.
    const FONT_PATH: &str = "/usr/share/fonts/liberation-sans-fonts/LiberationSans-Bold.ttf";
    if !std::path::Path::new(FONT_PATH).exists() {
        // Bitmap fallback draws opaque pixels; the AA assertion below does not apply.
        return;
    }
    let layout = json!({
        "schemaVersion": 1, "width": 240, "height": 320, "orientation": "portrait",
        "background": "#0a0a1e",
        "elements": [
            {"id": "t", "type": "text", "frame": {"x": 0, "y": 0, "w": 120, "h": 30},
             "style": {"fontSize": 12, "color": "#ffffff", "align": "center", "verticalAlign": "middle"},
             "props": {"text": "SkyWest Airlines", "wrap": true}}
        ]
    });
    let norm = validate_layout(layout).unwrap();
    let img = render(&norm, &json!({}), &HashMap::new());
    let mut solid = 0;
    let mut partial = 0;
    for y in 0..30 {
        for x in 0..120 {
            let l = img.get_pixel(x, y).0;
            if l[0] >= 250 { solid += 1; }
            else if l[0] > 30 { partial += 1; }
        }
    }
    assert!(solid > 20, "expected solid glyph pixels, got {solid}");
    assert!(partial > 20, "expected antialiased edge pixels (blended coverage), got {partial}");
}

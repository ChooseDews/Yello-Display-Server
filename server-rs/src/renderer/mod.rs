//! Canonical renderer for normalized schema-v1 Yello layouts.

use ab_glyph::{point, FontRef, Glyph, PxScale, PxScaleFont, ScaleFont};
use image::{imageops::FilterType, Rgb, RgbImage, Rgba, RgbaImage};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

use crate::layout_model::{find_by_id, layout_dimensions};
use crate::script_runtime::{execute_script, ScriptOutput};

pub const DOT_RADIUS: i32 = 10;

const FONT_PATH: &str = "/usr/share/fonts/liberation-sans-fonts/LiberationSans-Bold.ttf";

pub fn parse_color(val: &Value, default: (u8, u8, u8)) -> (u8, u8, u8) {
    if let Some(s) = val.as_str() {
        let s = s.trim_start_matches('#');
        if s.len() == 6 {
            if let (Ok(r), Ok(g), Ok(b)) = (
                u8::from_str_radix(&s[0..2], 16),
                u8::from_str_radix(&s[2..4], 16),
                u8::from_str_radix(&s[4..6], 16),
            ) {
                return (r, g, b);
            }
        }
    }
    if let Some(arr) = val.as_array() {
        if arr.len() == 3 {
            let r = arr[0].as_u64().unwrap_or(default.0 as u64) as u8;
            let g = arr[1].as_u64().unwrap_or(default.1 as u64) as u8;
            let b = arr[2].as_u64().unwrap_or(default.2 as u64) as u8;
            return (r, g, b);
        }
    }
    default
}

pub fn brightness(rgb: (u8, u8, u8)) -> f64 {
    0.299 * (rgb.0 as f64) + 0.587 * (rgb.1 as f64) + 0.114 * (rgb.2 as f64)
}

// 8x8 Basic ASCII Font Bitmap
pub const FONT_8X8: &[u8; 96 * 8] = include_bytes!("font8x8.bin");

pub fn get_glyph(c: char) -> &'static [u8] {
    let idx = if c >= ' ' && c <= '~' {
        (c as usize) - (' ' as usize)
    } else {
        ('?' as usize) - (' ' as usize)
    };
    &FONT_8X8[idx * 8..(idx + 1) * 8]
}

pub fn draw_bitmap_text(
    img: &mut RgbaImage,
    start_x: i32,
    start_y: i32,
    text: &str,
    color: Rgba<u8>,
    scale: u32,
) {
    let scale = scale.max(1);
    let mut cursor_x = start_x;
    for c in text.chars() {
        if c == '\n' {
            continue;
        }
        let glyph = get_glyph(c);
        for (row_idx, &row_bits) in glyph.iter().enumerate() {
            for col_idx in 0..8 {
                if (row_bits & (1 << col_idx)) != 0 {
                    for sy in 0..scale {
                        for sx in 0..scale {
                            let px = cursor_x + (col_idx as i32) * (scale as i32) + sx as i32;
                            let py = start_y + (row_idx as i32) * (scale as i32) + sy as i32;
                            if px >= 0 && px < img.width() as i32 && py >= 0 && py < img.height() as i32 {
                                img.put_pixel(px as u32, py as u32, color);
                            }
                        }
                    }
                }
            }
        }
        cursor_x += (8 * scale) as i32;
    }
}

pub fn wrap_text(text: &str, max_width_px: u32, scale: u32) -> Vec<String> {
    let char_w = 8 * scale.max(1);
    let max_chars = (max_width_px / char_w).max(1) as usize;
    let mut lines = Vec::new();

    for paragraph in text.lines() {
        let words = paragraph.split(' ').collect::<Vec<_>>();
        let mut current_line = String::new();
        for word in words {
            let candidate = if current_line.is_empty() {
                word.to_string()
            } else {
                format!("{current_line} {word}")
            };
            if !current_line.is_empty() && candidate.len() > max_chars {
                lines.push(current_line);
                current_line = word.to_string();
            } else {
                current_line = candidate;
            }
        }
        lines.push(current_line);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

pub fn draw_text_box(
    img: &mut RgbImage,
    frame_x: i32,
    frame_y: i32,
    width: u32,
    height: u32,
    text: &str,
    style: &Value,
    wrap: bool,
) {
    let font_size = style.get("fontSize").and_then(|v| v.as_u64()).unwrap_or(18) as u32;
    let align = style.get("align").and_then(|v| v.as_str()).unwrap_or("center");
    let vertical = style.get("verticalAlign").and_then(|v| v.as_str()).unwrap_or("middle");
    let color_rgb = parse_color(style.get("color").unwrap_or(&Value::Null), (255, 255, 255));

    if let Some(layer) = draw_truetype_text(width, height, text, font_size, align, vertical, color_rgb, wrap) {
        for dy in 0..height {
            let py = frame_y + dy as i32;
            if py < 0 || py >= img.height() as i32 { continue; }
            for dx in 0..width {
                let px = frame_x + dx as i32;
                if px < 0 || px >= img.width() as i32 { continue; }
                let p = layer.get_pixel(dx, dy);
                let alpha = p[3] as f32 / 255.0;
                if alpha <= 0.0 { continue; }
                let bg = img.get_pixel(px as u32, py as u32);
                let blend = |t: u8, b: u8| -> u8 {
                    (t as f32 * alpha + b as f32 * (1.0 - alpha)).round().clamp(0.0, 255.0) as u8
                };
                img.put_pixel(px as u32, py as u32, Rgb([blend(p[0], bg[0]), blend(p[1], bg[1]), blend(p[2], bg[2])]));
            }
        }
        return;
    }

    let scale = (font_size as f32 / 8.0).max(1.0).round() as u32;
    let color_rgba = Rgba([color_rgb.0, color_rgb.1, color_rgb.2, 255]);
    let char_h = 8 * scale;
    let char_w = 8 * scale;
    let lines = if wrap {
        wrap_text(text, width.saturating_sub(4), scale)
    } else {
        text.lines().map(|s| s.to_string()).collect()
    };
    let total_text_h = lines.len() as u32 * (char_h + 2);
    let top_y = match vertical {
        "top" => 2,
        "bottom" => (height as i32) - (total_text_h as i32) - 2,
        _ => ((height as i32) - (total_text_h as i32)) / 2,
    };
    let mut temp_layer = RgbaImage::new(width, height);
    for (i, line) in lines.iter().enumerate() {
        let line_w = line.len() as u32 * char_w;
        let line_x = match align {
            "left" => 2,
            "right" => (width as i32) - (line_w as i32) - 2,
            _ => ((width as i32) - (line_w as i32)) / 2,
        };
        let line_y = top_y + (i as i32) * ((char_h + 2) as i32);
        draw_bitmap_text(&mut temp_layer, line_x, line_y, line, color_rgba, scale);
    }

    for dy in 0..height {
        let py = frame_y + dy as i32;
        if py < 0 || py >= img.height() as i32 { continue; }
        for dx in 0..width {
            let px = frame_x + dx as i32;
            if px < 0 || px >= img.width() as i32 { continue; }
            let p = temp_layer.get_pixel(dx, dy);
            if p[3] > 0 {
                img.put_pixel(px as u32, py as u32, Rgb([p[0], p[1], p[2]]));
            }
        }
    }
}

fn draw_truetype_text(
    width: u32,
    height: u32,
    text: &str,
    font_size: u32,
    align: &str,
    vertical: &str,
    color: (u8, u8, u8),
    wrap: bool,
) -> Option<RgbaImage> {
    let font_data = std::fs::read(FONT_PATH).ok()?;
    let font = FontRef::try_from_slice(&font_data).ok()?;
    let font = PxScaleFont {
        font,
        scale: PxScale::from(font_size as f32),
    };
    let max_text_width = (width.saturating_sub(4)) as f32;
    let lines: Vec<String> = if wrap {
        wrapped_lines(&font, text, max_text_width)
    } else {
        text.lines().map(|s| s.to_string()).collect()
    };
    let line_spacing = 2.0;
    let ascent = font.ascent();
    let descent = font.descent();
    let line_height = ascent - descent + line_spacing;
    let total_text_height = lines.len() as f32 * line_height - line_spacing;
    let mut top_y = match vertical {
        "top" => 1.0,
        "bottom" => height as f32 - total_text_height - 1.0,
        _ => ((height as f32) - total_text_height) / 2.0,
    };
    let color_rgba = Rgba([color.0, color.1, color.2, 255]);
    let mut layer = RgbaImage::new(width.max(1), height.max(1));

    for line in lines {
        let text_width = measure_text(&font, &line);
        let cursor_x = match align {
            "left" => 2.0,
            "right" => (width as f32) - text_width - 2.0,
            _ => ((width as f32) - text_width) / 2.0,
        }.max(1.0);
    let cursor_y = top_y + ascent;
        draw_line(&font, &mut layer, &line, cursor_x, cursor_y, color_rgba);
        top_y += line_height;
    }
    Some(layer)
}

fn wrapped_lines(font: &PxScaleFont<FontRef<'_>>, text: &str, max_width: f32) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.lines() {
        let mut current = String::new();
        for word in paragraph.split(' ') {
            let candidate = if current.is_empty() {
                word.to_string()
            } else {
                format!("{current} {word}")
            };
            if !current.is_empty() && measure_text(font, &candidate) > max_width {
                lines.push(current);
                current = word.to_string();
            } else {
                current = candidate;
            }
        }
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

fn measure_text(font: &PxScaleFont<FontRef<'_>>, text: &str) -> f32 {
    text.chars().map(|c| font.h_advance(font.glyph_id(c))).sum()
}

fn draw_line(
    font: &PxScaleFont<FontRef<'_>>,
    layer: &mut RgbaImage,
    text: &str,
    start_x: f32,
    start_y: f32,
    color: Rgba<u8>,
) {
    let mut cursor = point(start_x, start_y);
    for character in text.chars() {
        let glyph: Glyph = font.glyph_id(character).with_scale_and_position(font.scale(), cursor);
        let advance = font.h_advance(glyph.id);
        if let Some(outline) = font.outline_glyph(glyph) {
            let bounds_min = outline.px_bounds().min;
            outline.draw(|x, y, coverage| {
                if coverage <= 0.0 { return; }
                let px = bounds_min.x + x as f32;
                let py = bounds_min.y + y as f32;
                if px < layer.width() as f32 && py < layer.height() as f32 {
                    let alpha = (coverage * 255.0).round().clamp(0.0, 255.0) as u8;
                    let xi = px as u32;
                    let yi = py as u32;
                    let existing = layer.get_pixel(xi, yi);
                    let merged = alpha.max(existing[3]);
                    layer.put_pixel(xi, yi, Rgba([color[0], color[1], color[2], merged]));
                }
            });
        }
        cursor.x += advance;
    }
}

pub fn draw_rounded_rect(
    img: &mut RgbImage,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    color: (u8, u8, u8),
    radius: u32,
    outline: Option<((u8, u8, u8), u32)>,
) {
    if w == 0 || h == 0 { return; }
    let r = radius.min(w / 2).min(h / 2) as i32;
    let rgb = Rgb([color.0, color.1, color.2]);

    for dy in 0..(h as i32) {
        let py = y + dy;
        if py < 0 || py >= img.height() as i32 { continue; }
        for dx in 0..(w as i32) {
            let px = x + dx;
            if px < 0 || px >= img.width() as i32 { continue; }

            let in_corner = if r > 0 {
                let tl = dx < r && dy < r && (dx - r).pow(2) + (dy - r).pow(2) > r.pow(2);
                let tr = dx >= (w as i32) - r && dy < r && (dx - ((w as i32) - r - 1)).pow(2) + (dy - r).pow(2) > r.pow(2);
                let bl = dx < r && dy >= (h as i32) - r && (dx - r).pow(2) + (dy - ((h as i32) - r - 1)).pow(2) > r.pow(2);
                let br = dx >= (w as i32) - r && dy >= (h as i32) - r && (dx - ((w as i32) - r - 1)).pow(2) + (dy - ((h as i32) - r - 1)).pow(2) > r.pow(2);
                tl || tr || bl || br
            } else {
                false
            };

            if in_corner {
                continue;
            }

            if let Some((out_col, border_w)) = outline {
                let is_border = dx < border_w as i32
                    || dx >= (w as i32) - border_w as i32
                    || dy < border_w as i32
                    || dy >= (h as i32) - border_w as i32;
                if is_border {
                    img.put_pixel(px as u32, py as u32, Rgb([out_col.0, out_col.1, out_col.2]));
                    continue;
                }
            }

            img.put_pixel(px as u32, py as u32, rgb);
        }
    }
}

pub fn clock_text(props: &Value) -> String {
    let fmt = props.get("format").and_then(|v| v.as_str()).unwrap_or("%H:%M:%S");
    let _tz_str = props.get("timezone").and_then(|v| v.as_str()).unwrap_or("");
    let now = chrono::Local::now();
    now.format(fmt).to_string()
}

pub fn ha_text(props: &Value, ctx: &Value) -> String {
    let entity_id = props.get("entityId").and_then(|v| v.as_str()).unwrap_or("");
    let entity = ctx.get("homeAssistant").and_then(|h| h.get(entity_id));
    if entity.is_none() {
        return "HA: waiting".to_string();
    }
    let ent = entity.unwrap();
    if ent.get("error").and_then(|v| v.as_bool()) == Some(true) {
        return "HA: unavailable".to_string();
    }
    let attr_name = props.get("attribute").and_then(|v| v.as_str()).unwrap_or("");
    let raw_val = if !attr_name.is_empty() {
        ent.get("attributes").and_then(|a| a.get(attr_name)).cloned().unwrap_or(Value::String("unknown".into()))
    } else {
        ent.get("state").cloned().unwrap_or(Value::String("unknown".into()))
    };

    let mut val_str = match &raw_val {
        Value::String(s) => s.clone(),
        Value::Number(n) => {
            let decimals = props.get("decimals").and_then(|v| v.as_u64()).unwrap_or(1) as usize;
            format!("{:.prec$}", n.as_f64().unwrap_or(0.0), prec = decimals)
        }
        _ => raw_val.to_string(),
    };

    if let Ok(num) = val_str.parse::<f64>() {
        let decimals = props.get("decimals").and_then(|v| v.as_u64()).unwrap_or(1) as usize;
        val_str = format!("{:.prec$}", num, prec = decimals);
    }

    let mut suffix = props.get("suffix").and_then(|v| v.as_str()).unwrap_or("").to_string();
    if suffix.is_empty() && props.get("showUnit").and_then(|v| v.as_bool()).unwrap_or(true) && attr_name.is_empty() {
        if let Some(unit) = ent.get("attributes").and_then(|a| a.get("unit_of_measurement")).and_then(|v| v.as_str()) {
            suffix = unit.to_string();
        }
    }

    let spacing = if !suffix.is_empty() && !suffix.starts_with('°') && !suffix.starts_with('%') {
        " "
    } else {
        ""
    };

    let prefix = props.get("prefix").and_then(|v| v.as_str()).unwrap_or("");
    format!("{prefix}{val_str}{spacing}{suffix}")
}

pub fn fit_image(source: &RgbImage, width: u32, height: u32, fit: &str) -> RgbImage {
    let width = width.max(1);
    let height = height.max(1);
    let (sw, sh) = (source.width().max(1), source.height().max(1));
    let filter = FilterType::Lanczos3;
    match fit {
        "stretch" => image::imageops::resize(source, width, height, filter),
        "cover" => {
            // PIL ImageOps.fit: largest centered crop matching the target
            // aspect ratio, then resize to exactly width x height.
            let target_aspect = width as f64 / height as f64;
            let aspect = sw as f64 / sh as f64;
            let (crop_w, crop_h) = if aspect >= target_aspect {
                ((sh as f64 * target_aspect).round() as u32, sh)
            } else {
                (sw, (sw as f64 / target_aspect).round() as u32)
            };
            let crop_w = crop_w.clamp(1, sw);
            let crop_h = crop_h.clamp(1, sh);
            let left = (sw - crop_w) / 2;
            let top = (sh - crop_h) / 2;
            let cropped = image::imageops::crop_imm(source, left, top, crop_w, crop_h).to_image();
            image::imageops::resize(&cropped, width, height, filter)
        }
        _ => {
            let scale = f64::min(width as f64 / sw as f64, height as f64 / sh as f64);
            let nw = ((sw as f64 * scale).round() as u32).clamp(1, width);
            let nh = ((sh as f64 * scale).round() as u32).clamp(1, height);
            let resized = image::imageops::resize(source, nw, nh, filter);
            let mut out = RgbImage::from_pixel(width, height, Rgb([0, 0, 0]));
            image::imageops::overlay(&mut out, &resized, ((width - nw) as i64) / 2, ((height - nh) as i64) / 2);
            out
        }
    }
}

pub fn render(
    layout: &Value,
    ctx: &Value,
    images: &HashMap<String, Arc<RgbImage>>,
) -> RgbImage {
    let (width, height) = layout_dimensions(layout);
    let bg_color = parse_color(layout.get("background").unwrap_or(&Value::Null), (10, 10, 30));
    let mut img = RgbImage::from_pixel(width as u32, height as u32, Rgb([bg_color.0, bg_color.1, bg_color.2]));

    let pressed_set: Vec<String> = ctx
        .get("pressed")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect())
        .unwrap_or_default();

    if let Some(elements) = layout.get("elements").and_then(|v| v.as_array()) {
        for elem in elements {
            if elem.get("visible").and_then(|v| v.as_bool()) == Some(false) {
                continue;
            }

            let elem_id = elem.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let elem_type = elem.get("type").and_then(|v| v.as_str()).unwrap_or("");
            let frame = elem.get("frame").unwrap_or(&Value::Null);
            let style = elem.get("style").unwrap_or(&Value::Null);
            let props = elem.get("props").unwrap_or(&Value::Null);

            let fx = frame.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let fy = frame.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let fw = frame.get("w").and_then(|v| v.as_u64()).unwrap_or(100) as u32;
            let fh = frame.get("h").and_then(|v| v.as_u64()).unwrap_or(40) as u32;

            match elem_type {
                "color-block" => {
                    let col = parse_color(style.get("color").unwrap_or(&Value::Null), (50, 80, 170));
                    let rad = style.get("radius").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
                    draw_rounded_rect(&mut img, fx, fy, fw, fh, col, rad, None);
                }
                "button" | "design-link" => {
                    let mut col = parse_color(style.get("color").unwrap_or(&Value::Null), (34, 85, 170));
                    if pressed_set.contains(&elem_id.to_string()) {
                        col = (col.0.saturating_add(70), col.1.saturating_add(70), col.2.saturating_add(70));
                    }
                    let rad = style.get("radius").and_then(|v| v.as_u64()).unwrap_or(8) as u32;
                    draw_rounded_rect(&mut img, fx, fy, fw, fh, col, rad, Some(((255, 255, 255), 2)));

                    let text_color = if brightness(col) > 140.0 { "#000000" } else { "#ffffff" };
                    let label = props.get("label").and_then(|v| v.as_str()).unwrap_or("Button");
                    let btn_style = serde_json::json!({
                        "fontSize": style.get("fontSize").and_then(|v| v.as_u64()).unwrap_or(16),
                        "color": text_color,
                        "align": "center",
                        "verticalAlign": "middle"
                    });
                    draw_text_box(&mut img, fx, fy, fw, fh, label, &btn_style, false);
                }
                "ha-toggle" => {
                    let entity_id = props.get("entityId").and_then(|v| v.as_str()).unwrap_or("");
                    let entity = ctx.get("homeAssistant").and_then(|h| h.get(entity_id));
                    let is_on = entity.and_then(|e| e.get("state")).and_then(|s| s.as_str()) == Some("on");
                    let on_col = parse_color(style.get("color").unwrap_or(&Value::Null), (242, 201, 76));
                    let off_col = parse_color(style.get("offColor").unwrap_or(&Value::Null), (53, 59, 72));
                    let mut col = if is_on { on_col } else { off_col };
                    if pressed_set.contains(&elem_id.to_string()) {
                        col = (col.0.saturating_add(50), col.1.saturating_add(50), col.2.saturating_add(50));
                    }
                    let rad = style.get("radius").and_then(|v| v.as_u64()).unwrap_or(8) as u32;
                    draw_rounded_rect(&mut img, fx, fy, fw, fh, col, rad, Some(((255, 255, 255), 2)));

                    let state_label = if entity.and_then(|e| e.get("error")).and_then(|v| v.as_bool()) == Some(true) {
                        "ERR"
                    } else if is_on {
                        "ON"
                    } else {
                        "OFF"
                    };
                    let label = props.get("label").and_then(|v| v.as_str()).unwrap_or("Light");
                    let text = format!("{label}: {state_label}");
                    let text_color = if brightness(col) > 140.0 { "#000000" } else { "#ffffff" };
                    let btn_style = serde_json::json!({
                        "fontSize": style.get("fontSize").and_then(|v| v.as_u64()).unwrap_or(16),
                        "color": text_color,
                        "align": "center",
                        "verticalAlign": "middle"
                    });
                    draw_text_box(&mut img, fx, fy, fw, fh, &text, &btn_style, false);
                }
                "text" => {
                    let text = props.get("text").and_then(|v| v.as_str()).unwrap_or("");
                    let wrap = props.get("wrap").and_then(|v| v.as_bool()).unwrap_or(true);
                    draw_text_box(&mut img, fx, fy, fw, fh, text, style, wrap);
                }
                "clock" => {
                    let text = clock_text(props);
                    draw_text_box(&mut img, fx, fy, fw, fh, &text, style, false);
                }
                "external-text" => {
                    let src_id = props.get("sourceId").and_then(|v| v.as_str()).unwrap_or("");
                    let text = ctx.get("external").and_then(|e| e.get(src_id)).and_then(|v| v.as_str()).unwrap_or("…");
                    let wrap = props.get("wrap").and_then(|v| v.as_bool()).unwrap_or(true);
                    draw_text_box(&mut img, fx, fy, fw, fh, text, style, wrap);
                }
                "ha-state" => {
                    let text = ha_text(props, ctx);
                    let wrap = props.get("wrap").and_then(|v| v.as_bool()).unwrap_or(true);
                    draw_text_box(&mut img, fx, fy, fw, fh, &text, style, wrap);
                }
                "script" => {
                    let code = props.get("code").and_then(|v| v.as_str()).unwrap_or("");
                    let wrap = props.get("wrap").and_then(|v| v.as_bool()).unwrap_or(true);
                    match execute_script(code, fw, fh, ctx) {
                        Ok(ScriptOutput { image: Some(script_img), .. }) => {
                            for dy in 0..fh {
                                let py = fy + dy as i32;
                                if py < 0 || py >= img.height() as i32 { continue; }
                                for dx in 0..fw {
                                    let px = fx + dx as i32;
                                    if px < 0 || px >= img.width() as i32 { continue; }
                                    let p = script_img.get_pixel(dx, dy);
                                    if p[3] > 0 {
                                        img.put_pixel(px as u32, py as u32, Rgb([p[0], p[1], p[2]]));
                                    }
                                }
                            }
                        }
                        Ok(ScriptOutput { text: Some(text), .. }) => {
                            draw_text_box(&mut img, fx, fy, fw, fh, &text, style, wrap);
                        }
                        Err(e) => {
                            let err_style = serde_json::json!({
                                "color": "#ff6577",
                                "fontSize": 12,
                                "align": "center",
                                "verticalAlign": "middle"
                            });
                            draw_text_box(&mut img, fx, fy, fw, fh, &format!("Script: {e}"), &err_style, true);
                        }
                        _ => {}
                    }
                }
                "image" => {
                    let src = props.get("src").and_then(|v| v.as_str()).unwrap_or("");
                    let fit = props.get("fit").and_then(|v| v.as_str()).unwrap_or("contain");
                    let source = if src.is_empty() { None } else { images.get(src) };
                    if let Some(source) = source {
                        let fitted = fit_image(source, fw, fh, fit);
                        image::imageops::overlay(&mut img, &fitted, fx as i64, fy as i64);
                    } else {
                        draw_rounded_rect(&mut img, fx, fy, fw, fh, (45, 47, 55), 0, Some(((105, 110, 125), 1)));
                        let label_style = serde_json::json!({
                            "fontSize": 12,
                            "color": "#aeb4c5",
                            "align": "center",
                            "verticalAlign": "middle"
                        });
                        draw_text_box(&mut img, fx, fy, fw, fh, "image", &label_style, false);
                    }
                }
                _ => {}
            }
        }
    }

    // Draw touch dots
    if let Some(dots) = ctx.get("dots").and_then(|v| v.as_array()) {
        for dot in dots {
            if let Some(darr) = dot.as_array() {
                if darr.len() >= 2 {
                    let cx = darr[0].as_i64().unwrap_or(0) as i32;
                    let cy = darr[1].as_i64().unwrap_or(0) as i32;
                    for dy in -DOT_RADIUS..=DOT_RADIUS {
                        let py = cy + dy;
                        if py < 0 || py >= img.height() as i32 { continue; }
                        for dx in -DOT_RADIUS..=DOT_RADIUS {
                            let px = cx + dx;
                            if px < 0 || px >= img.width() as i32 { continue; }
                            let dist2 = dx * dx + dy * dy;
                            if dist2 <= DOT_RADIUS * DOT_RADIUS {
                                if dist2 >= (DOT_RADIUS - 2) * (DOT_RADIUS - 2) {
                                    img.put_pixel(px as u32, py as u32, Rgb([255, 255, 255]));
                                } else {
                                    img.put_pixel(px as u32, py as u32, Rgb([255, 60, 60]));
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    img
}

pub fn hit_test_buttons<'a>(layout: &'a Value, x: i32, y: i32) -> Option<&'a Value> {
    if let Some(elements) = layout.get("elements").and_then(|v| v.as_array()) {
        for elem in elements.iter().rev() {
            let elem_type = elem.get("type").and_then(|v| v.as_str()).unwrap_or("");
            if !["button", "design-link", "ha-toggle"].contains(&elem_type) {
                continue;
            }
            if elem.get("visible").and_then(|v| v.as_bool()) == Some(false) {
                continue;
            }
            if elem.get("locked").and_then(|v| v.as_bool()) == Some(true) {
                continue;
            }
            let frame = elem.get("frame").unwrap_or(&Value::Null);
            let fx = frame.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let fy = frame.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            let fw = frame.get("w").and_then(|v| v.as_u64()).unwrap_or(0) as i32;
            let fh = frame.get("h").and_then(|v| v.as_u64()).unwrap_or(0) as i32;

            if x >= fx && x < fx + fw && y >= fy && y < fy + fh {
                return Some(elem);
            }
        }
    }
    None
}

pub fn resolve_button_action<'a>(layout: &'a Value, element: &Value) -> Option<&'a Value> {
    let action_id = element.get("props").and_then(|p| p.get("actionId")).and_then(|v| v.as_str())?;
    layout.get("actions").and_then(|a| a.as_array()).and_then(|arr| find_by_id(arr, action_id))
}

pub fn has_clock(layout: &Value) -> bool {
    if let Some(elements) = layout.get("elements").and_then(|v| v.as_array()) {
        elements.iter().any(|e| {
            let t = e.get("type").and_then(|v| v.as_str()).unwrap_or("");
            let vis = e.get("visible").and_then(|v| v.as_bool()).unwrap_or(true);
            (t == "clock" || t == "script") && vis
        })
    } else {
        false
    }
}

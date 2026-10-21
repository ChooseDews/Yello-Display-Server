use image::{ImageFormat, Rgb, RgbImage, Rgba, RgbaImage};
use std::io::Cursor;
use yello_server::decode_image_bytes;

fn encode(format: ImageFormat) -> Vec<u8> {
    let mut buf = Cursor::new(Vec::new());
    if format == ImageFormat::Jpeg {
        let img = RgbImage::from_pixel(4, 2, Rgb([255, 40, 60]));
        img.write_to(&mut buf, format).expect("encode failed");
    } else {
        let img = RgbaImage::from_pixel(4, 2, Rgba([255, 40, 60, 255]));
        img.write_to(&mut buf, format).expect("encode failed");
    }
    buf.into_inner()
}

fn assert_decodes(format: ImageFormat) {
    let bytes = encode(format);
    let img = decode_image_bytes(&bytes).unwrap_or_else(|e| panic!("{format:?}: {e}"));
    assert_eq!(img.dimensions(), (4, 2));
    assert_eq!(img.get_pixel(0, 0).0, [255, 40, 60]);
}

#[test]
fn test_decode_png() {
    assert_decodes(ImageFormat::Png);
}

#[test]
fn test_decode_jpeg() {
    // JPEG is lossy: only check dimensions, not exact pixels.
    let bytes = encode(ImageFormat::Jpeg);
    let img = decode_image_bytes(&bytes).expect("jpeg decode failed");
    assert_eq!(img.dimensions(), (4, 2));
}

#[test]
fn test_decode_webp() {
    assert_decodes(ImageFormat::WebP);
}

#[test]
fn test_decode_gif() {
    assert_decodes(ImageFormat::Gif);
}

#[test]
fn test_decode_bmp() {
    assert_decodes(ImageFormat::Bmp);
}

#[test]
fn test_decode_rgba_png_flattens_to_rgb() {
    // Transparent-logo style source: RGB channels survive the RGBA->RGB drop.
    let mut img = RgbaImage::new(2, 2);
    img.put_pixel(0, 0, Rgba([10, 200, 30, 255]));
    img.put_pixel(1, 0, Rgba([10, 200, 30, 0]));
    let mut buf = Cursor::new(Vec::new());
    img.write_to(&mut buf, ImageFormat::Png).unwrap();
    let decoded = decode_image_bytes(&buf.into_inner()).unwrap();
    assert_eq!(decoded.get_pixel(0, 0).0, [10, 200, 30]);
}

#[test]
fn test_decode_rejects_garbage() {
    assert!(decode_image_bytes(b"not an image at all").is_err());
}

#[test]
fn test_decode_rejects_oversized_dimensions_before_allocating() {
    // Minimal 54-byte 24bpp BMP header declaring a 100000x100000 image.
    // decode_image_bytes must reject it from the header alone, before
    // trying to allocate a ~30 GB pixel buffer.
    let mut bmp = Vec::with_capacity(54);
    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&54u32.to_le_bytes()); // file size
    bmp.extend_from_slice(&0u32.to_le_bytes()); // reserved
    bmp.extend_from_slice(&54u32.to_le_bytes()); // pixel data offset
    bmp.extend_from_slice(&40u32.to_le_bytes()); // DIB header size
    bmp.extend_from_slice(&100_000i32.to_le_bytes()); // width
    bmp.extend_from_slice(&100_000i32.to_le_bytes()); // height
    bmp.extend_from_slice(&1u16.to_le_bytes()); // planes
    bmp.extend_from_slice(&24u16.to_le_bytes()); // bpp
    bmp.extend_from_slice(&0u32.to_le_bytes()); // compression
    bmp.extend_from_slice(&0u32.to_le_bytes()); // image size
    bmp.extend_from_slice(&0u32.to_le_bytes()); // x ppm
    bmp.extend_from_slice(&0u32.to_le_bytes()); // y ppm
    bmp.extend_from_slice(&0u32.to_le_bytes()); // colors used
    bmp.extend_from_slice(&0u32.to_le_bytes()); // important colors
    let err = decode_image_bytes(&bmp).unwrap_err();
    assert!(err.contains("too large"), "unexpected error: {err}");
}

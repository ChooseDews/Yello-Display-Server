//! Dirty-tile calculation for bandwidth-efficient display updates.

use image::RgbImage;

/// Calculate changed regions between two RGB images.
pub fn dirty_regions(
    current: &RgbImage,
    previous: &RgbImage,
    max_area: Option<u32>,
    tile_width: Option<u32>,
    tile_height: Option<u32>,
    full_threshold: Option<f32>,
) -> Vec<(u32, u32, u32, u32)> {
    if current.dimensions() != previous.dimensions() {
        return vec![(0, 0, current.width(), current.height())];
    }

    let width = current.width();
    let height = current.height();
    let max_area = max_area.unwrap_or(480);
    let tile_width = tile_width.unwrap_or(20);
    let tile_height = tile_height.unwrap_or(12);
    let full_threshold = full_threshold.unwrap_or(0.35);

    let total_pixels = width * height;
    if total_pixels == 0 {
        return vec![];
    }

    let mut changed_count = 0u32;
    for y in 0..height {
        for x in 0..width {
            if current.get_pixel(x, y) != previous.get_pixel(x, y) {
                changed_count += 1;
            }
        }
    }

    if changed_count == 0 {
        return vec![];
    }

    if (changed_count as f32) / (total_pixels as f32) >= full_threshold {
        return vec![(0, 0, width, height)];
    }

    let mut regions = Vec::new();
    let mut y = 0u32;
    while y < height {
        let actual_height = tile_height.min(height - y);
        let mut row_tiles = Vec::new();
        let mut x = 0u32;
        while x < width {
            let actual_width = tile_width.min(width - x);
            let mut tile_changed = false;
            'tile_check: for ty in y..(y + actual_height) {
                for tx in x..(x + actual_width) {
                    if current.get_pixel(tx, ty) != previous.get_pixel(tx, ty) {
                        tile_changed = true;
                        break 'tile_check;
                    }
                }
            }
            if tile_changed {
                row_tiles.push((x, actual_width));
            }
            x += tile_width;
        }

        let mut index = 0;
        while index < row_tiles.len() {
            let (start_x, mut region_width) = row_tiles[index];
            index += 1;
            while index < row_tiles.len() {
                let (next_x, next_width) = row_tiles[index];
                if next_x != start_x + region_width || (region_width + next_width) * actual_height > max_area {
                    break;
                }
                region_width += next_width;
                index += 1;
            }
            regions.push((start_x, y, region_width, actual_height));
        }

        y += tile_height;
    }

    regions
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgb;

    #[test]
    fn test_no_change_has_no_regions() {
        let img1 = RgbImage::new(100, 100);
        let img2 = RgbImage::new(100, 100);
        assert_eq!(dirty_regions(&img1, &img2, None, None, None, None), vec![]);
    }

    #[test]
    fn test_single_change() {
        let img1 = RgbImage::new(100, 100);
        let mut img2 = RgbImage::new(100, 100);
        img2.put_pixel(5, 5, Rgb([255, 0, 0]));
        let regions = dirty_regions(&img1, &img2, None, None, None, None);
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0], (0, 0, 20, 12));
    }
}

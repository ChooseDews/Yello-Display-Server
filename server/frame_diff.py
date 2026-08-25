"""Dirty-tile calculation for bandwidth-efficient display updates."""

from __future__ import annotations

import numpy as np


def dirty_regions(
    current: np.ndarray,
    previous: np.ndarray,
    *,
    max_area: int = 480,
    tile_width: int = 20,
    tile_height: int = 12,
    full_threshold: float = 0.35,
) -> list[tuple[int, int, int, int]]:
    if current.shape != previous.shape:
        raise ValueError("frame shapes must match")
    height, width = current.shape[:2]
    changed = np.any(current != previous, axis=2)
    changed_count = int(changed.sum())
    if changed_count == 0:
        return []
    if changed_count / (width * height) >= full_threshold:
        return [(0, 0, width, height)]

    regions: list[tuple[int, int, int, int]] = []
    for y in range(0, height, tile_height):
        row_tiles: list[tuple[int, int]] = []
        actual_height = min(tile_height, height - y)
        for x in range(0, width, tile_width):
            actual_width = min(tile_width, width - x)
            if changed[y:y + actual_height, x:x + actual_width].any():
                row_tiles.append((x, actual_width))
        index = 0
        while index < len(row_tiles):
            start_x, region_width = row_tiles[index]
            index += 1
            while index < len(row_tiles):
                next_x, next_width = row_tiles[index]
                if next_x != start_x + region_width or (region_width + next_width) * actual_height > max_area:
                    break
                region_width += next_width
                index += 1
            regions.append((start_x, y, region_width, actual_height))
    return regions

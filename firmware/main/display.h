#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#define DISPLAY_WIDTH 240
#define DISPLAY_HEIGHT 320
#define DISPLAY_LANDSCAPE_WIDTH 320
#define DISPLAY_LANDSCAPE_HEIGHT 240
#define DISPLAY_MAX_WIDTH 320

/* Convert 8-bit-per-channel RGB into an RGB565 value in the panel's native
 * byte order (high byte first in memory, i.e. byte-swapped on this
 * little-endian CPU), matching what esp_lcd streams to the ILI9341 as-is. */
#define RGB565_RAW(r, g, b) \
    (uint16_t)((((r) & 0xF8) << 8) | (((g) & 0xFC) << 3) | (((b) & 0xF8) >> 3))
#define RGB565(r, g, b) \
    (uint16_t)((RGB565_RAW(r, g, b) >> 8) | (RGB565_RAW(r, g, b) << 8))

void display_init(void);

/* Switch the panel's logical coordinate system. Portrait remains the boot/default mode. */
void display_set_orientation(bool landscape);
uint16_t display_get_width(void);
uint16_t display_get_height(void);

/* Fill an axis-aligned rect [x, x+w) x [y, y+h) with a single RGB565 color.
 * Streams the fill in small horizontal strips — never allocates a full-screen buffer. */
void display_fill_rect(uint16_t x, uint16_t y, uint16_t w, uint16_t h, uint16_t color565);

/* Blit caller-owned row-major RGB565 bytes in ILI9341-native high-byte-first
 * order into [x, x+w) x [y, y+h). The call waits for SPI DMA to finish, so
 * the caller may safely release the buffer after it returns. */
void display_draw_bitmap(uint16_t x, uint16_t y, uint16_t w, uint16_t h, const uint16_t *pixels);

/* Starts the background task that dequeues and blits server-pushed zone updates. */
void display_start_render_task(void);

/* Copies a complete yello_zone_header_t + payload message onto the render queue.
 * Applies short back-pressure during full-frame bursts; returns false only if
 * the LCD cannot free a queue slot within the bounded wait. */
bool display_enqueue_zone_update(const uint8_t *data, size_t len);

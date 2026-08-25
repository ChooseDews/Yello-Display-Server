#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "boot_screen.h"
#include "display.h"

typedef struct {
    char character;
    uint8_t rows[7];
} glyph_t;

/* Compact 5x7 uppercase font. The boot screen intentionally owns only the
 * characters used by its fixed status copy, keeping flash usage tiny. */
static const glyph_t FONT[] = {
    {' ', {0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00}},
    {'A', {0x0e, 0x11, 0x11, 0x1f, 0x11, 0x11, 0x11}},
    {'B', {0x1e, 0x11, 0x11, 0x1e, 0x11, 0x11, 0x1e}},
    {'C', {0x0e, 0x11, 0x10, 0x10, 0x10, 0x11, 0x0e}},
    {'D', {0x1e, 0x11, 0x11, 0x11, 0x11, 0x11, 0x1e}},
    {'E', {0x1f, 0x10, 0x10, 0x1e, 0x10, 0x10, 0x1f}},
    {'F', {0x1f, 0x10, 0x10, 0x1e, 0x10, 0x10, 0x10}},
    {'G', {0x0e, 0x11, 0x10, 0x17, 0x11, 0x11, 0x0f}},
    {'I', {0x1f, 0x04, 0x04, 0x04, 0x04, 0x04, 0x1f}},
    {'K', {0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11}},
    {'L', {0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1f}},
    {'M', {0x11, 0x1b, 0x15, 0x15, 0x11, 0x11, 0x11}},
    {'N', {0x11, 0x19, 0x19, 0x15, 0x13, 0x13, 0x11}},
    {'O', {0x0e, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0e}},
    {'R', {0x1e, 0x11, 0x11, 0x1e, 0x14, 0x12, 0x11}},
    {'S', {0x0f, 0x10, 0x10, 0x0e, 0x01, 0x01, 0x1e}},
    {'T', {0x1f, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04}},
    {'V', {0x11, 0x11, 0x11, 0x11, 0x11, 0x0a, 0x04}},
    {'W', {0x11, 0x11, 0x15, 0x15, 0x15, 0x0a, 0x0a}},
    {'Y', {0x11, 0x11, 0x0a, 0x04, 0x04, 0x04, 0x04}},
};

static bool s_active;

static const glyph_t *find_glyph(char character)
{
    for (size_t i = 0; i < sizeof(FONT) / sizeof(FONT[0]); i++) {
        if (FONT[i].character == character) {
            return &FONT[i];
        }
    }
    return &FONT[0];
}

static void draw_text(uint16_t x, uint16_t y, const char *text, uint8_t scale, uint16_t color)
{
    while (*text != '\0') {
        const glyph_t *glyph = find_glyph(*text++);
        for (uint8_t row = 0; row < 7; row++) {
            for (uint8_t column = 0; column < 5; column++) {
                if ((glyph->rows[row] & (1U << (4 - column))) != 0) {
                    display_fill_rect(x + column * scale, y + row * scale, scale, scale, color);
                }
            }
        }
        x += 6 * scale;
    }
}

static const char *status_text(boot_status_t status)
{
    switch (status) {
    case BOOT_STATUS_CONNECTING: return "CONNECTING";
    case BOOT_STATUS_CONNECTED:  return "ONLINE";
    case BOOT_STATUS_RETRYING:   return "RETRYING";
    case BOOT_STATUS_ERROR:      return "ERROR";
    default:                     return "WAITING";
    }
}

static uint16_t status_color(boot_status_t status)
{
    switch (status) {
    case BOOT_STATUS_CONNECTED: return RGB565(75, 214, 135);
    case BOOT_STATUS_ERROR:     return RGB565(255, 88, 106);
    case BOOT_STATUS_RETRYING:  return RGB565(255, 159, 67);
    default:                    return RGB565(242, 201, 76);
    }
}

static void draw_status_row(uint16_t y, const char *label, boot_status_t status)
{
    if (!s_active) {
        return;
    }
    const uint16_t background = RGB565(20, 24, 34);
    const uint16_t muted = RGB565(145, 154, 173);
    const uint16_t color = status_color(status);
    display_fill_rect(14, y, 212, 54, background);
    display_fill_rect(14, y, 4, 54, color);
    draw_text(28, y + 9, label, 2, muted);
    display_fill_rect(28, y + 31, 8, 8, color);
    draw_text(44, y + 29, status_text(status), 2, color);
}

void boot_screen_init(void)
{
    s_active = true;
    display_fill_rect(0, 0, DISPLAY_WIDTH, DISPLAY_HEIGHT, RGB565(9, 12, 20));
    display_fill_rect(0, 0, DISPLAY_WIDTH, 6, RGB565(242, 201, 76));
    draw_text(18, 22, "YELLO", 4, RGB565(242, 201, 76));
    draw_text(20, 60, "DEVICE BOOT", 2, RGB565(170, 179, 197));
    draw_status_row(102, "WIFI", BOOT_STATUS_WAITING);
    draw_status_row(170, "SERVER", BOOT_STATUS_WAITING);
    draw_text(20, 294, "READY FOR FRAME", 2, RGB565(104, 114, 134));
}

void boot_screen_set_wifi(boot_status_t status)
{
    draw_status_row(102, "WIFI", status);
}

void boot_screen_set_server(boot_status_t status)
{
    draw_status_row(170, "SERVER", status);
}

void boot_screen_finish(void)
{
    s_active = false;
}

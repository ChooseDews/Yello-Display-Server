#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "freertos/FreeRTOS.h"
#include "freertos/task.h"

#include "device_config.h"
#include "display.h"
#include "setup_screen.h"
#include "touch.h"

typedef struct {
    char character;
    uint8_t rows[7];
} glyph_t;

/* Small setup-only font. Lowercase input is intentionally rendered with the
 * matching uppercase shape; the header always shows whether LOWER or UPPER is
 * active, while password content remains masked. */
static const glyph_t FONT[] = {
    {' ', {0, 0, 0, 0, 0, 0, 0}},
    {'A', {14, 17, 17, 31, 17, 17, 17}}, {'B', {30, 17, 17, 30, 17, 17, 30}},
    {'C', {14, 17, 16, 16, 16, 17, 14}}, {'D', {30, 17, 17, 17, 17, 17, 30}},
    {'E', {31, 16, 16, 30, 16, 16, 31}}, {'F', {31, 16, 16, 30, 16, 16, 16}},
    {'G', {14, 17, 16, 23, 17, 17, 15}}, {'H', {17, 17, 17, 31, 17, 17, 17}},
    {'I', {31, 4, 4, 4, 4, 4, 31}}, {'J', {7, 2, 2, 2, 18, 18, 12}},
    {'K', {17, 18, 20, 24, 20, 18, 17}}, {'L', {16, 16, 16, 16, 16, 16, 31}},
    {'M', {17, 27, 21, 21, 17, 17, 17}}, {'N', {17, 25, 25, 21, 19, 19, 17}},
    {'O', {14, 17, 17, 17, 17, 17, 14}}, {'P', {30, 17, 17, 30, 16, 16, 16}},
    {'Q', {14, 17, 17, 17, 21, 18, 13}}, {'R', {30, 17, 17, 30, 20, 18, 17}},
    {'S', {15, 16, 16, 14, 1, 1, 30}}, {'T', {31, 4, 4, 4, 4, 4, 4}},
    {'U', {17, 17, 17, 17, 17, 17, 14}}, {'V', {17, 17, 17, 17, 17, 10, 4}},
    {'W', {17, 17, 17, 21, 21, 21, 10}}, {'X', {17, 17, 10, 4, 10, 17, 17}},
    {'Y', {17, 17, 10, 4, 4, 4, 4}}, {'Z', {31, 1, 2, 4, 8, 16, 31}},
    {'0', {14, 17, 19, 21, 25, 17, 14}}, {'1', {4, 12, 4, 4, 4, 4, 14}},
    {'2', {14, 17, 1, 2, 4, 8, 31}}, {'3', {30, 1, 1, 14, 1, 1, 30}},
    {'4', {2, 6, 10, 18, 31, 2, 2}}, {'5', {31, 16, 16, 30, 1, 1, 30}},
    {'6', {14, 16, 16, 30, 17, 17, 14}}, {'7', {31, 1, 2, 4, 8, 8, 8}},
    {'8', {14, 17, 17, 14, 17, 17, 14}}, {'9', {14, 17, 17, 15, 1, 1, 14}},
    {'.', {0, 0, 0, 0, 0, 6, 6}}, {',', {0, 0, 0, 0, 6, 6, 4}},
    {'-', {0, 0, 0, 31, 0, 0, 0}}, {'_', {0, 0, 0, 0, 0, 0, 31}},
    {'!', {4, 4, 4, 4, 4, 0, 4}}, {'?', {14, 17, 1, 2, 4, 0, 4}},
    {'@', {14, 17, 23, 21, 23, 16, 14}}, {'#', {10, 31, 10, 10, 31, 10, 0}},
    {'$', {4, 15, 20, 14, 5, 30, 4}}, {'%', {24, 25, 2, 4, 8, 19, 3}},
    {'^', {4, 10, 17, 0, 0, 0, 0}}, {'&', {12, 18, 20, 8, 21, 18, 13}},
    {'*', {0, 21, 14, 31, 14, 21, 0}}, {'(', {2, 4, 8, 8, 8, 4, 2}},
    {')', {8, 4, 2, 2, 2, 4, 8}}, {'=', {0, 31, 0, 31, 0, 0, 0}},
    {'+', {0, 4, 4, 31, 4, 4, 0}}, {'[', {14, 8, 8, 8, 8, 8, 14}},
    {']', {14, 2, 2, 2, 2, 2, 14}}, {'{', {3, 4, 4, 8, 4, 4, 3}},
    {'}', {24, 4, 4, 2, 4, 4, 24}}, {';', {0, 6, 6, 0, 6, 6, 4}},
    {':', {0, 6, 6, 0, 6, 6, 0}}, {'\'', {4, 4, 0, 0, 0, 0, 0}},
    {'"', {10, 10, 0, 0, 0, 0, 0}}, {'/', {1, 2, 2, 4, 8, 8, 16}},
    {'\\', {16, 8, 8, 4, 2, 2, 1}}, {'<', {2, 4, 8, 16, 8, 4, 2}},
    {'>', {8, 4, 2, 1, 2, 4, 8}},
};

typedef enum { FIELD_SSID, FIELD_PASSWORD, FIELD_HOST, FIELD_PORT, FIELD_COUNT } field_t;

static const uint16_t COLOR_BG = RGB565(9, 12, 20);
static const uint16_t COLOR_PANEL = RGB565(24, 29, 40);
static const uint16_t COLOR_KEY = RGB565(42, 49, 64);
static const uint16_t COLOR_LINE = RGB565(73, 83, 103);
static const uint16_t COLOR_TEXT = RGB565(238, 242, 249);
static const uint16_t COLOR_MUTED = RGB565(145, 154, 173);
static const uint16_t COLOR_ACCENT = RGB565(242, 201, 76);
static const uint16_t COLOR_DANGER = RGB565(255, 101, 119);

static field_t s_field;
static bool s_upper = true;
static bool s_symbols = false;
static char s_port[6];

static const glyph_t *find_glyph(char character)
{
    if (character >= 'a' && character <= 'z') character -= ('a' - 'A');
    for (size_t i = 0; i < sizeof(FONT) / sizeof(FONT[0]); i++) {
        if (FONT[i].character == character) return &FONT[i];
    }
    for (size_t i = 0; i < sizeof(FONT) / sizeof(FONT[0]); i++) {
        if (FONT[i].character == '?') return &FONT[i];
    }
    return &FONT[0];
}

static uint16_t text_width(const char *text, uint8_t scale)
{
    return strlen(text) * 6 * scale;
}

static void draw_text(uint16_t x, uint16_t y, const char *text, uint8_t scale, uint16_t color)
{
    while (*text) {
        const glyph_t *glyph = find_glyph(*text++);
        for (uint8_t row = 0; row < 7; row++) {
            for (uint8_t column = 0; column < 5; column++) {
                if (glyph->rows[row] & (1U << (4 - column))) {
                    display_fill_rect(x + column * scale, y + row * scale, scale, scale, color);
                }
            }
        }
        x += 6 * scale;
    }
}

static void draw_centered(uint16_t x, uint16_t y, uint16_t w, uint16_t h,
                          const char *text, uint8_t scale, uint16_t color)
{
    uint16_t width = text_width(text, scale);
    uint16_t tx = x + (w > width ? (w - width) / 2 : 0);
    uint16_t text_height = 7 * scale;
    draw_text(tx, y + (h > text_height ? (h - text_height) / 2 : 0), text, scale, color);
}

static void draw_button(uint16_t x, uint16_t y, uint16_t w, uint16_t h,
                        const char *label, uint16_t fill, uint16_t text_color, uint8_t scale)
{
    display_fill_rect(x, y, w, h, COLOR_LINE);
    display_fill_rect(x + 1, y + 1, w - 2, h - 2, fill);
    draw_centered(x, y, w, h, label, scale, text_color);
}

static char *field_buffer(yello_runtime_config_t *config, field_t field, size_t *capacity)
{
    switch (field) {
    case FIELD_SSID: *capacity = sizeof(config->ssid); return config->ssid;
    case FIELD_PASSWORD: *capacity = sizeof(config->password); return config->password;
    case FIELD_HOST: *capacity = sizeof(config->server_host); return config->server_host;
    default: *capacity = sizeof(s_port); return s_port;
    }
}

static void visible_value(const yello_runtime_config_t *config, field_t field, char *output, size_t output_size)
{
    const char *value;
    if (field == FIELD_SSID) value = config->ssid;
    else if (field == FIELD_PASSWORD) value = config->password;
    else if (field == FIELD_HOST) value = config->server_host;
    else value = s_port;

    size_t length = strlen(value);
    size_t shown = length > 24 ? 24 : length;
    const char *start = value + length - shown;
    if (field == FIELD_PASSWORD) {
        shown = shown < output_size - 1 ? shown : output_size - 1;
        memset(output, '*', shown);
        output[shown] = '\0';
    } else {
        strlcpy(output, start, output_size);
    }
}

static void draw_field(const yello_runtime_config_t *config, field_t field)
{
    static const char *labels[] = {"SSID", "PASSWORD", "SERVER", "PORT"};
    const uint16_t y = 22 + field * 27;
    const bool selected = field == s_field;
    display_fill_rect(4, y, 232, 25, selected ? COLOR_ACCENT : COLOR_LINE);
    display_fill_rect(5, y + 1, 230, 23, COLOR_PANEL);
    draw_text(10, y + 9, labels[field], 1, selected ? COLOR_ACCENT : COLOR_MUTED);
    char value[65];
    visible_value(config, field, value, sizeof(value));
    draw_text(76, y + 9, value, 1, COLOR_TEXT);
}

static void draw_header(const char *message, uint16_t color)
{
    display_fill_rect(0, 0, DISPLAY_WIDTH, 20, COLOR_BG);
    draw_text(6, 6, message, 1, color);
    draw_text(188, 6, s_symbols ? "SYM" : s_upper ? "UPPER" : "LOWER", 1, COLOR_MUTED);
}

static void draw_keyboard(void)
{
    const char *digits = "1234567890";
    const char *row1 = s_symbols ? "!@#$%^&*()" : "QWERTYUIOP";
    const char *row2 = s_symbols ? "-_=+[]{}?" : "ASDFGHJKL";
    const char *row3 = s_symbols ? ".,;:'\"/\\<>" : "ZXCVBNM";
    for (uint8_t i = 0; i < 10; i++) {
        char label[2] = {digits[i], 0};
        draw_button(i * 24, 134, 24, 25, label, COLOR_KEY, COLOR_TEXT, 1);
        label[0] = row1[i];
        draw_button(i * 24, 160, 24, 25, label, COLOR_KEY, COLOR_TEXT, 1);
    }
    for (uint8_t i = 0; i < 9; i++) {
        char label[2] = {row2[i], 0};
        draw_button(i * 24, 186, 24, 25, label, COLOR_KEY, COLOR_TEXT, 1);
    }
    draw_button(216, 186, 24, 25, "<", COLOR_KEY, COLOR_ACCENT, 1);

    if (s_symbols) {
        for (uint8_t i = 0; i < 10; i++) {
            char label[2] = {row3[i], 0};
            draw_button(i * 24, 212, 24, 25, label, COLOR_KEY, COLOR_TEXT, 1);
        }
    } else {
        draw_button(0, 212, 36, 25, s_upper ? "LOW" : "UP", COLOR_KEY, COLOR_ACCENT, 1);
        for (uint8_t i = 0; i < 7; i++) {
            char character = row3[i];
            if (!s_upper) character += ('a' - 'A');
            char label[2] = {character, 0};
            draw_button(36 + i * 24, 212, 24, 25, label, COLOR_KEY, COLOR_TEXT, 1);
        }
        draw_button(204, 212, 36, 25, ".", COLOR_KEY, COLOR_TEXT, 1);
    }
    draw_button(0, 238, 48, 32, s_symbols ? "ABC" : "SYM", COLOR_KEY, COLOR_ACCENT, 1);
    draw_button(48, 238, 96, 32, "SPACE", COLOR_KEY, COLOR_TEXT, 1);
    draw_button(144, 238, 24, 32, "-", COLOR_KEY, COLOR_TEXT, 1);
    draw_button(168, 238, 24, 32, "_", COLOR_KEY, COLOR_TEXT, 1);
    draw_button(192, 238, 48, 32, "CLEAR", COLOR_KEY, COLOR_DANGER, 1);
}

static void draw_setup(const yello_runtime_config_t *config, bool allow_cancel)
{
    display_fill_rect(0, 0, DISPLAY_WIDTH, DISPLAY_HEIGHT, COLOR_BG);
    draw_header("DEVICE SETUP", COLOR_ACCENT);
    for (field_t field = 0; field < FIELD_COUNT; field++) draw_field(config, field);
    draw_keyboard();
    if (allow_cancel) {
        draw_button(0, 273, 78, 47, "CANCEL", COLOR_PANEL, COLOR_MUTED, 1);
        draw_button(78, 273, 162, 47, "SAVE", COLOR_ACCENT, RGB565(25, 21, 7), 2);
    } else {
        draw_button(0, 273, 240, 47, "SAVE", COLOR_ACCENT, RGB565(25, 21, 7), 2);
    }
}

static bool can_append(field_t field, char character)
{
    if (field == FIELD_PORT) return character >= '0' && character <= '9';
    if (field == FIELD_HOST) return character != ' ';
    return true;
}

static void append_character(yello_runtime_config_t *config, char character)
{
    if (!can_append(s_field, character)) return;
    size_t capacity;
    char *buffer = field_buffer(config, s_field, &capacity);
    size_t length = strlen(buffer);
    if (length + 1 < capacity) {
        buffer[length] = character;
        buffer[length + 1] = '\0';
        draw_field(config, s_field);
    }
}

static void backspace(yello_runtime_config_t *config)
{
    size_t capacity;
    char *buffer = field_buffer(config, s_field, &capacity);
    size_t length = strlen(buffer);
    if (length) {
        buffer[length - 1] = '\0';
        draw_field(config, s_field);
    }
}

static void clear_field(yello_runtime_config_t *config)
{
    size_t capacity;
    char *buffer = field_buffer(config, s_field, &capacity);
    buffer[0] = '\0';
    draw_field(config, s_field);
}

static void handle_key(yello_runtime_config_t *config, uint16_t x, uint16_t y)
{
    if (y >= 22 && y < 130) {
        s_field = (field_t)((y - 22) / 27);
        if (s_field >= FIELD_COUNT) s_field = FIELD_PORT;
        for (field_t field = 0; field < FIELD_COUNT; field++) draw_field(config, field);
        return;
    }
    if (y >= 134 && y < 159) {
        append_character(config, "1234567890"[x / 24]);
    } else if (y >= 160 && y < 185) {
        char character = (s_symbols ? "!@#$%^&*()" : "QWERTYUIOP")[x / 24];
        if (!s_symbols && !s_upper) character += ('a' - 'A');
        append_character(config, character);
    } else if (y >= 186 && y < 211) {
        uint8_t index = x / 24;
        if (index == 9) backspace(config);
        else {
            char character = (s_symbols ? "-_=+[]{}?" : "ASDFGHJKL")[index];
            if (!s_symbols && !s_upper) character += ('a' - 'A');
            append_character(config, character);
        }
    } else if (y >= 212 && y < 237) {
        if (s_symbols) {
            append_character(config, ".,;:'\"/\\<>"[x / 24]);
        } else if (x < 36) {
            s_upper = !s_upper;
            draw_keyboard();
            draw_header("DEVICE SETUP", COLOR_ACCENT);
        } else if (x < 204) {
            char character = "ZXCVBNM"[(x - 36) / 24];
            if (!s_upper) character += ('a' - 'A');
            append_character(config, character);
        } else {
            append_character(config, '.');
        }
    } else if (y >= 238 && y < 271) {
        if (x < 48) {
            s_symbols = !s_symbols;
            draw_keyboard();
            draw_header("DEVICE SETUP", COLOR_ACCENT);
        } else if (x < 144) append_character(config, ' ');
        else if (x < 168) append_character(config, '-');
        else if (x < 192) append_character(config, '_');
        else clear_field(config);
    }
}

bool setup_screen_boot_requested(void)
{
    display_fill_rect(0, 286, DISPLAY_WIDTH, 34, COLOR_PANEL);
    draw_centered(0, 286, DISPLAY_WIDTH, 34, "HOLD SCREEN FOR SETUP", 1, COLOR_MUTED);
    TickType_t start = xTaskGetTickCount();
    TickType_t pressed_since = 0;
    while (xTaskGetTickCount() - start < pdMS_TO_TICKS(1800)) {
        uint16_t x, y;
        bool pressed = touch_poll(&x, &y);
        if (pressed) {
            if (pressed_since == 0) pressed_since = xTaskGetTickCount();
            if (xTaskGetTickCount() - pressed_since >= pdMS_TO_TICKS(700)) return true;
        } else {
            pressed_since = 0;
        }
        vTaskDelay(pdMS_TO_TICKS(30));
    }
    return false;
}

bool setup_screen_run(yello_runtime_config_t *config, bool allow_cancel)
{
    yello_runtime_config_t editing = *config;
    s_field = FIELD_SSID;
    s_upper = true;
    s_symbols = false;
    snprintf(s_port, sizeof(s_port), "%u", config->server_port);
    draw_setup(&editing, allow_cancel);

    uint16_t ignored_x, ignored_y;
    while (touch_poll(&ignored_x, &ignored_y)) vTaskDelay(pdMS_TO_TICKS(30));
    bool was_pressed = false;
    while (1) {
        uint16_t x, y;
        bool pressed = touch_poll(&x, &y);
        if (pressed && !was_pressed) {
            if (y >= 273) {
                if (allow_cancel && x < 78) return false;
                long port = strtol(s_port, NULL, 10);
                if (editing.ssid[0] == '\0' || editing.server_host[0] == '\0' || port < 1 || port > 65535) {
                    draw_header("CHECK SSID HOST PORT", COLOR_DANGER);
                } else {
                    editing.server_port = (uint16_t)port;
                    if (device_config_save(&editing)) {
                        *config = editing;
                        draw_header("SETTINGS SAVED", RGB565(99, 217, 139));
                        vTaskDelay(pdMS_TO_TICKS(500));
                        return true;
                    }
                    draw_header("SAVE FAILED", COLOR_DANGER);
                }
            } else {
                handle_key(&editing, x, y);
            }
        }
        was_pressed = pressed;
        vTaskDelay(pdMS_TO_TICKS(30));
    }
}

#include <stdlib.h>
#include <string.h>

#include "driver/spi_master.h"
#include "driver/gpio.h"
#include "esp_lcd_panel_io.h"
#include "esp_lcd_panel_ops.h"
#include "esp_lcd_panel_vendor.h"
#include "esp_lcd_ili9341.h"
#include "esp_log.h"
#include "freertos/FreeRTOS.h"
#include "freertos/queue.h"
#include "freertos/semphr.h"
#include "freertos/task.h"

#include "display.h"
#include "protocol.h"

/* TFT pins, HSPI bus, separate from the touch VSPI bus. */
#define TFT_SPI_HOST SPI2_HOST
#define TFT_PIN_MOSI 13
#define TFT_PIN_MISO 12
#define TFT_PIN_SCLK 14
#define TFT_PIN_CS 15
#define TFT_PIN_DC 2
#define TFT_PIN_RST (-1) /* not wired; panel driver soft-resets instead */
#define TFT_PIN_BACKLIGHT 21

#define TFT_PCLK_HZ (40 * 1000 * 1000)

/* Streaming fill/blit strip buffer sizing: never hold a full frame in RAM. */
#define FILL_BUF_PIXELS (DISPLAY_MAX_WIDTH * 8)

static const char *TAG = "display";
static esp_lcd_panel_handle_t s_panel = NULL;
static uint16_t s_fill_buf[FILL_BUF_PIXELS];
static SemaphoreHandle_t s_color_transfer_done = NULL;
static SemaphoreHandle_t s_display_mutex = NULL;
static uint16_t s_logical_width = DISPLAY_WIDTH;
static uint16_t s_logical_height = DISPLAY_HEIGHT;
static bool s_landscape = false;

/* esp_lcd_panel_draw_bitmap() only queues an SPI DMA transaction. The pixel
 * buffer must remain valid until this callback fires; otherwise a following
 * zone allocation can reuse the memory while DMA is still reading it. */
static bool color_transfer_done_cb(esp_lcd_panel_io_handle_t panel_io,
                                   esp_lcd_panel_io_event_data_t *event_data,
                                   void *user_ctx)
{
    BaseType_t high_priority_task_woken = pdFALSE;
    xSemaphoreGiveFromISR(s_color_transfer_done, &high_priority_task_woken);
    return high_priority_task_woken == pdTRUE;
}

static void wait_for_color_transfer(void)
{
    if (xSemaphoreTake(s_color_transfer_done, pdMS_TO_TICKS(1000)) != pdTRUE) {
        ESP_LOGE(TAG, "timed out waiting for LCD color transfer");
        abort();
    }
}

void display_init(void)
{
    gpio_reset_pin(TFT_PIN_BACKLIGHT);
    gpio_set_direction(TFT_PIN_BACKLIGHT, GPIO_MODE_OUTPUT);
    gpio_set_level(TFT_PIN_BACKLIGHT, 0); /* keep off until panel is initialized */

    spi_bus_config_t buscfg = {
        .sclk_io_num = TFT_PIN_SCLK,
        .mosi_io_num = TFT_PIN_MOSI,
        .miso_io_num = TFT_PIN_MISO,
        .quadwp_io_num = -1,
        .quadhd_io_num = -1,
        .max_transfer_sz = FILL_BUF_PIXELS * sizeof(uint16_t),
    };
    ESP_ERROR_CHECK(spi_bus_initialize(TFT_SPI_HOST, &buscfg, SPI_DMA_CH_AUTO));

    s_color_transfer_done = xSemaphoreCreateBinary();
    ESP_ERROR_CHECK(s_color_transfer_done != NULL ? ESP_OK : ESP_ERR_NO_MEM);
    s_display_mutex = xSemaphoreCreateMutex();
    ESP_ERROR_CHECK(s_display_mutex != NULL ? ESP_OK : ESP_ERR_NO_MEM);

    esp_lcd_panel_io_handle_t io_handle = NULL;
    esp_lcd_panel_io_spi_config_t io_config = {
        .dc_gpio_num = TFT_PIN_DC,
        .cs_gpio_num = TFT_PIN_CS,
        .pclk_hz = TFT_PCLK_HZ,
        .lcd_cmd_bits = 8,
        .lcd_param_bits = 8,
        .spi_mode = 0,
        .trans_queue_depth = 10,
    };
    ESP_ERROR_CHECK(esp_lcd_new_panel_io_spi((esp_lcd_spi_bus_handle_t)TFT_SPI_HOST, &io_config, &io_handle));
    const esp_lcd_panel_io_callbacks_t io_callbacks = {
        .on_color_trans_done = color_transfer_done_cb,
    };
    ESP_ERROR_CHECK(esp_lcd_panel_io_register_event_callbacks(io_handle, &io_callbacks, NULL));

    esp_lcd_panel_dev_config_t panel_config = {
        .reset_gpio_num = TFT_PIN_RST,
        .rgb_ele_order = LCD_RGB_ELEMENT_ORDER_RGB,
        .bits_per_pixel = 16,
    };
    ESP_ERROR_CHECK(esp_lcd_new_panel_ili9341(io_handle, &panel_config, &s_panel));

    ESP_ERROR_CHECK(esp_lcd_panel_reset(s_panel));
    ESP_ERROR_CHECK(esp_lcd_panel_init(s_panel));
    /* Gamma curves for the USB-C + Micro-USB CYD panel revision. Its cloned
     * controller renders photographs milky and severely shifts mid-tones with
     * the generic ILI9341 curves, even though primary color bars look valid. */
    static const uint8_t positive_gamma[] = {
        0x00, 0x0C, 0x11, 0x04, 0x11, 0x08, 0x37, 0x89,
        0x4C, 0x06, 0x0C, 0x0A, 0x2E, 0x34, 0x0F,
    };
    static const uint8_t negative_gamma[] = {
        0x00, 0x0B, 0x11, 0x05, 0x13, 0x09, 0x33, 0x67,
        0x48, 0x07, 0x0E, 0x0B, 0x23, 0x33, 0x0F,
    };
    ESP_ERROR_CHECK(esp_lcd_panel_io_tx_param(io_handle, 0xE0, positive_gamma, sizeof(positive_gamma)));
    ESP_ERROR_CHECK(esp_lcd_panel_io_tx_param(io_handle, 0xE1, negative_gamma, sizeof(negative_gamma)));
    ESP_ERROR_CHECK(esp_lcd_panel_invert_color(s_panel, false));
    /* This panel's native GRAM is landscape (long axis = CASET). Without swap_xy,
     * writes past ~column 156 of our portrait 240-wide frame silently miss the
     * addressable window (confirmed by direct hardware test) instead of erroring. */
    ESP_ERROR_CHECK(esp_lcd_panel_swap_xy(s_panel, true));
    ESP_ERROR_CHECK(esp_lcd_panel_mirror(s_panel, false, false));
    ESP_ERROR_CHECK(esp_lcd_panel_disp_on_off(s_panel, true));

    gpio_set_level(TFT_PIN_BACKLIGHT, 1);

    ESP_LOGI(TAG, "display initialized (%dx%d)", DISPLAY_WIDTH, DISPLAY_HEIGHT);
}

void display_set_orientation(bool landscape)
{
    if (s_panel == NULL || landscape == s_landscape) {
        return;
    }
    xSemaphoreTake(s_display_mutex, portMAX_DELAY);
    ESP_ERROR_CHECK(esp_lcd_panel_swap_xy(s_panel, !landscape));
    ESP_ERROR_CHECK(esp_lcd_panel_mirror(s_panel, false, false));
    s_landscape = landscape;
    s_logical_width = landscape ? DISPLAY_LANDSCAPE_WIDTH : DISPLAY_WIDTH;
    s_logical_height = landscape ? DISPLAY_LANDSCAPE_HEIGHT : DISPLAY_HEIGHT;
    xSemaphoreGive(s_display_mutex);
    display_fill_rect(0, 0, s_logical_width, s_logical_height, RGB565(0, 0, 0));
    ESP_LOGI(TAG, "orientation changed to %s (%ux%u)", landscape ? "landscape" : "portrait",
             s_logical_width, s_logical_height);
}

uint16_t display_get_width(void)
{
    return s_logical_width;
}

uint16_t display_get_height(void)
{
    return s_logical_height;
}

void display_fill_rect(uint16_t x, uint16_t y, uint16_t w, uint16_t h, uint16_t color565)
{
    if (w == 0 || h == 0) {
        return;
    }
    xSemaphoreTake(s_display_mutex, portMAX_DELAY);
    uint16_t rows_per_strip = FILL_BUF_PIXELS / w;
    if (rows_per_strip == 0) {
        rows_per_strip = 1;
    }
    uint16_t fill_count = (rows_per_strip * w < FILL_BUF_PIXELS) ? (rows_per_strip * w) : FILL_BUF_PIXELS;
    for (uint16_t i = 0; i < fill_count; i++) {
        s_fill_buf[i] = color565;
    }

    uint16_t rows_done = 0;
    while (rows_done < h) {
        uint16_t strip_rows = rows_per_strip;
        if (strip_rows > (h - rows_done)) {
            strip_rows = h - rows_done;
        }
        ESP_ERROR_CHECK(esp_lcd_panel_draw_bitmap(
            s_panel, x, y + rows_done, x + w, y + rows_done + strip_rows, s_fill_buf));
        wait_for_color_transfer();
        rows_done += strip_rows;
    }
    xSemaphoreGive(s_display_mutex);
}

void display_draw_bitmap(uint16_t x, uint16_t y, uint16_t w, uint16_t h, const uint16_t *pixels)
{
    if (w == 0 || h == 0) {
        return;
    }
    xSemaphoreTake(s_display_mutex, portMAX_DELAY);
    ESP_ERROR_CHECK(esp_lcd_panel_draw_bitmap(s_panel, x, y, x + w, y + h, pixels));
    wait_for_color_transfer();
    xSemaphoreGive(s_display_mutex);
}

#define ZONE_QUEUE_DEPTH 16

typedef struct {
    uint8_t *data;
    size_t len;
} zone_update_msg_t;

static QueueHandle_t s_zone_queue = NULL;

static void render_task(void *arg)
{
    zone_update_msg_t msg;
    while (1) {
        if (xQueueReceive(s_zone_queue, &msg, portMAX_DELAY) != pdTRUE) {
            continue;
        }

        if (msg.len >= 3 && msg.data[0] == YELLO_PROTO_MAGIC &&
            msg.data[1] == YELLO_PROTO_VERSION && msg.data[2] == YELLO_MSG_DEVICE_CONFIG) {
            if (msg.len == sizeof(yello_device_config_t)) {
                const yello_device_config_t *config = (const yello_device_config_t *)msg.data;
                bool landscape = config->orientation == YELLO_ORIENTATION_LANDSCAPE;
                uint16_t expected_width = landscape ? DISPLAY_LANDSCAPE_WIDTH : DISPLAY_WIDTH;
                uint16_t expected_height = landscape ? DISPLAY_LANDSCAPE_HEIGHT : DISPLAY_HEIGHT;
                if (config->width == expected_width && config->height == expected_height) {
                    display_set_orientation(landscape);
                } else {
                    ESP_LOGW(TAG, "invalid orientation dimensions %ux%u", config->width, config->height);
                }
            }
            free(msg.data);
            continue;
        }

        const yello_zone_header_t *hdr = (const yello_zone_header_t *)msg.data;
        size_t expected_len = sizeof(*hdr) + hdr->payload_len;
        if (msg.len < sizeof(*hdr) || msg.len != expected_len ||
            hdr->magic != YELLO_PROTO_MAGIC || hdr->version != YELLO_PROTO_VERSION ||
            hdr->msg_type != YELLO_MSG_ZONE_UPDATE) {
            ESP_LOGW(TAG, "dropping malformed zone update (len=%u)", (unsigned)msg.len);
            free(msg.data);
            continue;
        }

        bool bounds_valid = hdr->w > 0 && hdr->h > 0 &&
                            hdr->x < s_logical_width && hdr->y < s_logical_height &&
                            hdr->w <= s_logical_width - hdr->x &&
                            hdr->h <= s_logical_height - hdr->y;
        uint32_t expected_payload_len = bounds_valid ? (uint32_t)hdr->w * (uint32_t)hdr->h * 2U : 0;
        if (hdr->format == YELLO_FMT_RAW_RGB565 && bounds_valid &&
            hdr->payload_len == expected_payload_len) {
            ESP_LOGD(TAG, "blit x=%u y=%u w=%u h=%u", hdr->x, hdr->y, hdr->w, hdr->h);
            display_draw_bitmap(hdr->x, hdr->y, hdr->w, hdr->h, (const uint16_t *)(msg.data + sizeof(*hdr)));
        } else {
            ESP_LOGW(TAG, "dropping invalid zone update format=%d x=%u y=%u w=%u h=%u payload_len=%u",
                     hdr->format, hdr->x, hdr->y, hdr->w, hdr->h, (unsigned)hdr->payload_len);
        }

        free(msg.data);
    }
}

void display_start_render_task(void)
{
    s_zone_queue = xQueueCreate(ZONE_QUEUE_DEPTH, sizeof(zone_update_msg_t));
    xTaskCreate(render_task, "display_render", 4096, NULL, 5, NULL);
}

bool display_enqueue_zone_update(const uint8_t *data, size_t len)
{
    uint8_t *copy = malloc(len);
    if (!copy) {
        ESP_LOGW(TAG, "oom copying zone update (%u bytes)", (unsigned)len);
        return false;
    }
    memcpy(copy, data, len);

    zone_update_msg_t msg = { .data = copy, .len = len };
    /* Back-pressure the WebSocket callback briefly while the LCD drains a
     * reconnect/full-frame burst. Dropping one of those zones leaves stale
     * pixels on screen until the next periodic full resync. */
    if (xQueueSend(s_zone_queue, &msg, pdMS_TO_TICKS(100)) != pdTRUE) {
        free(copy);
        return false;
    }
    return true;
}

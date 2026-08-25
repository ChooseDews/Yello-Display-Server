#include "driver/spi_master.h"
#include "esp_lcd_panel_io.h"
#include "esp_lcd_touch.h"
#include "esp_lcd_touch_xpt2046.h"
#include "esp_log.h"

#include "display.h"
#include "touch.h"

/* XPT2046 touch pins, separate VSPI bus from the TFT's HSPI bus. */
#define TOUCH_SPI_HOST SPI3_HOST
#define TOUCH_PIN_MOSI 32
#define TOUCH_PIN_MISO 39
#define TOUCH_PIN_SCLK 25
#define TOUCH_PIN_CS 33
#define TOUCH_PIN_IRQ 36

static const char *TAG = "touch";
static esp_lcd_touch_handle_t s_touch = NULL;
static bool s_landscape = false;

void touch_init(void)
{
    spi_bus_config_t buscfg = {
        .sclk_io_num = TOUCH_PIN_SCLK,
        .mosi_io_num = TOUCH_PIN_MOSI,
        .miso_io_num = TOUCH_PIN_MISO,
        .quadwp_io_num = -1,
        .quadhd_io_num = -1,
        .max_transfer_sz = 32,
    };
    ESP_ERROR_CHECK(spi_bus_initialize(TOUCH_SPI_HOST, &buscfg, SPI_DMA_DISABLED));

    esp_lcd_panel_io_handle_t io_handle = NULL;
    esp_lcd_panel_io_spi_config_t io_config = ESP_LCD_TOUCH_IO_SPI_XPT2046_CONFIG(TOUCH_PIN_CS);
    ESP_ERROR_CHECK(esp_lcd_new_panel_io_spi((esp_lcd_spi_bus_handle_t)TOUCH_SPI_HOST, &io_config, &io_handle));

    esp_lcd_touch_config_t tp_cfg = {
        .x_max = DISPLAY_WIDTH,
        .y_max = DISPLAY_HEIGHT,
        .rst_gpio_num = -1,
        .int_gpio_num = TOUCH_PIN_IRQ,
        .flags = {
            .swap_xy = 0,
            .mirror_x = 0,
            /* Touch panel's y ADC axis runs opposite to the display's y
             * (verified against on-screen content: dots landed y-flipped). */
            .mirror_y = 1,
        },
    };
    ESP_ERROR_CHECK(esp_lcd_touch_new_spi_xpt2046(io_handle, &tp_cfg, &s_touch));

    ESP_LOGI(TAG, "touch initialized");
}

bool touch_poll(uint16_t *x, uint16_t *y)
{
    uint16_t strength[1];
    uint8_t count = 0;

    ESP_ERROR_CHECK(esp_lcd_touch_read_data(s_touch));
    bool touched = esp_lcd_touch_get_coordinates(s_touch, x, y, strength, &count, 1);
    if (touched && s_landscape) {
        /* Portrait mode swaps the ILI9341 axes. Native landscape addresses are
         * the corresponding transposed touch coordinates. */
        uint16_t portrait_x = *x;
        *x = *y < DISPLAY_LANDSCAPE_WIDTH ? *y : DISPLAY_LANDSCAPE_WIDTH - 1;
        *y = portrait_x < DISPLAY_LANDSCAPE_HEIGHT ? portrait_x : DISPLAY_LANDSCAPE_HEIGHT - 1;
    }
    return touched;
}

void touch_set_orientation(bool landscape)
{
    s_landscape = landscape;
    ESP_LOGI(TAG, "orientation changed to %s", landscape ? "landscape" : "portrait");
}

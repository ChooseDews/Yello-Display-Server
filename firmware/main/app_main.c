#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "driver/gpio.h"
#include "esp_log.h"
#include "esp_system.h"

#include "display.h"
#include "boot_screen.h"
#include "device_config.h"
#include "protocol.h"
#include "setup_screen.h"
#include "touch.h"
#include "wifi.h"
#include "ws_client.h"

/* RGB status LED, active-low */
#define LED_R_GPIO 4
#define LED_G_GPIO 16
#define LED_B_GPIO 17

static const char *TAG = "yello_m1";

static void led_set(bool r, bool g, bool b)
{
    gpio_set_level(LED_R_GPIO, !r);
    gpio_set_level(LED_G_GPIO, !g);
    gpio_set_level(LED_B_GPIO, !b);
}

void app_main(void)
{
    const gpio_num_t led_pins[] = { LED_R_GPIO, LED_G_GPIO, LED_B_GPIO };
    for (size_t i = 0; i < sizeof(led_pins) / sizeof(led_pins[0]); i++) {
        gpio_reset_pin(led_pins[i]);
        gpio_set_direction(led_pins[i], GPIO_MODE_OUTPUT);
    }

    ESP_LOGI(TAG, "Yello device booting");

    display_init();

    boot_screen_init();

    touch_init();

    device_config_init_storage();
    yello_runtime_config_t device_config;
    bool persisted_config = device_config_load(&device_config);
    bool configured = device_config_is_valid(&device_config);
    bool open_setup = !configured;
    if (configured) {
        open_setup = setup_screen_boot_requested();
    }
    if (open_setup) {
        ESP_LOGI(TAG, "opening on-device setup%s", configured ? " by boot gesture" : " for first boot");
        if (!setup_screen_run(&device_config, configured)) {
            ESP_LOGI(TAG, "on-device setup cancelled");
        } else {
            persisted_config = true;
        }
        boot_screen_init();
    }
    if (!device_config_is_valid(&device_config)) {
        ESP_LOGE(TAG, "device configuration is still incomplete; restarting setup");
        setup_screen_run(&device_config, false);
        boot_screen_init();
    }
    ESP_LOGI(TAG, "configuration source: %s", persisted_config ? "NVS" : "build defaults");

    ESP_LOGI(TAG, "connecting to wifi");
    boot_screen_set_wifi(BOOT_STATUS_CONNECTING);
    wifi_init_sta(&device_config);

    ESP_LOGI(TAG, "starting render task");
    display_start_render_task();

    ESP_LOGI(TAG, "connecting to websocket server");
    boot_screen_set_server(BOOT_STATUS_CONNECTING);
    ws_client_init(&device_config);

    led_set(false, false, true);
    bool was_pressed = false;

    uint16_t last_x = 0, last_y = 0;
    TickType_t next_heartbeat = xTaskGetTickCount();

    while (1) {
        uint16_t x, y;
        bool pressed = touch_poll(&x, &y);
        if (pressed && !was_pressed) {
            ESP_LOGI(TAG, "touch down x=%u y=%u", x, y);
            ws_client_send_touch(YELLO_TOUCH_DOWN, x, y);
        } else if (pressed && was_pressed && (x != last_x || y != last_y)) {
            ws_client_send_touch(YELLO_TOUCH_MOVE, x, y);
        } else if (!pressed && was_pressed) {
            ESP_LOGI(TAG, "touch up x=%u y=%u", last_x, last_y);
            ws_client_send_touch(YELLO_TOUCH_UP, last_x, last_y);
        }
        if (pressed) {
            last_x = x;
            last_y = y;
        }
        was_pressed = pressed;

        TickType_t now = xTaskGetTickCount();
        if ((int32_t)(now - next_heartbeat) >= 0) {
            ws_client_send_status(wifi_get_rssi());
            next_heartbeat = now + pdMS_TO_TICKS(10000);
        }

        vTaskDelay(pdMS_TO_TICKS(50));
    }
}

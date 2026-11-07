#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "driver/gpio.h"
#include "esp_log.h"
#include "esp_system.h"

#include "display.h"
#include "boot_screen.h"
#include "frame_patch.h"
#include "device_config.h"
#include "protocol.h"
#include "serial_config.h"
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
    /* USB-serial configuration listens from here on — including while
     * the touchscreen setup is up — so the web flasher can write the
     * Wi-Fi/server settings right after flashing. A successful write
     * saves to NVS and reboots the device into the new configuration. */
    serial_config_start();
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
    frame_patch_init();

    ESP_LOGI(TAG, "connecting to websocket server");
    boot_screen_set_server(BOOT_STATUS_CONNECTING);
    ws_client_init(&device_config);

    led_set(false, false, true);

    /* Touch debounce: the XPT2046's pressed signal flickers as contact
     * is made and broken (resistive bounce), and every extra DOWN the
     * firmware emits fires the element under the finger again on the
     * server — one physical tap could register as two clicks. A press
     * or release therefore has to be seen on two consecutive polls
     * (the loop polls every ~50 ms) before it is believed; a flicker
     * back to the reported state cancels the pending transition
     * instead of emitting anything. The server additionally gates
     * actions to DOWN events and debounces repeats, so this is the
     * first of two layers, not the only one. */
    bool stable_pressed = false;  /* debounced state already reported */
    bool pending_pressed = false; /* candidate state seen once so far */
    bool has_pending = false;
    uint16_t last_x = 0, last_y = 0;
    TickType_t next_heartbeat = xTaskGetTickCount();

    while (1) {
        uint16_t x = 0, y = 0;
        bool pressed = touch_poll(&x, &y);
        bool emit_down = false;
        bool emit_up = false;
        if (pressed == stable_pressed) {
            has_pending = false; /* flicker died out; nothing to report */
        } else if (!has_pending || pending_pressed != pressed) {
            pending_pressed = pressed;
            has_pending = true; /* first sighting: wait for confirmation */
        } else {
            stable_pressed = pressed; /* second sighting in a row: commit */
            has_pending = false;
            emit_down = stable_pressed;
            emit_up = !stable_pressed;
        }

        if (emit_down) {
            last_x = x;
            last_y = y;
            ESP_LOGI(TAG, "touch down x=%u y=%u", x, y);
            ws_client_send_touch(YELLO_TOUCH_DOWN, x, y);
        } else if (emit_up) {
            ESP_LOGI(TAG, "touch up x=%u y=%u", last_x, last_y);
            ws_client_send_touch(YELLO_TOUCH_UP, last_x, last_y);
        } else if (stable_pressed && pressed && (x != last_x || y != last_y)) {
            ws_client_send_touch(YELLO_TOUCH_MOVE, x, y);
            last_x = x;
            last_y = y;
        }

        TickType_t now = xTaskGetTickCount();
        if ((int32_t)(now - next_heartbeat) >= 0) {
            ws_client_send_status(wifi_get_rssi());
            next_heartbeat = now + pdMS_TO_TICKS(10000);
        }

        vTaskDelay(pdMS_TO_TICKS(50));
    }
}

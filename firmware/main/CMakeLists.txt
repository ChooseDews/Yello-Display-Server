idf_component_register(
    SRCS "app_main.c" "boot_screen.c" "device_config.c" "display.c" "setup_screen.c" "touch.c" "wifi.c" "ws_client.c"
    INCLUDE_DIRS "."
    PRIV_REQUIRES driver esp_lcd esp_lcd_ili9341 esp_lcd_touch esp_lcd_touch_xpt2046
                  esp_wifi esp_event esp_netif nvs_flash esp_websocket_client
)

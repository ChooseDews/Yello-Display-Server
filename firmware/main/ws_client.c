#include <stdio.h>
#include <string.h>

#include "esp_event.h"
#include "esp_log.h"
#include "esp_mac.h"
#include "esp_system.h"
#include "esp_websocket_client.h"

#include "esp_timer.h"
#include "freertos/FreeRTOS.h"

#include "display.h"
#include "boot_screen.h"
#include "protocol.h"
#include "touch.h"
#include "ws_client.h"

static const char *TAG = "ws_client";
static esp_websocket_client_handle_t s_client = NULL;

/* Reassembly buffer for one logical WS binary message, which may arrive split
 * across multiple WEBSOCKET_EVENT_DATA events (esp_websocket_client's own RX
 * buffer is smaller than our zone-update messages). Sized well above the
 * ~1KB per-message budget the wire protocol targets. */
#define WS_MAX_MSG_BYTES 4096
static uint8_t s_msg_buf[WS_MAX_MSG_BYTES];

#define WS_OPCODE_BINARY 0x02

static void restart_task(void *arg)
{
    vTaskDelay(pdMS_TO_TICKS(150));
    esp_restart();
}

static void handle_data_event(esp_websocket_event_data_t *data)
{
    if (data->op_code != WS_OPCODE_BINARY || data->payload_len <= 0) {
        return;
    }
    if (data->payload_len > WS_MAX_MSG_BYTES) {
        ESP_LOGW(TAG, "message too large (%d > %d bytes), dropping", data->payload_len, WS_MAX_MSG_BYTES);
        return;
    }
    if (data->payload_offset + data->data_len > WS_MAX_MSG_BYTES) {
        ESP_LOGW(TAG, "fragment overruns reassembly buffer, dropping");
        return;
    }

    memcpy(s_msg_buf + data->payload_offset, data->data_ptr, data->data_len);

    if (data->payload_offset + data->data_len >= data->payload_len) {
        if (data->payload_len < 3 || s_msg_buf[0] != YELLO_PROTO_MAGIC ||
            s_msg_buf[1] != YELLO_PROTO_VERSION) {
            ESP_LOGW(TAG, "invalid message header");
            return;
        }
        uint8_t message_type = s_msg_buf[2];
        if (message_type == YELLO_MSG_ZONE_UPDATE) {
            boot_screen_finish();
            if (!display_enqueue_zone_update(s_msg_buf, data->payload_len)) {
                ESP_LOGW(TAG, "render queue full, dropping zone update");
            }
        } else if (message_type == YELLO_MSG_DEVICE_CONFIG && data->payload_len == sizeof(yello_device_config_t)) {
            const yello_device_config_t *config = (const yello_device_config_t *)s_msg_buf;
            touch_set_orientation(config->orientation == YELLO_ORIENTATION_LANDSCAPE);
            if (!display_enqueue_zone_update(s_msg_buf, data->payload_len)) {
                ESP_LOGW(TAG, "render queue full, dropping device config");
            }
        } else if (message_type == YELLO_MSG_DEVICE_COMMAND && data->payload_len == sizeof(yello_device_command_t)) {
            const yello_device_command_t *command = (const yello_device_command_t *)s_msg_buf;
            if (command->command == YELLO_COMMAND_RESTART) {
                ESP_LOGW(TAG, "restart requested by server");
                xTaskCreate(restart_task, "remote_restart", 2048, NULL, 8, NULL);
            }
        } else {
            ESP_LOGW(TAG, "unsupported message type=%u len=%d", message_type, data->payload_len);
        }
    }
}

static void ws_event_handler(void *handler_args, esp_event_base_t base, int32_t event_id, void *event_data)
{
    esp_websocket_event_data_t *data = (esp_websocket_event_data_t *)event_data;

    switch (event_id) {
    case WEBSOCKET_EVENT_CONNECTED:
        ESP_LOGI(TAG, "connected");
        boot_screen_set_server(BOOT_STATUS_CONNECTED);
        break;
    case WEBSOCKET_EVENT_DISCONNECTED:
        ESP_LOGW(TAG, "disconnected");
        boot_screen_set_server(BOOT_STATUS_RETRYING);
        break;
    case WEBSOCKET_EVENT_ERROR:
        ESP_LOGE(TAG, "error");
        boot_screen_set_server(BOOT_STATUS_ERROR);
        break;
    case WEBSOCKET_EVENT_DATA:
        handle_data_event(data);
        break;
    default:
        break;
    }
}

void ws_client_init(const yello_runtime_config_t *config)
{
    uint8_t mac[6];
    ESP_ERROR_CHECK(esp_read_mac(mac, ESP_MAC_WIFI_STA));
    char uri[192];
#ifdef CONFIG_YELLO_WS_USE_TLS
    const char *scheme = "wss";
#else
    const char *scheme = "ws";
#endif
    snprintf(uri, sizeof(uri),
             "%s://%s:%u%s?device_id=%02x%02x%02x%02x%02x%02x",
             scheme, config->server_host, config->server_port, CONFIG_YELLO_WS_PATH,
             mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]);

    esp_websocket_client_config_t ws_cfg = {
        .uri = uri,
        .enable_close_reconnect = true,
        .reconnect_timeout_ms = 2000,
        .network_timeout_ms = 10000,
    };

    s_client = esp_websocket_client_init(&ws_cfg);
    esp_websocket_register_events(s_client, WEBSOCKET_EVENT_ANY, ws_event_handler, NULL);
    esp_websocket_client_start(s_client);

    ESP_LOGI(TAG, "connecting to %s", uri);
}

bool ws_client_send_touch(uint8_t touch_type, uint16_t x, uint16_t y)
{
    if (s_client == NULL || !esp_websocket_client_is_connected(s_client)) {
        return false;
    }

    yello_touch_event_t event = {
        .magic = YELLO_PROTO_MAGIC,
        .version = YELLO_PROTO_VERSION,
        .msg_type = YELLO_MSG_TOUCH_EVENT,
        .touch_type = touch_type,
        .x = x,
        .y = y,
        .timestamp_ms = (uint32_t)(esp_timer_get_time() / 1000),
    };

    int sent = esp_websocket_client_send_bin(s_client, (const char *)&event, sizeof(event), pdMS_TO_TICKS(100));
    return sent == sizeof(event);
}

bool ws_client_send_status(int16_t wifi_rssi_dbm)
{
    if (s_client == NULL || !esp_websocket_client_is_connected(s_client)) {
        return false;
    }
    yello_device_status_t status = {
        .magic = YELLO_PROTO_MAGIC,
        .version = YELLO_PROTO_VERSION,
        .msg_type = YELLO_MSG_DEVICE_STATUS,
        .flags = 0,
        .uptime_ms = (uint32_t)(esp_timer_get_time() / 1000),
        .wifi_rssi_dbm = wifi_rssi_dbm,
        .reserved = 0,
    };
    int sent = esp_websocket_client_send_bin(s_client, (const char *)&status, sizeof(status), pdMS_TO_TICKS(100));
    return sent == sizeof(status);
}

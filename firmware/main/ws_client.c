#include <stdio.h>
#include <string.h>

#include "esp_event.h"
#include "esp_log.h"
#include "esp_mac.h"
#include "esp_system.h"
#include "esp_websocket_client.h"

#include "esp_timer.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"

#include "display.h"
#include "boot_screen.h"
#include "frame_patch.h"
#include "protocol.h"
#include "touch.h"
#include "ws_client.h"
#include "yello_version.h"

static const char *TAG = "ws_client";
static esp_websocket_client_handle_t s_client = NULL;

/* Reassembly buffer for the small control messages (DeviceConfig 8 B,
 * DeviceCommand 4 B), which may still arrive split across
 * WEBSOCKET_EVENT_DATA events. FramePatch messages are NOT reassembled:
 * at 10-90 KiB they cannot fit any sane static buffer, so their payload
 * fragments stream straight into the patch decode queue. */
#define WS_MAX_MSG_BYTES 4096
static uint8_t s_msg_buf[WS_MAX_MSG_BYTES];

#define WS_OPCODE_BINARY 0x02

typedef enum {
    MSG_KIND_UNKNOWN, /* first bytes not yet classified */
    MSG_KIND_SMALL,   /* control message: reassemble in s_msg_buf */
    MSG_KIND_PATCH,   /* FramePatch: stream fragments to the decoder */
    MSG_KIND_DROP,    /* invalid/oversize: drain until the message ends */
} msg_kind_t;

static msg_kind_t s_msg_kind = MSG_KIND_UNKNOWN;
static uint8_t s_hdr_buf[sizeof(yello_frame_patch_header_t)];
static size_t s_hdr_len = 0;     /* header bytes accumulated so far */
static bool s_patch_begun = false;
static bool s_patch_dropped = false; /* begin failed: drain, resync at end */

static void restart_task(void *arg)
{
    (void)arg;
    vTaskDelay(pdMS_TO_TICKS(150));
    esp_restart();
}

static void dispatch_small_message(const uint8_t *buf, int len)
{
    if (len < 3 || buf[0] != YELLO_PROTO_MAGIC || buf[1] != YELLO_PROTO_VERSION) {
        ESP_LOGW(TAG, "invalid message header");
        return;
    }
    uint8_t message_type = buf[2];
    if (message_type == YELLO_MSG_DEVICE_CONFIG && len == sizeof(yello_device_config_t)) {
        const yello_device_config_t *config = (const yello_device_config_t *)buf;
        touch_set_orientation(config->orientation == YELLO_ORIENTATION_LANDSCAPE);
        if (!display_enqueue_device_config(buf, len)) {
            ESP_LOGW(TAG, "render queue full, dropping device config");
        }
    } else if (message_type == YELLO_MSG_DEVICE_COMMAND && len == sizeof(yello_device_command_t)) {
        const yello_device_command_t *command = (const yello_device_command_t *)buf;
        if (command->command == YELLO_COMMAND_RESTART) {
            ESP_LOGW(TAG, "restart requested by server");
            xTaskCreate(restart_task, "remote_restart", 2048, NULL, 8, NULL);
        }
    } else {
        ESP_LOGW(TAG, "unsupported message type=%u len=%d", message_type, len);
    }
}

/* Accumulate header bytes from the current fragment; returns how many
 * bytes of `data` were consumed. */
static size_t hdr_accumulate(const uint8_t *data, size_t len, size_t want_total)
{
    if (s_hdr_len >= want_total) {
        return 0;
    }
    size_t take = want_total - s_hdr_len;
    if (take > len) {
        take = len;
    }
    memcpy(s_hdr_buf + s_hdr_len, data, take);
    s_hdr_len += take;
    return take;
}

static void begin_patch_from_header(void)
{
    const yello_frame_patch_header_t *hdr = (const yello_frame_patch_header_t *)s_hdr_buf;
    boot_screen_finish();
    if (frame_patch_begin(hdr)) {
        s_patch_begun = true;
    } else {
        /* Queue is full: never retry begin mid-message (the payload
         * head is already gone). Drain the message and let
         * frame_patch_end() send the queue-overflow resync. */
        s_patch_dropped = true;
    }
}

static void handle_data_event(esp_websocket_event_data_t *data)
{
    if (data->op_code != WS_OPCODE_BINARY || data->payload_len <= 0) {
        return;
    }
    const uint8_t *ptr = (const uint8_t *)data->data_ptr;
    size_t len = data->data_len;
    bool final_fragment = (data->payload_offset + data->data_len >= data->payload_len);

    if (data->payload_offset == 0) {
        s_msg_kind = MSG_KIND_UNKNOWN;
        s_hdr_len = 0;
        s_patch_begun = false;
        s_patch_dropped = false;
    }

    /* Keep the small-message reassembly copy for every fragment that
     * fits; classification below decides whether it is ever used. */
    if (data->payload_len <= WS_MAX_MSG_BYTES &&
        data->payload_offset + data->data_len <= WS_MAX_MSG_BYTES) {
        memcpy(s_msg_buf + data->payload_offset, data->data_ptr, data->data_len);
    }

    if (s_msg_kind == MSG_KIND_UNKNOWN) {
        size_t consumed = hdr_accumulate(ptr, len, 3);
        ptr += consumed;
        len -= consumed;
        if (s_hdr_len < 3) {
            return; /* header split across fragments; wait for more */
        }
        if (s_hdr_buf[0] != YELLO_PROTO_MAGIC || s_hdr_buf[1] != YELLO_PROTO_VERSION) {
            ESP_LOGW(TAG, "invalid message header");
            s_msg_kind = MSG_KIND_DROP;
            return;
        }
        s_msg_kind = (s_hdr_buf[2] == YELLO_MSG_FRAME_PATCH) ? MSG_KIND_PATCH : MSG_KIND_SMALL;
        if (s_msg_kind == MSG_KIND_PATCH) {
            consumed = hdr_accumulate(ptr, len, sizeof(s_hdr_buf));
            ptr += consumed;
            len -= consumed;
            if (s_hdr_len < sizeof(s_hdr_buf)) {
                return; /* header split across fragments; wait for more */
            }
            /* The header's payload_len must match this WS message. */
            const yello_frame_patch_header_t *hdr = (const yello_frame_patch_header_t *)s_hdr_buf;
            if (hdr->payload_len + sizeof(*hdr) != (uint32_t)data->payload_len) {
                ESP_LOGW(TAG, "patch payload_len mismatch (hdr %u, msg %d)",
                         (unsigned)hdr->payload_len, data->payload_len);
                s_msg_kind = MSG_KIND_DROP;
                if (final_fragment) {
                    ws_client_send_resync(YELLO_RESYNC_MALFORMED, frame_patch_last_good_seq());
                }
                return;
            }
            begin_patch_from_header();
            /* fall through to feed the rest of this fragment */
        } else {
            /* Small message: the reassembly copy above is authoritative. */
            if (final_fragment) {
                if (data->payload_len > WS_MAX_MSG_BYTES) {
                    ESP_LOGW(TAG, "message too large (%d bytes), dropping", data->payload_len);
                } else {
                    dispatch_small_message(s_msg_buf, data->payload_len);
                }
            }
            return;
        }
    } else if (s_msg_kind == MSG_KIND_PATCH && s_patch_dropped) {
        if (final_fragment) {
            frame_patch_end(); /* sends the queue-overflow resync */
            s_patch_dropped = false;
        }
        return;
    } else if (s_msg_kind == MSG_KIND_PATCH && !s_patch_begun) {
        size_t consumed = hdr_accumulate(ptr, len, sizeof(s_hdr_buf));
        ptr += consumed;
        len -= consumed;
        if (s_hdr_len < sizeof(s_hdr_buf)) {
            return;
        }
        const yello_frame_patch_header_t *hdr = (const yello_frame_patch_header_t *)s_hdr_buf;
        if (hdr->payload_len + sizeof(*hdr) != (uint32_t)data->payload_len) {
            ESP_LOGW(TAG, "patch payload_len mismatch (hdr %u, msg %d)",
                     (unsigned)hdr->payload_len, data->payload_len);
            s_msg_kind = MSG_KIND_DROP;
            if (final_fragment) {
                ws_client_send_resync(YELLO_RESYNC_MALFORMED, frame_patch_last_good_seq());
            }
            return;
        }
        begin_patch_from_header();
        /* fall through to feed the rest of this fragment */
    } else if (s_msg_kind == MSG_KIND_SMALL) {
        if (final_fragment) {
            if (data->payload_len > WS_MAX_MSG_BYTES) {
                ESP_LOGW(TAG, "message too large (%d bytes), dropping", data->payload_len);
            } else {
                dispatch_small_message(s_msg_buf, data->payload_len);
            }
        }
        return;
    } else if (s_msg_kind == MSG_KIND_DROP) {
        if (final_fragment && s_hdr_len >= 3 && s_hdr_buf[2] == YELLO_MSG_FRAME_PATCH) {
            ws_client_send_resync(YELLO_RESYNC_MALFORMED, frame_patch_last_good_seq());
        }
        return;
    }

    /* MSG_KIND_PATCH with the header consumed: stream this fragment's
     * remaining bytes to the decode queue. */
    if (s_msg_kind == MSG_KIND_PATCH && s_patch_begun) {
        if (len > 0 || final_fragment) {
            frame_patch_feed(ptr, len, final_fragment);
        }
        if (final_fragment) {
            frame_patch_end();
            s_patch_begun = false;
        }
    } else if (s_msg_kind == MSG_KIND_PATCH && s_patch_dropped && final_fragment) {
        /* Single-fragment patch whose begin failed. */
        frame_patch_end();
        s_patch_dropped = false;
    }
}

static void ws_event_handler(void *handler_args, esp_event_base_t base, int32_t event_id, void *event_data)
{
    (void)handler_args;
    (void)base;
    esp_websocket_event_data_t *data = (esp_websocket_event_data_t *)event_data;

    switch (event_id) {
    case WEBSOCKET_EVENT_CONNECTED:
        ESP_LOGI(TAG, "connected");
        boot_screen_set_server(BOOT_STATUS_CONNECTED);
        /* Versioning handshake: introduce this firmware (version,
         * protocol, capabilities) before the server pushes anything
         * negotiated. The URL params in ws_client_init give the server
         * the same values even earlier; this message is authoritative. */
        ws_client_send_hello();
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
    char uri[256];
#ifdef CONFIG_YELLO_WS_USE_TLS
    const char *scheme = "wss";
#else
    const char *scheme = "ws";
#endif
    /* fw/proto/caps are the URL form of DeviceHello: they let the
     * server set up the session (esp. whether compression is allowed)
     * before the first frame, instead of only learning it from the
     * hello message after connect. */
    snprintf(uri, sizeof(uri),
             "%s://%s:%u%s?device_id=%02x%02x%02x%02x%02x%02x&fw=%s&proto=%u&caps=%u",
             scheme, config->server_host, config->server_port, CONFIG_YELLO_WS_PATH,
             mac[0], mac[1], mac[2], mac[3], mac[4], mac[5],
             YELLO_FW_VERSION_STRING, (unsigned)YELLO_PROTO_VERSION,
             (unsigned)YELLO_CAPS_CURRENT);

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

bool ws_client_send_hello(void)
{
    if (s_client == NULL || !esp_websocket_client_is_connected(s_client)) {
        return false;
    }
    /* Display geometry: the panel is 240x320 native (see AGENTS.md);
     * these are the frame dimensions the server renders for us. */
    yello_device_hello_t hello = {
        .magic = YELLO_PROTO_MAGIC,
        .version = YELLO_PROTO_VERSION,
        .msg_type = YELLO_MSG_DEVICE_HELLO,
        .reserved0 = 0,
        .capabilities = YELLO_CAPS_CURRENT,
        .fw_major = YELLO_FW_VERSION_MAJOR,
        .fw_minor = YELLO_FW_VERSION_MINOR,
        .fw_patch = YELLO_FW_VERSION_PATCH,
        .max_window_bits = 12, /* tinfl dictionary is 4 KiB (windowBits=12) */
        .display_width = 240,
        .display_height = 320,
        .reserved1 = 0,
    };
    int sent = esp_websocket_client_send_bin(s_client, (const char *)&hello, sizeof(hello), pdMS_TO_TICKS(100));
    if (sent == sizeof(hello)) {
        ESP_LOGI(TAG, "hello sent: fw %s proto %u caps 0x%x",
                 YELLO_FW_VERSION_STRING, (unsigned)YELLO_PROTO_VERSION,
                 (unsigned)YELLO_CAPS_CURRENT);
    }
    return sent == sizeof(hello);
}

bool ws_client_send_resync(uint8_t reason, uint32_t last_good_seq)
{
    if (s_client == NULL || !esp_websocket_client_is_connected(s_client)) {
        return false;
    }
    yello_resync_request_t req = {
        .magic = YELLO_PROTO_MAGIC,
        .version = YELLO_PROTO_VERSION,
        .msg_type = YELLO_MSG_RESYNC_REQUEST,
        .reason = reason,
        .last_good_seq = last_good_seq,
    };
    int sent = esp_websocket_client_send_bin(s_client, (const char *)&req, sizeof(req), pdMS_TO_TICKS(100));
    return sent == sizeof(req);
}

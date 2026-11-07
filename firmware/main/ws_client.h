#pragma once

#include <stdbool.h>
#include <stdint.h>

#include "device_config.h"

/* Connects to the provisioned host/port and the build-configured path/TLS mode.
 * Logs connect/disconnect/error events; relies on the client's own default
 * auto-reconnect behavior. */
void ws_client_init(const yello_runtime_config_t *config);

/* Sends a yello_touch_event_t to the server. Returns false if not connected
 * or the send fails; safe to call regardless of connection state. */
bool ws_client_send_touch(uint8_t touch_type, uint16_t x, uint16_t y);

/* Application heartbeat carrying uptime and current WiFi RSSI. */
bool ws_client_send_status(int16_t wifi_rssi_dbm);

/* Protocol v2: ask the server for a full-frame resync after a dropped
 * or undecodable FramePatch. reason is a yello_resync_reason_t. */
bool ws_client_send_resync(uint8_t reason, uint32_t last_good_seq);

/* Versioning handshake: report firmware version, protocol version and
 * capability bitmask (yello_device_hello_t) to the server. Sent
 * automatically on every (re)connect; the server gates DEFLATE
 * compression on the YELLO_CAP_DEFLATE capability bit. */
bool ws_client_send_hello(void);

/* Sends an OtaStatus message: frames `json` (a NUL-terminated UTF-8
 * JSON body, e.g. {"state":"accepted"}) with the 3-byte protocol
 * header as YELLO_MSG_OTA_STATUS. Used by ota_client; returns false
 * if not connected or the send fails. */
bool ws_client_send_ota_status(const char *json);

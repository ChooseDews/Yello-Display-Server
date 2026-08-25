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

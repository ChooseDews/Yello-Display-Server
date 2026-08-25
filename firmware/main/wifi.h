#pragma once

#include <stdint.h>

#include "device_config.h"

/* Connects to the provisioned SSID/password. Logs connect/disconnect/got-IP
 * events; relies on the WiFi driver's own default reconnect-on-disconnect behavior. */
void wifi_init_sta(const yello_runtime_config_t *config);

/* Current station RSSI in dBm, or -127 while disconnected/unavailable. */
int16_t wifi_get_rssi(void);

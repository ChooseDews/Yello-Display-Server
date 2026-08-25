#pragma once

#include <stdbool.h>
#include <stdint.h>

#define YELLO_SSID_MAX_LEN 32
#define YELLO_WIFI_PASSWORD_MAX_LEN 63
#define YELLO_SERVER_HOST_MAX_LEN 95

typedef struct {
    char ssid[YELLO_SSID_MAX_LEN + 1];
    char password[YELLO_WIFI_PASSWORD_MAX_LEN + 1];
    char server_host[YELLO_SERVER_HOST_MAX_LEN + 1];
    uint16_t server_port;
} yello_runtime_config_t;

/* Initializes the shared NVS partition used by provisioning and the Wi-Fi driver. */
void device_config_init_storage(void);

/* Loads persisted settings when present, otherwise fills Kconfig build defaults.
 * Returns true only when a complete persisted configuration was loaded. */
bool device_config_load(yello_runtime_config_t *config);

bool device_config_is_valid(const yello_runtime_config_t *config);
bool device_config_save(const yello_runtime_config_t *config);

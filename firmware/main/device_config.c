#include <string.h>

#include "esp_err.h"
#include "esp_log.h"
#include "nvs.h"
#include "nvs_flash.h"

#include "device_config.h"

#define CONFIG_NAMESPACE "yello_cfg"
#define CONFIG_VERSION 1

static const char *TAG = "device_config";

static void load_build_defaults(yello_runtime_config_t *config)
{
    memset(config, 0, sizeof(*config));
    strlcpy(config->ssid, CONFIG_YELLO_WIFI_SSID, sizeof(config->ssid));
    strlcpy(config->password, CONFIG_YELLO_WIFI_PASSWORD, sizeof(config->password));
    strlcpy(config->server_host, CONFIG_YELLO_WS_HOST, sizeof(config->server_host));
    config->server_port = CONFIG_YELLO_WS_PORT;
}

void device_config_init_storage(void)
{
    esp_err_t result = nvs_flash_init();
    if (result == ESP_ERR_NVS_NO_FREE_PAGES || result == ESP_ERR_NVS_NEW_VERSION_FOUND) {
        ESP_ERROR_CHECK(nvs_flash_erase());
        result = nvs_flash_init();
    }
    ESP_ERROR_CHECK(result);
}

bool device_config_is_valid(const yello_runtime_config_t *config)
{
    return config != NULL && config->ssid[0] != '\0' &&
           config->server_host[0] != '\0' && config->server_port != 0;
}

bool device_config_load(yello_runtime_config_t *config)
{
    load_build_defaults(config);

    nvs_handle_t handle = 0;
    esp_err_t result = nvs_open(CONFIG_NAMESPACE, NVS_READONLY, &handle);
    if (result == ESP_ERR_NVS_NOT_FOUND) {
        ESP_LOGI(TAG, "no saved device configuration; using build defaults");
        return false;
    }
    if (result != ESP_OK) {
        ESP_LOGW(TAG, "could not open saved configuration: %s", esp_err_to_name(result));
        return false;
    }

    yello_runtime_config_t stored = {0};
    uint8_t version = 0;
    size_t ssid_len = sizeof(stored.ssid);
    size_t password_len = sizeof(stored.password);
    size_t host_len = sizeof(stored.server_host);
    result = nvs_get_u8(handle, "version", &version);
    if (result == ESP_OK && version == CONFIG_VERSION) {
        result = nvs_get_str(handle, "ssid", stored.ssid, &ssid_len);
    }
    if (result == ESP_OK) {
        result = nvs_get_str(handle, "password", stored.password, &password_len);
    }
    if (result == ESP_OK) {
        result = nvs_get_str(handle, "host", stored.server_host, &host_len);
    }
    if (result == ESP_OK) {
        result = nvs_get_u16(handle, "port", &stored.server_port);
    }
    nvs_close(handle);

    if (result != ESP_OK || !device_config_is_valid(&stored)) {
        ESP_LOGW(TAG, "saved configuration is incomplete; using build defaults");
        return false;
    }
    *config = stored;
    ESP_LOGI(TAG, "loaded saved configuration for SSID '%s' and server %s:%u",
             config->ssid, config->server_host, config->server_port);
    return true;
}

bool device_config_save(const yello_runtime_config_t *config)
{
    if (!device_config_is_valid(config)) {
        return false;
    }
    nvs_handle_t handle = 0;
    esp_err_t result = nvs_open(CONFIG_NAMESPACE, NVS_READWRITE, &handle);
    if (result == ESP_OK) result = nvs_set_u8(handle, "version", CONFIG_VERSION);
    if (result == ESP_OK) result = nvs_set_str(handle, "ssid", config->ssid);
    if (result == ESP_OK) result = nvs_set_str(handle, "password", config->password);
    if (result == ESP_OK) result = nvs_set_str(handle, "host", config->server_host);
    if (result == ESP_OK) result = nvs_set_u16(handle, "port", config->server_port);
    if (result == ESP_OK) result = nvs_commit(handle);
    if (handle) nvs_close(handle);
    if (result != ESP_OK) {
        ESP_LOGE(TAG, "failed to save configuration: %s", esp_err_to_name(result));
        return false;
    }
    ESP_LOGI(TAG, "saved device configuration");
    return true;
}

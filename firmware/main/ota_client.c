/* Over-the-air firmware update client.
 *
 * Flow: the server sends an OtaOffer (protocol.h) over the WebSocket.
 * ota_client_handle_offer() validates it and, if accepted, spawns
 * ota_task() so the WebSocket/render path is never blocked by the
 * download. The task fetches http://<server_host>:<server_port>/
 * firmware.bin (the same host/port the WebSocket connects to) with
 * esp_http_client, streams it into the next OTA partition with
 * esp_ota_write while running a SHA-256 over the same bytes, and
 * reports OtaStatus progress at every new 10% boundary.
 *
 * The boot partition flips ONLY after the download is complete and
 * both checks pass: the byte count must equal the offered size and
 * the SHA-256 digest must equal the offered sha256. Any failure
 * before that point aborts the OTA write, reports "failed" with a
 * short error string, and the device simply keeps running the old
 * firmware. After esp_ota_end + esp_ota_set_boot_partition succeed
 * the device reports "rebooting" and restarts into the new image;
 * the new image is only marked valid once it has connected and sent
 * its DeviceHello (see ws_client_send_hello), otherwise the
 * bootloader rolls back to this image on the next reset
 * (CONFIG_BOOTLOADER_APP_ROLLBACK_ENABLE). */

#include <ctype.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <strings.h>

#include "cJSON.h"
#include "esp_err.h"
#include "esp_http_client.h"
#include "esp_log.h"
#include "esp_ota_ops.h"
#include "esp_partition.h"
#include "esp_system.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"
#include "mbedtls/sha256.h"

#include "device_config.h"
#include "ota_client.h"
#include "protocol.h"
#include "ws_client.h"
#include "yello_version.h"

static const char *TAG = "ota_client";

/* Size of each OTA app slot in partitions.csv (ota_0 / ota_1). */
#define OTA_PARTITION_MAX_SIZE 0x1E0000
#define OTA_READ_CHUNK_BYTES 8192
#define OTA_TASK_STACK_BYTES 8192
#define OTA_HTTP_TIMEOUT_MS 10000
#define OTA_VERSION_MAX_LEN 31
#define OTA_SHA256_HEX_LEN 64

typedef struct {
    char version[OTA_VERSION_MAX_LEN + 1];
    uint32_t size;
    char sha256[OTA_SHA256_HEX_LEN + 1]; /* normalized to lowercase */
} ota_offer_t;

static bool s_ota_in_progress = false;

bool ota_client_in_progress(void)
{
    return s_ota_in_progress;
}

/* Builds the OtaStatus JSON body and hands it to ws_client, which
 * frames it as [magic, version, YELLO_MSG_OTA_STATUS] + JSON.
 * `progress` is only included for "downloading", `error` only for
 * "failed" — pass -1 / NULL otherwise. */
static void send_ota_status(const char *state, int progress, const char *error)
{
    cJSON *root = cJSON_CreateObject();
    if (root == NULL) {
        return;
    }
    cJSON_AddStringToObject(root, "state", state);
    if (progress >= 0) {
        cJSON_AddNumberToObject(root, "progress", progress);
    }
    if (error != NULL) {
        cJSON_AddStringToObject(root, "error", error);
    }
    char *json = cJSON_PrintUnformatted(root);
    cJSON_Delete(root);
    if (json == NULL) {
        return;
    }
    if (!ws_client_send_ota_status(json)) {
        ESP_LOGW(TAG, "could not send OTA status '%s' (not connected?)", state);
    }
    cJSON_free(json);
}

static void send_failed(const char *error)
{
    ESP_LOGW(TAG, "OTA failed: %s", error);
    send_ota_status("failed", -1, error);
}

/* Validates that `text` is exactly 64 hex chars and copies it into
 * `out` lowercased, so it can be strcmp()ed against our own
 * lowercase hex digest. */
static bool normalize_sha256(const char *text, char *out, size_t out_size)
{
    if (text == NULL || strlen(text) != OTA_SHA256_HEX_LEN ||
        out_size < OTA_SHA256_HEX_LEN + 1) {
        return false;
    }
    for (size_t i = 0; i < OTA_SHA256_HEX_LEN; i++) {
        if (!isxdigit((unsigned char)text[i])) {
            return false;
        }
        out[i] = (char)tolower((unsigned char)text[i]);
    }
    out[OTA_SHA256_HEX_LEN] = '\0';
    return true;
}

static void ota_task(void *arg)
{
    ota_offer_t *offer = (ota_offer_t *)arg;
    const char *fail_error = NULL;
    char http_error[24] = {0}; /* "http-<code>" */
    esp_http_client_handle_t client = NULL;
    esp_ota_handle_t ota_handle = 0;
    bool ota_begun = false;
    uint8_t *buffer = NULL;
    mbedtls_sha256_context sha_ctx;
    bool sha_started = false;

    /* The firmware image is served by the same server (and port) the
     * WebSocket connects to; reload the config the same way
     * ws_client_init's URL was built (NVS first, build defaults
     * otherwise — device_config_load fills the defaults either way). */
    yello_runtime_config_t config;
    device_config_load(&config);
    if (!device_config_is_valid(&config)) {
        fail_error = "no-config";
        goto fail;
    }
    char url[YELLO_SERVER_HOST_MAX_LEN + 32];
    snprintf(url, sizeof(url), "http://%s:%u/firmware.bin",
             config.server_host, (unsigned)config.server_port);
    ESP_LOGI(TAG, "OTA %s: %u bytes from %s", offer->version,
             (unsigned)offer->size, url);

    const esp_partition_t *partition = esp_ota_get_next_update_partition(NULL);
    if (partition == NULL) {
        fail_error = "no-partition";
        goto fail;
    }
    if (offer->size > partition->size) {
        fail_error = "too-large";
        goto fail;
    }

    esp_http_client_config_t http_config = {
        .url = url,
        .method = HTTP_METHOD_GET,
        .timeout_ms = OTA_HTTP_TIMEOUT_MS,
        .keep_alive_enable = false,
    };
    client = esp_http_client_init(&http_config);
    if (client == NULL) {
        fail_error = "http-init";
        goto fail;
    }
    if (esp_http_client_open(client, 0) != ESP_OK) {
        fail_error = "http-open";
        goto fail;
    }
    (void)esp_http_client_fetch_headers(client);
    int http_status = esp_http_client_get_status_code(client);
    if (http_status != 200) {
        snprintf(http_error, sizeof(http_error), "http-%d", http_status);
        fail_error = http_error;
        goto fail;
    }

    esp_err_t err = esp_ota_begin(partition, OTA_SIZE_UNKNOWN, &ota_handle);
    if (err != ESP_OK) {
        ESP_LOGE(TAG, "esp_ota_begin failed: %s", esp_err_to_name(err));
        fail_error = "ota-begin";
        goto fail;
    }
    ota_begun = true;

    buffer = malloc(OTA_READ_CHUNK_BYTES);
    if (buffer == NULL) {
        fail_error = "no-mem";
        goto fail;
    }
    mbedtls_sha256_init(&sha_ctx);
    if (mbedtls_sha256_starts(&sha_ctx, 0) != 0) {
        fail_error = "sha-init";
        goto fail;
    }
    sha_started = true;

    uint32_t total = 0;
    int next_progress = 10; /* report at each new 10% boundary */
    while (total < offer->size) {
        int want = (int)(offer->size - total);
        if (want > OTA_READ_CHUNK_BYTES) {
            want = OTA_READ_CHUNK_BYTES;
        }
        int read = esp_http_client_read(client, (char *)buffer, want);
        if (read < 0) {
            fail_error = "http-read";
            goto fail;
        }
        if (read == 0) {
            break; /* EOF before `size` bytes: caught by size check below */
        }
        err = esp_ota_write(ota_handle, buffer, (size_t)read);
        if (err != ESP_OK) {
            ESP_LOGE(TAG, "esp_ota_write failed: %s", esp_err_to_name(err));
            fail_error = "ota-write";
            goto fail;
        }
        if (mbedtls_sha256_update(&sha_ctx, buffer, (size_t)read) != 0) {
            fail_error = "sha-update";
            goto fail;
        }
        total += (uint32_t)read;
        int progress = (int)(total * 100u / offer->size);
        if (progress >= next_progress) {
            send_ota_status("downloading", progress, NULL);
            next_progress = (progress / 10 + 1) * 10;
        }
    }

    if (total != offer->size) {
        ESP_LOGW(TAG, "downloaded %u bytes, offer said %u",
                 (unsigned)total, (unsigned)offer->size);
        fail_error = "size-mismatch";
        goto fail;
    }

    send_ota_status("verifying", -1, NULL);
    uint8_t digest[32];
    if (mbedtls_sha256_finish(&sha_ctx, digest) != 0) {
        fail_error = "sha-finish";
        goto fail;
    }
    sha_started = false; /* context finished; freed in cleanup below */
    char digest_hex[OTA_SHA256_HEX_LEN + 1];
    static const char hex_digits[] = "0123456789abcdef";
    for (size_t i = 0; i < sizeof(digest); i++) {
        digest_hex[i * 2] = hex_digits[digest[i] >> 4];
        digest_hex[i * 2 + 1] = hex_digits[digest[i] & 0x0F];
    }
    digest_hex[OTA_SHA256_HEX_LEN] = '\0';
    if (strcmp(digest_hex, offer->sha256) != 0) {
        ESP_LOGW(TAG, "SHA-256 mismatch: got %s, offer said %s",
                 digest_hex, offer->sha256);
        fail_error = "sha256-mismatch";
        goto fail;
    }

    /* Image is complete and verified — only now may it become
     * bootable. esp_ota_end validates the image itself. */
    err = esp_ota_end(ota_handle);
    ota_begun = false; /* esp_ota_end consumes the handle either way */
    if (err != ESP_OK) {
        ESP_LOGE(TAG, "esp_ota_end failed: %s", esp_err_to_name(err));
        fail_error = "ota-end";
        goto fail;
    }
    err = esp_ota_set_boot_partition(partition);
    if (err != ESP_OK) {
        ESP_LOGE(TAG, "esp_ota_set_boot_partition failed: %s", esp_err_to_name(err));
        fail_error = "ota-set-boot";
        goto fail;
    }

    ESP_LOGI(TAG, "OTA verified, boot partition set to %s; rebooting",
             partition->label);
    send_ota_status("rebooting", -1, NULL);
    mbedtls_sha256_free(&sha_ctx);
    free(buffer);
    esp_http_client_close(client);
    esp_http_client_cleanup(client);
    free(offer);
    vTaskDelay(pdMS_TO_TICKS(500));
    esp_restart();
    /* esp_restart() never returns; if it somehow did, fall through
     * to the fail cleanup without reporting (update already set). */
    s_ota_in_progress = false;
    vTaskDelete(NULL);
    return;

fail:
    if (ota_begun) {
        esp_ota_abort(ota_handle);
    }
    if (sha_started) {
        mbedtls_sha256_free(&sha_ctx);
    }
    free(buffer);
    if (client != NULL) {
        esp_http_client_close(client);
        esp_http_client_cleanup(client);
    }
    send_failed(fail_error != NULL ? fail_error : "unknown");
    free(offer);
    s_ota_in_progress = false;
    vTaskDelete(NULL);
}

void ota_client_handle_offer(const uint8_t *json, size_t len)
{
    if (json == NULL || len == 0) {
        send_failed("bad-offer");
        return;
    }

    /* cJSON needs a NUL-terminated string; the WS payload is not. */
    char *text = malloc(len + 1);
    if (text == NULL) {
        send_failed("no-mem");
        return;
    }
    memcpy(text, json, len);
    text[len] = '\0';
    cJSON *root = cJSON_Parse(text);
    free(text);
    if (root == NULL) {
        send_failed("bad-offer");
        return;
    }

    const cJSON *version = cJSON_GetObjectItemCaseSensitive(root, "version");
    const cJSON *size = cJSON_GetObjectItemCaseSensitive(root, "size");
    const cJSON *sha256 = cJSON_GetObjectItemCaseSensitive(root, "sha256");

    ota_offer_t *offer = calloc(1, sizeof(*offer));
    bool valid = offer != NULL &&
                 cJSON_IsString(version) && version->valuestring[0] != '\0' &&
                 strlen(version->valuestring) <= OTA_VERSION_MAX_LEN &&
                 cJSON_IsNumber(size) && size->valuedouble > 0 &&
                 size->valuedouble <= OTA_PARTITION_MAX_SIZE &&
                 (uint32_t)size->valuedouble == size->valuedouble &&
                 cJSON_IsString(sha256) &&
                 normalize_sha256(sha256->valuestring, offer->sha256,
                                  sizeof(offer->sha256));
    if (valid) {
        strlcpy(offer->version, version->valuestring, sizeof(offer->version));
        offer->size = (uint32_t)size->valuedouble;
    }
    cJSON_Delete(root);

    if (!valid) {
        free(offer);
        send_failed("bad-offer");
        return;
    }
    if (strcmp(offer->version, YELLO_FW_VERSION_STRING) == 0) {
        free(offer);
        send_failed("already-current");
        return;
    }
    if (s_ota_in_progress) {
        free(offer);
        send_failed("busy");
        return;
    }

    s_ota_in_progress = true;
    send_ota_status("accepted", -1, NULL);
    if (xTaskCreate(ota_task, "yello_ota", OTA_TASK_STACK_BYTES, offer,
                    5, NULL) != pdPASS) {
        s_ota_in_progress = false;
        free(offer);
        send_failed("no-task");
        return;
    }
    /* The task owns `offer` from here on. */
}

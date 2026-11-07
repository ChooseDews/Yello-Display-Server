#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "cJSON.h"
#include "driver/uart.h"
#include "esp_log.h"
#include "esp_system.h"
#include "freertos/FreeRTOS.h"
#include "freertos/task.h"

#include "device_config.h"
#include "protocol.h"
#include "serial_config.h"
#include "yello_version.h"

static const char *TAG = "serial_config";

#define SERIAL_UART UART_NUM_0
#define SERIAL_BAUD 115200
#define SERIAL_LINE_MAX 512
#define SERIAL_RX_BUF 1024

static void serial_write_str(const char *text)
{
    uart_write_bytes(SERIAL_UART, text, strlen(text));
}

/* Minimal JSON string escaping for the values we echo back (SSID / host
 * could contain quotes or backslashes; control chars are dropped to
 * spaces — they are illegal in an SSID we would accept anyway). */
static void json_escape_into(char *out, size_t out_size, const char *in)
{
    size_t o = 0;
    for (size_t i = 0; in[i] != '\0' && o + 3 < out_size; i++) {
        unsigned char c = (unsigned char)in[i];
        if (c == '"' || c == '\\') {
            out[o++] = '\\';
            out[o++] = (char)c;
        } else if (c < 0x20) {
            out[o++] = ' ';
        } else {
            out[o++] = (char)c;
        }
    }
    out[o] = '\0';
}

static void send_status(void)
{
    yello_runtime_config_t current;
    bool persisted = device_config_load(&current);
    char ssid[YELLO_SSID_MAX_LEN * 2 + 1];
    char host[YELLO_SERVER_HOST_MAX_LEN * 2 + 1];
    json_escape_into(ssid, sizeof(ssid), current.ssid);
    json_escape_into(host, sizeof(host), current.server_host);
    char reply[512];
    snprintf(reply, sizeof(reply),
             "YELLO_STATUS {\"fw\":\"%s\",\"proto\":%d,\"configured\":%s,"
             "\"ssid\":\"%s\",\"host\":\"%s\",\"port\":%u,\"passwordSet\":%s}\n",
             YELLO_FW_VERSION_STRING, YELLO_PROTO_VERSION,
             (persisted && device_config_is_valid(&current)) ? "true" : "false",
             ssid, host, (unsigned)current.server_port,
             current.password[0] != '\0' ? "true" : "false");
    serial_write_str(reply);
}

static void send_error(const char *message)
{
    char reply[192];
    snprintf(reply, sizeof(reply), "YELLO_ERROR {\"error\":\"%s\"}\n", message);
    serial_write_str(reply);
}

static void reboot_soon(void)
{
    /* Let the reply drain out of the UART before resetting, so the web
     * app reliably sees the OK. */
    uart_wait_tx_done(SERIAL_UART, pdMS_TO_TICKS(500));
    vTaskDelay(pdMS_TO_TICKS(250));
    esp_restart();
}

static void handle_set_config(const char *json_text)
{
    cJSON *root = cJSON_Parse(json_text);
    if (root == NULL) {
        send_error("invalid JSON");
        return;
    }
    const cJSON *ssid = cJSON_GetObjectItemCaseSensitive(root, "ssid");
    const cJSON *password = cJSON_GetObjectItemCaseSensitive(root, "password");
    const cJSON *host = cJSON_GetObjectItemCaseSensitive(root, "host");
    const cJSON *port = cJSON_GetObjectItemCaseSensitive(root, "port");

    yello_runtime_config_t next;
    memset(&next, 0, sizeof(next));

    bool ok = cJSON_IsString(ssid) && ssid->valuestring[0] != '\0' &&
              strlen(ssid->valuestring) <= YELLO_SSID_MAX_LEN &&
              cJSON_IsString(host) && host->valuestring[0] != '\0' &&
              strlen(host->valuestring) <= YELLO_SERVER_HOST_MAX_LEN &&
              cJSON_IsNumber(port) && port->valuedouble >= 1 &&
              port->valuedouble <= 65535;
    bool keep_password = false;
    if (ok && (password == NULL || cJSON_IsNull(password))) {
        keep_password = true;
    } else if (ok && cJSON_IsString(password) &&
               strlen(password->valuestring) <= YELLO_WIFI_PASSWORD_MAX_LEN) {
        strlcpy(next.password, password->valuestring, sizeof(next.password));
    } else if (ok) {
        ok = false;
    }

    if (ok) {
        strlcpy(next.ssid, ssid->valuestring, sizeof(next.ssid));
        strlcpy(next.server_host, host->valuestring, sizeof(next.server_host));
        next.server_port = (uint16_t)port->valuedouble;
        if (keep_password) {
            yello_runtime_config_t current;
            device_config_load(&current);
            strlcpy(next.password, current.password, sizeof(next.password));
        }
    }
    cJSON_Delete(root);

    if (!ok || !device_config_is_valid(&next)) {
        send_error("ssid, host and a port 1-65535 are required (password <= 63 chars)");
        return;
    }
    if (!device_config_save(&next)) {
        send_error("could not save configuration");
        return;
    }
    ESP_LOGI(TAG, "configuration written over USB serial for SSID '%s', server %s:%u",
             next.ssid, next.server_host, next.server_port);
    serial_write_str("YELLO_OK {\"saved\":true}\n");
    reboot_soon();
}

static void handle_line(const char *line)
{
    if (strcmp(line, "YELLO_HELLO") == 0 || strcmp(line, "YELLO_GET_CONFIG") == 0) {
        send_status();
    } else if (strncmp(line, "YELLO_SET_CONFIG ", 17) == 0) {
        handle_set_config(line + 17);
    } else if (strcmp(line, "YELLO_CLEAR_CONFIG") == 0) {
        if (device_config_erase()) {
            serial_write_str("YELLO_OK {\"cleared\":true}\n");
            reboot_soon();
        } else {
            send_error("could not clear configuration");
        }
    }
    /* Anything else (boot noise, ESP_LOG echoes, terminal chatter) is
     * not addressed to us: ignore it silently. */
}

static void serial_config_task(void *unused)
{
    (void)unused;
    static char line[SERIAL_LINE_MAX];
    size_t len = 0;
    uint8_t byte = 0;
    ESP_LOGI(TAG, "USB serial configuration listening on UART0 at %d baud", SERIAL_BAUD);
    while (1) {
        int read = uart_read_bytes(SERIAL_UART, &byte, 1, pdMS_TO_TICKS(200));
        if (read <= 0) {
            continue;
        }
        if (byte == '\r') {
            continue;
        }
        if (byte == '\n') {
            line[len] = '\0';
            if (len > 0) {
                handle_line(line);
            }
            len = 0;
            continue;
        }
        if (len + 1 < sizeof(line)) {
            line[len++] = (char)byte;
        } else {
            /* Overlong line: drop it rather than acting on a prefix. */
            len = 0;
        }
    }
}

void serial_config_start(void)
{
    const uart_config_t uart_config = {
        .baud_rate = SERIAL_BAUD,
        .data_bits = UART_DATA_8_BITS,
        .parity = UART_PARITY_DISABLE,
        .stop_bits = UART_STOP_BITS_1,
        .flow_ctrl = UART_HW_FLOWCTRL_DISABLE,
        .source_clk = UART_SCLK_DEFAULT,
    };
    esp_err_t result = uart_driver_install(SERIAL_UART, SERIAL_RX_BUF, 0, 0, NULL, 0);
    if (result == ESP_OK) {
        result = uart_param_config(SERIAL_UART, &uart_config);
    }
    if (result == ESP_OK) {
        result = uart_set_pin(SERIAL_UART, UART_PIN_NO_CHANGE, UART_PIN_NO_CHANGE,
                              UART_PIN_NO_CHANGE, UART_PIN_NO_CHANGE);
    }
    if (result != ESP_OK) {
        ESP_LOGE(TAG, "could not start UART0 driver: %s", esp_err_to_name(result));
        return;
    }
    xTaskCreate(serial_config_task, "yello_serial_cfg", 4096, NULL, 5, NULL);
}

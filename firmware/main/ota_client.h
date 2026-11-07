#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

/* Handles a YELLO_MSG_OTA_OFFER payload: `json` points at the UTF-8
 * JSON body (the bytes after the 3-byte protocol header) and `len` is
 * its length in bytes (not NUL-terminated). Validates the offer and,
 * if accepted, spawns the download task — this call itself never
 * blocks on the network. Rejections (bad offer, already-current,
 * busy) are reported to the server as OtaStatus "failed" from here. */
void ota_client_handle_offer(const uint8_t *json, size_t len);

/* True while an OTA download/verify is running on its own task. */
bool ota_client_in_progress(void);

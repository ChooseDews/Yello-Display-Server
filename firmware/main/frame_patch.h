#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "protocol.h"

/* ESP-IDF glue between the WebSocket client and the streaming patch
 * decoder (patch_stream.c): fragments are copied onto a queue and a
 * dedicated decode task inflates + parses + blits them, so the
 * WebSocket event task never blocks on inflate or SPI. */

/* Create the decode queue + task. Call once at boot, after
 * display_start_render_task(). */
void frame_patch_init(void);

/* Begin a FramePatch message whose 20-byte header has been parsed.
 * Called from the WebSocket event context; the header is copied. */
bool frame_patch_begin(const yello_frame_patch_header_t *hdr);

/* Feed one payload fragment (payload bytes only, header excluded).
 * final_chunk marks the last fragment of the message. The bytes are
 * copied; the caller keeps ownership of its buffer. */
bool frame_patch_feed(const uint8_t *data, size_t len, bool final_chunk);

/* End the current patch message: the decode task validates it and, on
 * any failure, sends a ResyncRequest itself. */
void frame_patch_end(void);

/* frame_seq of the last fully applied patch (maintained by the decode
 * task; a plain word read, safe from other tasks). */
uint32_t frame_patch_last_good_seq(void);

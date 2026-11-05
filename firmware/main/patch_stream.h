#pragma once

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#include "protocol.h"
#include "tinfl.h"

/* Streaming FramePatch decoder (protocol v2) - pure C, no ESP-IDF or
 * FreeRTOS dependencies, so the exact code that runs on the device is
 * host-testable (see firmware/host_test/).
 *
 * Feed it a patch header, then the payload in arbitrary fragments (the
 * WebSocket fragment boundaries), then end(). Compressed payloads are
 * inflated through tinfl into a 4096-byte wrapping dictionary (the server
 * compresses with windowBits=12, so 4 KiB is the whole window) and the
 * decompressed zone-record stream is parsed incrementally: zone pixels
 * are accumulated into a strip buffer and handed to the blit callback
 * one strip of complete rows at a time. A full frame is never held in
 * RAM, and a zone record is never blitted partially.
 */

#define PATCH_DICT_SIZE 4096 /* must equal 1 << YELLO server PATCH_WINDOW_BITS */
#define PATCH_STRIP_MAX_WIDTH 320
#define PATCH_STRIP_ROWS 8
#define PATCH_STRIP_BYTES (PATCH_STRIP_MAX_WIDTH * PATCH_STRIP_ROWS * 2)
/* Sanity ceiling for one bundle: a full 320x240 frame of zone records is
 * 153,600 pixel bytes + 8/record; anything past this is malformed. */
#define PATCH_MAX_UNCOMPRESSED_BYTES (160 * 1024)

typedef enum {
    PATCH_STREAM_OK = 0,
    PATCH_STREAM_SEQ_GAP,
    PATCH_STREAM_INFLATE_FAILED,
    PATCH_STREAM_ADLER_MISMATCH,
    PATCH_STREAM_MALFORMED,
} patch_stream_status_t;

/* Called with one strip of complete rows: pixels_be is w*h*2 bytes of
 * big-endian RGB565 (panel-native), valid only for the call duration. */
typedef void (*patch_blit_fn)(void *ctx, uint16_t x, uint16_t y, uint16_t w, uint16_t h,
                              const uint8_t *pixels_be);

typedef struct {
    patch_blit_fn blit;
    void *blit_ctx;

    /* Patch-level state. */
    bool in_patch;
    patch_stream_status_t sticky; /* first error of the current patch */
    bool compressed;
    bool inflate_done;
    uint32_t frame_seq;
    uint16_t zones_expected;
    uint32_t uncompressed_len;
    uint32_t uncompressed_seen;
    uint16_t logical_w;
    uint16_t logical_h;

    /* Sequence tracking across patches. */
    bool have_seq;
    uint32_t expected_seq;
    uint32_t last_good_seq;

    /* Inflate state (compressed patches only). */
    tinfl_decompressor decomp;
    uint8_t dict[PATCH_DICT_SIZE];
    size_t dict_ofs;

    /* Zone-record parser state. */
    bool in_zone_pixels;
    uint8_t hdr_buf[sizeof(yello_zone_record_header_t)];
    size_t hdr_have;
    uint16_t zones_seen;
    uint16_t zone_x, zone_y, zone_w, zone_h;
    uint32_t zone_pixels_remaining; /* bytes */
    uint16_t zone_rows_done;
    uint16_t strip_rows_target;
    size_t strip_capacity; /* bytes for the current strip */
    size_t strip_fill;     /* bytes accumulated toward strip_capacity */
    uint8_t strip_buf[PATCH_STRIP_BYTES];
} patch_stream_t;

/* The struct is ~18 KiB (inflate tables + dictionary + strip buffer):
 * allocate it statically, never on a task stack. */
void patch_stream_init(patch_stream_t *ps, patch_blit_fn blit, void *blit_ctx);

/* Start a patch. Validates the header and the frame sequence: a
 * non-full patch whose seq is not the expected next one is rejected
 * with PATCH_STREAM_SEQ_GAP (its feeds are then drained harmlessly and
 * end() reports the gap). Full-frame patches always resynchronise the
 * sequence. Returns the begin status; the same status is sticky for
 * the rest of the patch. */
patch_stream_status_t patch_stream_begin(patch_stream_t *ps,
                                         const yello_frame_patch_header_t *hdr,
                                         uint16_t logical_w, uint16_t logical_h);

/* Feed payload bytes. final_chunk must be set on the last fragment of
 * the WebSocket message (it clears tinfl's HAS_MORE_INPUT flag so the
 * stream end + Adler-32 trailer are validated during this patch). */
patch_stream_status_t patch_stream_feed(patch_stream_t *ps, const uint8_t *data, size_t len,
                                        bool final_chunk);

/* Finish a patch: validates totals (decompressed length, zone count,
 * parser at a record boundary, inflate stream done) and, on success,
 * advances the expected sequence. */
patch_stream_status_t patch_stream_end(patch_stream_t *ps);

/* Drop any in-progress patch without touching sequence state (used when
 * the transport aborted mid-patch). The next patch, full or not, is
 * judged on its own sequence number. */
void patch_stream_reset(patch_stream_t *ps);

uint32_t patch_stream_last_good_seq(const patch_stream_t *ps);

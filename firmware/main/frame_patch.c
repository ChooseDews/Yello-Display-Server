#include <stdlib.h>
#include <string.h>

#include "esp_log.h"
#include "freertos/FreeRTOS.h"
#include "freertos/queue.h"
#include "freertos/task.h"

#include "display.h"
#include "frame_patch.h"
#include "patch_stream.h"
#include "ws_client.h"

static const char *TAG = "frame_patch";

/* A photo full frame is ~90 KiB compressed and WebSocket fragments are
 * ~1-4 KiB, so the queue mainly smooths SPI bursts; the decode task
 * (inflate + 40 MHz SPI) is faster than WiFi delivery in practice. */
#define PATCH_QUEUE_DEPTH 64
#define DECODE_TASK_STACK 8192
#define DECODE_TASK_PRIORITY 5

typedef enum {
    PATCH_MSG_BEGIN,
    PATCH_MSG_FEED,
    PATCH_MSG_END,
} patch_msg_kind_t;

typedef struct {
    patch_msg_kind_t kind;
    uint32_t patch_id;
    yello_frame_patch_header_t hdr; /* BEGIN only */
    uint8_t *data;                  /* FEED only, owned by the queue slot */
    size_t len;                     /* FEED only */
    bool final_chunk;               /* FEED only */
} patch_msg_t;

static QueueHandle_t s_queue = NULL;
static patch_stream_t s_stream; /* ~18 KiB: static, never on a stack */
static uint32_t s_patch_id = 0;
static bool s_drop_patch = false;
static volatile bool s_abort_requested = false;
static volatile uint32_t s_last_good_seq = 0;
static uint32_t s_task_patch_id = 0;

static void blit_strip(void *ctx, uint16_t x, uint16_t y, uint16_t w, uint16_t h,
                       const uint8_t *pixels_be)
{
    (void)ctx;
    display_draw_bitmap(x, y, w, h, (const uint16_t *)pixels_be);
}

static uint8_t resync_reason_for(patch_stream_status_t status)
{
    switch (status) {
    case PATCH_STREAM_SEQ_GAP:
        return YELLO_RESYNC_SEQ_GAP;
    case PATCH_STREAM_INFLATE_FAILED:
        return YELLO_RESYNC_INFLATE_FAILED;
    case PATCH_STREAM_ADLER_MISMATCH:
        return YELLO_RESYNC_ADLER_MISMATCH;
    case PATCH_STREAM_MALFORMED:
    default:
        return YELLO_RESYNC_MALFORMED;
    }
}

static void decode_task(void *arg)
{
    (void)arg;
    patch_msg_t msg;
    while (1) {
        if (xQueueReceive(s_queue, &msg, pdMS_TO_TICKS(200)) != pdTRUE) {
            if (s_abort_requested) {
                /* The WS side dropped fragments of the in-flight patch;
                 * reset so its remainder can never be applied, and let
                 * the already-sent resync bring a clean full frame. */
                patch_stream_reset(&s_stream);
                s_abort_requested = false;
            }
            continue;
        }
        switch (msg.kind) {
        case PATCH_MSG_BEGIN:
            s_task_patch_id = msg.patch_id;
            patch_stream_begin(&s_stream, &msg.hdr, display_get_width(), display_get_height());
            break;
        case PATCH_MSG_FEED:
            if (msg.patch_id == s_task_patch_id) {
                patch_stream_feed(&s_stream, msg.data, msg.len, msg.final_chunk);
            }
            free(msg.data);
            break;
        case PATCH_MSG_END:
            if (msg.patch_id == s_task_patch_id) {
                patch_stream_status_t status = patch_stream_end(&s_stream);
                s_last_good_seq = patch_stream_last_good_seq(&s_stream);
                if (status != PATCH_STREAM_OK) {
                    ESP_LOGW(TAG, "patch failed status=%d, requesting resync", (int)status);
                    ws_client_send_resync(resync_reason_for(status), s_last_good_seq);
                }
            }
            break;
        }
    }
}

void frame_patch_init(void)
{
    patch_stream_init(&s_stream, blit_strip, NULL);
    s_queue = xQueueCreate(PATCH_QUEUE_DEPTH, sizeof(patch_msg_t));
    xTaskCreate(decode_task, "patch_decode", DECODE_TASK_STACK, NULL, DECODE_TASK_PRIORITY, NULL);
}

static bool enqueue(patch_msg_t *msg)
{
    if (xQueueSend(s_queue, msg, pdMS_TO_TICKS(100)) != pdTRUE) {
        return false;
    }
    return true;
}

bool frame_patch_begin(const yello_frame_patch_header_t *hdr)
{
    s_patch_id++;
    s_drop_patch = false;
    patch_msg_t msg = { .kind = PATCH_MSG_BEGIN, .patch_id = s_patch_id, .hdr = *hdr };
    if (!enqueue(&msg)) {
        s_drop_patch = true;
        s_abort_requested = true;
        return false;
    }
    return true;
}

bool frame_patch_feed(const uint8_t *data, size_t len, bool final_chunk)
{
    if (s_drop_patch) {
        return false;
    }
    if (len == 0) {
        /* A zero-length final fragment still matters: it tells tinfl
         * the stream is complete. Queue it without an allocation. */
        patch_msg_t msg = {
            .kind = PATCH_MSG_FEED, .patch_id = s_patch_id, .data = NULL, .len = 0, .final_chunk = final_chunk
        };
        if (!enqueue(&msg)) {
            s_drop_patch = true;
            s_abort_requested = true;
            return false;
        }
        return true;
    }
    uint8_t *copy = malloc(len);
    if (!copy) {
        ESP_LOGW(TAG, "oom copying patch fragment (%u bytes)", (unsigned)len);
        s_drop_patch = true;
        s_abort_requested = true;
        return false;
    }
    memcpy(copy, data, len);
    patch_msg_t msg = {
        .kind = PATCH_MSG_FEED, .patch_id = s_patch_id, .data = copy, .len = len, .final_chunk = final_chunk
    };
    if (!enqueue(&msg)) {
        free(copy);
        s_drop_patch = true;
        s_abort_requested = true;
        return false;
    }
    return true;
}

void frame_patch_end(void)
{
    if (s_drop_patch) {
        /* Fragments were lost on our side of the queue; the decode task
         * is resetting via s_abort_requested. Ask the server for a full
         * frame now instead of waiting for the next patch to gap. */
        ws_client_send_resync(YELLO_RESYNC_QUEUE_OVERFLOW, s_last_good_seq);
        return;
    }
    patch_msg_t msg = { .kind = PATCH_MSG_END, .patch_id = s_patch_id };
    if (!enqueue(&msg)) {
        ws_client_send_resync(YELLO_RESYNC_QUEUE_OVERFLOW, s_last_good_seq);
    }
}

uint32_t frame_patch_last_good_seq(void)
{
    return s_last_good_seq;
}

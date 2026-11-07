#pragma once

#include <stdbool.h>
#include <stdint.h>

/* Wire protocol v2 shared with server-rs/src/protocol/mod.rs — keep both in sync by hand.
 *
 * v2 (breaking, Nov 2026): per-zone ZoneUpdate messages are gone. Each frame
 * push is ONE FramePatch WebSocket binary message bundling every dirty zone
 * record, DEFLATE-compressed as a single zlib stream (windowBits = 12, so
 * the inflate dictionary here is only 4 KiB) with a raw fallback when
 * compression does not pay. A dropped/corrupt patch is answered with a
 * ResyncRequest and the server re-sends a full frame. */

#define YELLO_PROTO_MAGIC   0x59 /* 'Y' */
#define YELLO_PROTO_VERSION 2

typedef enum {
    YELLO_MSG_TOUCH_EVENT = 2,    /* device -> server */
    YELLO_MSG_DEVICE_CONFIG = 3,  /* server -> device */
    YELLO_MSG_DEVICE_STATUS = 4,  /* device -> server heartbeat */
    YELLO_MSG_DEVICE_COMMAND = 5, /* server -> device */
    YELLO_MSG_FRAME_PATCH = 6,    /* server -> device */
    YELLO_MSG_RESYNC_REQUEST = 7, /* device -> server */
    YELLO_MSG_DEVICE_HELLO = 8,   /* device -> server, first message after connect */
    YELLO_MSG_OTA_OFFER = 9,      /* server -> device, JSON body (see below) */
    YELLO_MSG_OTA_STATUS = 10,    /* device -> server, JSON body (see below) */
} yello_msg_type_t;

/* OTA messages (types 9/10) are small control messages like
 * DeviceConfig/DeviceCommand — they reassemble into the 4 KiB WS
 * buffer — but instead of a packed struct their payload is UTF-8 JSON
 * right after the shared 3-byte header [magic, version, msg_type]
 * (no length prefix, no trailing NUL; the WS message length bounds it):
 *   OtaOffer  (server -> device):
 *     {"version":"2.2.0","size":1234567,"sha256":"<64 hex chars>"}
 *   OtaStatus (device -> server):
 *     {"state":"accepted|downloading|verifying|rebooting|failed",
 *      "progress":<int percent, only for downloading>,
 *      "error":"<short string, only for failed>"} */

/* Device capability bits (yello_device_hello_t.capabilities, also sent
 * as the `caps` WebSocket URL param). The server must never send a
 * feature whose bit is clear — in particular it sends DEFLATE-compressed
 * FramePatch payloads only to devices with YELLO_CAP_DEFLATE, and raw
 * bundles to everyone else. */
#define YELLO_CAP_RAW_BUNDLE  (1u << 0) /* raw FramePatch bundle (every v2 device) */
#define YELLO_CAP_DEFLATE     (1u << 1) /* DEFLATE/zlib payloads (tinfl, 4 KiB window) */
#define YELLO_CAP_FRAME_PATCH (1u << 2) /* FramePatch message (vs removed v1 ZoneUpdate) */
#define YELLO_CAP_OTA         (1u << 3) /* OTA firmware update (OtaOffer/OtaStatus) */
#define YELLO_CAPS_CURRENT    (YELLO_CAP_RAW_BUNDLE | YELLO_CAP_DEFLATE | YELLO_CAP_FRAME_PATCH | YELLO_CAP_OTA)

typedef enum {
    YELLO_ORIENTATION_PORTRAIT = 0,
    YELLO_ORIENTATION_LANDSCAPE = 1,
} yello_orientation_t;

typedef enum {
    YELLO_COMMAND_RESTART = 1,
} yello_device_command_type_t;

typedef enum {
    YELLO_TOUCH_DOWN = 0,
    YELLO_TOUCH_MOVE = 1,
    YELLO_TOUCH_UP = 2,
} yello_touch_type_t;

/* FramePatch flags (header byte 3). */
#define YELLO_PATCH_FLAG_COMPRESSED 0x01
#define YELLO_PATCH_FLAG_FULL_FRAME 0x02

/* Why the device asked for a full-frame resync (ResyncRequest reason). */
typedef enum {
    YELLO_RESYNC_SEQ_GAP = 1,
    YELLO_RESYNC_INFLATE_FAILED = 2,
    YELLO_RESYNC_ADLER_MISMATCH = 3,
    YELLO_RESYNC_MALFORMED = 4,
    YELLO_RESYNC_QUEUE_OVERFLOW = 5,
} yello_resync_reason_t;

#pragma pack(push, 1)

/* server -> device, 20 bytes, followed by exactly payload_len bytes.
 * If flags & YELLO_PATCH_FLAG_COMPRESSED, the payload is one zlib stream
 * (windowBits=12) of the uncompressed bundle; otherwise the payload IS the
 * bundle (payload_len == uncompressed_len). The bundle is a sequence of
 * zone_count zone records (see yello_zone_record_header_t). */
typedef struct {
    uint8_t magic;            /* YELLO_PROTO_MAGIC */
    uint8_t version;          /* YELLO_PROTO_VERSION */
    uint8_t msg_type;         /* YELLO_MSG_FRAME_PATCH */
    uint8_t flags;            /* YELLO_PATCH_FLAG_* */
    uint32_t frame_seq;       /* increments per patch sent to this device */
    uint16_t zone_count;      /* zone records in the bundle */
    uint16_t reserved;        /* 0 */
    uint32_t uncompressed_len; /* bundle length before compression */
    uint32_t payload_len;     /* bytes following this header */
} yello_frame_patch_header_t;

/* One zone record inside the (uncompressed) bundle: 8-byte header followed
 * by w*h*2 bytes of row-major BIG-endian RGB565 pixels (ILI9341-native, so
 * strips blit without a per-pixel swap). Header fields are little-endian. */
typedef struct {
    uint16_t x;
    uint16_t y;
    uint16_t w;
    uint16_t h;
} yello_zone_record_header_t;

/* device -> server, 12 bytes, no payload */
typedef struct {
    uint8_t magic;        /* YELLO_PROTO_MAGIC */
    uint8_t version;      /* YELLO_PROTO_VERSION */
    uint8_t msg_type;     /* YELLO_MSG_TOUCH_EVENT */
    uint8_t touch_type;   /* yello_touch_type_t */
    uint16_t x;           /* calibrated screen x, 0..239 */
    uint16_t y;           /* calibrated screen y, 0..319 */
    uint32_t timestamp_ms; /* ms since boot */
} yello_touch_event_t;

/* server -> device, 8 bytes */
typedef struct {
    uint8_t magic;
    uint8_t version;
    uint8_t msg_type;
    uint8_t orientation;
    uint16_t width;
    uint16_t height;
} yello_device_config_t;

/* device -> server heartbeat, 12 bytes */
typedef struct {
    uint8_t magic;
    uint8_t version;
    uint8_t msg_type;
    uint8_t flags;
    uint32_t uptime_ms;
    int16_t wifi_rssi_dbm;
    uint16_t reserved;
} yello_device_status_t;

/* server -> device, 4 bytes */
typedef struct {
    uint8_t magic;
    uint8_t version;
    uint8_t msg_type;
    uint8_t command;
} yello_device_command_t;

/* device -> server, 8 bytes */
typedef struct {
    uint8_t magic;
    uint8_t version;
    uint8_t msg_type;  /* YELLO_MSG_RESYNC_REQUEST */
    uint8_t reason;    /* yello_resync_reason_t */
    uint32_t last_good_seq; /* frame_seq of the last fully applied patch */
} yello_resync_request_t;

/* device -> server, 20 bytes — the versioning handshake, sent once
 * right after the WebSocket connects (see ws_client_send_hello). The
 * server records firmware_version / proto version / capabilities on
 * the session and gates compression on YELLO_CAP_DEFLATE. The version
 * byte here is REPORTED by the device, not validated by the server,
 * so a future protocol version can still introduce itself. */
typedef struct {
    uint8_t magic;            /* YELLO_PROTO_MAGIC */
    uint8_t version;          /* protocol version this firmware speaks */
    uint8_t msg_type;         /* YELLO_MSG_DEVICE_HELLO */
    uint8_t reserved0;
    uint32_t capabilities;    /* YELLO_CAP_* bitmask, little-endian */
    uint8_t fw_major;
    uint8_t fw_minor;
    uint8_t fw_patch;
    uint8_t max_window_bits;  /* 12 when YELLO_CAP_DEFLATE is set, else 0 */
    uint16_t display_width;
    uint16_t display_height;
    uint32_t reserved1;
} yello_device_hello_t;

#pragma pack(pop)

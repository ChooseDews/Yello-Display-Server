#pragma once

#include <stdint.h>

/* Wire protocol shared with server/protocol.py — keep both in sync by hand. */

#define YELLO_PROTO_MAGIC   0x59 /* 'Y' */
#define YELLO_PROTO_VERSION 1

typedef enum {
    YELLO_MSG_ZONE_UPDATE = 1, /* server -> device */
    YELLO_MSG_TOUCH_EVENT = 2, /* device -> server */
    YELLO_MSG_DEVICE_CONFIG = 3, /* server -> device */
    YELLO_MSG_DEVICE_STATUS = 4, /* device -> server heartbeat */
    YELLO_MSG_DEVICE_COMMAND = 5, /* server -> device */
} yello_msg_type_t;

typedef enum {
    YELLO_ORIENTATION_PORTRAIT = 0,
    YELLO_ORIENTATION_LANDSCAPE = 1,
} yello_orientation_t;

typedef enum {
    YELLO_COMMAND_RESTART = 1,
} yello_device_command_type_t;

typedef enum {
    YELLO_FMT_RAW_RGB565 = 0, /* implemented */
    YELLO_FMT_RLE_RGB565 = 1, /* reserved, not implemented yet */
} yello_pixel_format_t;

typedef enum {
    YELLO_TOUCH_DOWN = 0,
    YELLO_TOUCH_MOVE = 1,
    YELLO_TOUCH_UP = 2,
} yello_touch_type_t;

#pragma pack(push, 1)

/* server -> device, 16 bytes, followed by exactly payload_len bytes of pixel data */
typedef struct {
    uint8_t magic;       /* YELLO_PROTO_MAGIC */
    uint8_t version;     /* YELLO_PROTO_VERSION */
    uint8_t msg_type;    /* YELLO_MSG_ZONE_UPDATE */
    uint8_t format;      /* yello_pixel_format_t */
    uint16_t x;          /* zone top-left x, 0..239 */
    uint16_t y;          /* zone top-left y, 0..319 */
    uint16_t w;          /* zone width in px */
    uint16_t h;          /* zone height in px */
    uint32_t payload_len; /* bytes following this header; for RAW_RGB565 == w*h*2 */
} yello_zone_header_t;
/* RAW_RGB565 payload pixels are row-major, BIG-endian (high byte first) —
 * the ILI9341's native SPI byte order, so the device can blit the payload
 * without a per-pixel swap. Header fields remain little-endian. */

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

#pragma pack(pop)

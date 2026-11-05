#pragma once

/* Firmware version — the single source of truth for what the device
 * reports in DeviceHello and the WebSocket URL (`fw=` param). Bump it
 * on every firmware release; keep firmware/CMakeLists.txt PROJECT_VER
 * in step so the ESP-IDF app descriptor agrees.
 *
 * 2.0.0 = protocol v2: FramePatch + streaming DEFLATE (tinfl) + the
 *         DeviceHello capability handshake.
 * 1.x   = the pre-v2 firmware (per-zone raw ZoneUpdate, no hello —
 *         the server only knows it as "unknown / no hello received"). */
#define YELLO_FW_VERSION_MAJOR 2
#define YELLO_FW_VERSION_MINOR 0
#define YELLO_FW_VERSION_PATCH 0
#define YELLO_FW_VERSION_STRING "2.0.0"

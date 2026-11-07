#pragma once

/* Firmware version — the single source of truth for what the device
 * reports in DeviceHello and the WebSocket URL (`fw=` param). Bump it
 * on every firmware release; keep firmware/CMakeLists.txt PROJECT_VER
 * in step so the ESP-IDF app descriptor agrees.
 *
 * 2.2.0 = 2.1.0 + over-the-air (OTA) firmware updates: the server can
 *         offer a new image (OtaOffer); the device downloads it over
 *         HTTP, verifies size + SHA-256, and only then flips the boot
 *         partition — with bootloader rollback if the new image never
 *         connects and says hello (see ota_client.c).
 * 2.1.0 = 2.0.0 + USB-serial configuration: the Studio web app can
 *         write the Wi-Fi credentials and server address to NVS over
 *         the flashing cable (see serial_config.h).
 * 2.0.0 = protocol v2: FramePatch + streaming DEFLATE (tinfl) + the
 *         DeviceHello capability handshake.
 * 1.x   = the pre-v2 firmware (per-zone raw ZoneUpdate, no hello —
 *         the server only knows it as "unknown / no hello received"). */
#define YELLO_FW_VERSION_MAJOR 2
#define YELLO_FW_VERSION_MINOR 2
#define YELLO_FW_VERSION_PATCH 0
#define YELLO_FW_VERSION_STRING "2.2.0"

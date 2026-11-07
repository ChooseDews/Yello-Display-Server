#pragma once

/* USB-serial device configuration (UART0 — the same CH340 port used for
 * flashing, 115200 8N1). Lets the Studio web app write the device's
 * Wi-Fi credentials and WebSocket server address into NVS right after
 * flashing, with no touchscreen provisioning and no rebuild.
 *
 * Line-based protocol; lines not starting with "YELLO_" are ignored, so
 * ESP_LOG output sharing the UART is harmless. Commands:
 *
 *   YELLO_HELLO\n
 *   YELLO_GET_CONFIG\n
 *     -> YELLO_STATUS {"fw":"2.1.0","proto":2,"configured":true,
 *        "ssid":"...","host":"...","port":8885,"passwordSet":true}
 *        (the password itself is never sent back)
 *
 *   YELLO_SET_CONFIG {"ssid":"...","password":"...","host":"...","port":8885}\n
 *     -> YELLO_OK {"saved":true}   then the device reboots into the new
 *        configuration. "password" may be omitted (or null) to keep the
 *        currently saved one; the other fields are required.
 *     -> YELLO_ERROR {"error":"..."}  on invalid input; nothing is saved.
 *
 *   YELLO_CLEAR_CONFIG\n
 *     -> YELLO_OK {"cleared":true}  then the device reboots into
 *        first-boot setup (build defaults / touchscreen provisioning).
 *
 * The listener task runs for the whole life of the device, so settings
 * can also be changed later over the same cable. Anyone with physical
 * USB access can reconfigure the device — by design for a home display;
 * there is no other authentication on this port.
 */

/* Starts the serial configuration task. Call once after
 * device_config_init_storage(): the task reads the live configuration
 * from NVS itself when answering, so it needs no config pointer. */
void serial_config_start(void);

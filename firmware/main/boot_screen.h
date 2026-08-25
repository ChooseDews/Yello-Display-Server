#pragma once

typedef enum {
    BOOT_STATUS_WAITING,
    BOOT_STATUS_CONNECTING,
    BOOT_STATUS_CONNECTED,
    BOOT_STATUS_RETRYING,
    BOOT_STATUS_ERROR,
} boot_status_t;

/* Shows connection progress until the first server-rendered frame arrives. */
void boot_screen_init(void);
void boot_screen_set_wifi(boot_status_t status);
void boot_screen_set_server(boot_status_t status);
void boot_screen_finish(void);

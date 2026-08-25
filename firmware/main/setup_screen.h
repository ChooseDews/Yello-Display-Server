#pragma once

#include <stdbool.h>

#include "device_config.h"

/* Shows a short boot hint and returns true when the display is held for setup. */
bool setup_screen_boot_requested(void);

/* Runs the blocking portrait provisioning UI. Returns true after settings were
 * validated and saved, or false when an already-configured device cancels. */
bool setup_screen_run(yello_runtime_config_t *config, bool allow_cancel);

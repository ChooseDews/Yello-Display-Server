#pragma once

#include <stdbool.h>
#include <stdint.h>

void touch_init(void);
void touch_set_orientation(bool landscape);

/* Poll the touch controller once. Returns true if currently pressed,
 * with x/y in display pixel coordinates (0..DISPLAY_WIDTH-1 / DISPLAY_HEIGHT-1). */
bool touch_poll(uint16_t *x, uint16_t *y);

# ESP_Yello_Device

## Device

Working with an ESP32 "Yellow" dev board — a 2.8" TFT touch display module
(silkscreened "TP4408-2.8") with an orange PCB, mounted on a stand in front of a webcam. The board has no software screenshot mechanism, so its screen is captured by photographing it via webcam.

The device is connected via USB to the host machine

## Screenshot tool

`capture_frame.py` is a `uv`-managed Python tool that grabs a webcam frame of
the device and processes it into a clean screenshot-like image (deskews,
crops tight to the board, corrects for the camera's mounted orientation and
mirroring).

Run it with no arguments to capture the current device screen:

```sh
uv run capture_frame.py
```

This prints the absolute path to the resulting image, which is always
`device_snapshot.jpg` in the current directory — read that file to see what's
on the device's screen right now. Each run also archives a timestamped copy
under `history/snapshots/device_<YYYYMMDD_HHMMSS>.jpg`

The auto-deskew can rotate/mirror ambiguously; when orientation matters, use
`uv run capture_frame.py --raw -o <path>` for the unprocessed camera frame.

## Hardware (confirmed by bring-up)

ESP32-2432S028R-family "Cheap Yellow Display" clone: ESP32-WROOM-32 (no
PSRAM), ILI9341 240x320 SPI TFT, XPT2046 resistive touch, CH340 serial at
`/dev/ttyUSB0` (access via `sg dialout -c "..."` — user isn't in dialout).

- TFT (HSPI/SPI2): MOSI=13 MISO=12 SCLK=14 CS=15 DC=2 RST=none BL=21 (active-high)
- Touch (VSPI/SPI3): MOSI=32 MISO=39 SCLK=25 CS=33 IRQ=36
- RGB LED (active-low): R=4 G=16 B=17
- Panel quirks: needs `swap_xy=true` + `mirror(false,false)` (native GRAM is
  landscape; without swap_xy, writes past ~col 156 are silently clipped) and
  `LCD_RGB_ELEMENT_ORDER_RGB` (BGR swaps red/blue). This USB-C + Micro-USB
  panel revision also needs the custom positive/negative gamma tables in
  `firmware/main/display.c`; generic ILI9341 gamma makes photos milky and
  severely shifts mid-tones even though primary color bars look correct.
  Touch needs no swap/mirror; raw driver coords already match display coords.

## Firmware (`firmware/`)

ESP-IDF v5.5.5 project (toolchain at `~/esp/esp-idf`, activate with
`. ~/esp/esp-idf/export.sh`). Build/flash from `firmware/`:

```sh
idf.py build
sg dialout -c "idf.py -p /dev/ttyUSB0 flash"
```

WiFi credentials + WS server host/port live in `sdkconfig` (gitignored) under
"Yello Device Configuration". To watch serial output without a TTY:
`sg dialout -c "stty -F /dev/ttyUSB0 115200 raw -echo && timeout 10 cat /dev/ttyUSB0"`.

The firmware is a remote display: it connects to a WebSocket server, blits
received zone updates (see `firmware/main/protocol.h`, mirrored byte-for-byte
in `server-rs/src/protocol/mod.rs` — change together), and sends touch events
back. Zone pixel payloads are big-endian RGB565 (ILI9341-native) so the device
blits without swapping; headers are little-endian. Keep each zone message
≤ ~480 px area (~1KB) to fit the esp_websocket_client RX buffer.

## Server (`server-rs/`)

Rust (tokio + hyper) WebSocket/HTTP server that owns layout, rendering, and
the Yello Studio browser designer; streams dirty-region zone updates and
handles touch events. Build/test:

```sh
cargo build --release --manifest-path server-rs/Cargo.toml
cargo test --manifest-path server-rs/Cargo.toml
```

Run with `cargo run --release --manifest-path server-rs/Cargo.toml`
(web studio on `8080`, device gateway on `8765`). Config: `studio.json` /
`secrets.json` next to the working directory; override paths/ports with
`YELLO_STUDIO_PATH`, `YELLO_SECRETS_PATH`, `YELLO_STATIC_DIR`,
`YELLO_WEB_PORT`, `YELLO_DEVICE_WS_PORT`.

# ESP Yello Device

Remote-rendered interfaces for an ESP32-2432S028R-compatible 2.8-inch touch
display. The ESP32 receives RGB565 regions over WebSocket and reports touch
events; the server owns layout, rendering, dynamic data, actions, and
the browser-based screen designer. The server is implemented in Rust
(`server-rs/`, tokio + hyper + image crate); the original Python reference
implementation was retired — its benchmark comparison against the Rust server
is preserved in [docs/server_benchmark.md](docs/server_benchmark.md), and
module details are in [rust_build.md](rust_build.md).

## Run the designer

```sh
cargo run --release --manifest-path server-rs/Cargo.toml
```

Open <http://127.0.0.1:8080/>. The device gateway listens on port `8765` on
all interfaces so the ESP32 can connect over the LAN. The editor itself binds
to localhost by default. Designs are persisted in `studio.json`, credentials
in `secrets.json` (both created next to the working directory; override with
`YELLO_STUDIO_PATH` / `YELLO_SECRETS_PATH`, or `YELLO_WEB_PORT` /
`YELLO_DEVICE_WS_PORT` for the ports).

### Run with Docker

```sh
docker compose up --build -d
```

The compose stack publishes the designer on `8080`, the ESP32 WebSocket gateway
on `8765`, and keeps designs and dashboard secrets in the `yello-data` volume
(`/data/studio.json`, `/data/secrets.json`). Runtime data and credentials are
excluded from both Git and image build contexts.

The designer supports text, clocks, external text, buttons, screen buttons, images, color
blocks, sandboxed scripts, Home Assistant sensor values, and Home Assistant entity toggles. Blocks
can be selected, dragged, resized, reordered, hidden, locked,
duplicated, aligned or deleted from a right-click menu, and edited with undo/redo.
Designs can be portrait or landscape, and Live view sends browser touches through
the same action path as the physical display. Connected screens appear in the header;
each can be assigned its own reusable design. **Save & apply** persists the
selected design in `studio.json` atomically and refreshes every connected screen
assigned to it.

A **Screen button** looks and behaves like a normal touch button but opens
another saved design on the device that pressed it. Add a matching screen
button on the destination design to build back, home, and multi-page menu
flows. Navigation changes only the device's current screen; its assigned home
design remains unchanged and is restored when it reconnects.

The designer's **Code** mode exposes the complete normalized layout as JSON.
JSON edits are schema-validated by the server before they are applied to the
visual design, participate in undo/redo, and remain unsaved until **Save & apply**.
Destructive confirmations and design-name entry use Yello Studio dialogs rather
than native browser prompts.

Configure Home Assistant from the dashboard Settings page to enable entity
browsing, state polling, and touch-driven toggles. The token never leaves the
server.

Connected screen cards show rolling rendered FPS, changed pixels per second,
wire bandwidth, 10-second heartbeat age, WiFi RSSI, and device uptime. Online
devices can be restarted from the Devices page. Full-frame transfer uses a tuned 2 ms per-zone cadence
instead of the original 10 ms, and high-volume per-zone firmware logs are
disabled at the normal info level.

## Benchmarks

`bench/run_bench.py` drives fake ESP32 devices (speaking the real binary wire
protocol) against the server and samples server CPU/RSS from `/proc`:

```sh
uv run bench/run_bench.py
```

Raw JSON and server logs land in `bench/results/` (gitignored). The historic
Python-vs-Rust methodology, results, and the concurrency fixes that came out of
them are documented in [docs/server_benchmark.md](docs/server_benchmark.md).

## Test

```sh
cargo test --manifest-path server-rs/Cargo.toml
```

## Build and flash firmware

```sh
cd firmware
. /home/john/esp/esp-idf/export.sh
idf.py build
sg dialout -c "idf.py -p /dev/ttyUSB0 flash"
```

### First-boot device setup

When no usable Wi-Fi configuration exists, the display opens **Device Setup**
before networking starts. Tap a field and use the local keyboard to enter:

- Wi-Fi SSID
- Wi-Fi password
- server hostname or LAN IP
- WebSocket port (normally `8765`)

Tap **Save** to validate and persist the settings in ESP32 NVS. The password is
masked, is never logged, and no longer needs to be compiled into firmware. To
edit an existing configuration, reboot the device and hold the touchscreen for
about one second while **HOLD SCREEN FOR SETUP** is displayed. **Cancel** keeps
the previously saved configuration.

Build-time Kconfig values remain migration/factory defaults. A fresh repository
build intentionally has an empty SSID, so a newly flashed device enters setup.

## Repository layout

| Path | Contents |
|---|---|
| `firmware/` | ESP-IDF v5.5.5 project for the ESP32 display device |
| `server-rs/` | Rust server (tokio/hyper) + Yello Studio web UI + tests |
| `bench/` | Benchmark harness and generated results |
| `docs/` | Long-form write-ups (benchmark analysis, design demos) |
| `capture_frame.py` | Webcam-based screenshot tool for the physical device |

## Screenshot tool

`capture_frame.py` photographs the display through a webcam and processes the
frame into a clean screenshot (deskew, crop, orientation fix):

```sh
uv run capture_frame.py          # writes device_snapshot.jpg
uv run capture_frame.py --raw -o raw.jpg   # unprocessed camera frame
```

Each run archives a timestamped copy under `history/snapshots/`.

## Continuous integration

The GitHub Actions workflow runs server tests, syntax-checks the browser app,
builds downloadable ESP32 flash artifacts, and builds multi-architecture
container images. Pushes to `main` and `v*` tags publish the image to
`ghcr.io/<owner>/<repository>`. It also builds a portable Windows bundle
(`yello-server.exe` + static assets) and a Linux AppImage; on `v*` tags both are
attached to the GitHub release.

See [AGENTS.md](AGENTS.md) for confirmed pin mappings, panel quirks, capture
instructions, and the wire-protocol constraints.

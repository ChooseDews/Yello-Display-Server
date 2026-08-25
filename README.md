# ESP Yello Device

Remote-rendered interfaces for an ESP32-2432S028R-compatible 2.8-inch touch
display. The ESP32 receives RGB565 regions over WebSocket and reports touch
events; the Python server owns layout, rendering, dynamic data, actions, and
the browser-based screen designer.

## Run the designer

```sh
cd server
uv run server.py
```

Open <http://127.0.0.1:8080/>. The device gateway listens on port `8765` on
all interfaces so the ESP32 can connect over the LAN. The editor itself binds
to localhost by default.

### Run with Docker

```sh
docker compose up --build -d
```

The compose stack publishes the designer on `8080`, the ESP32 WebSocket gateway
on `8765`, and keeps designs and dashboard secrets in the `yello-data` volume.
Copy `.env.example` to `.env` only when optional environment configuration is
needed. Runtime data and credentials are excluded from both Git and image build
contexts.

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

Configure Home Assistant from the dashboard Settings page (or through
`server/.env`) to enable entity browsing, state polling, and touch-driven
toggles. The token never leaves the Python server.

Connected screen cards show rolling rendered FPS, changed pixels per second,
wire bandwidth, 10-second heartbeat age, WiFi RSSI, and device uptime. Online
devices can be restarted from the Devices page. Full-frame transfer uses a tuned 2 ms per-zone cadence
instead of the original 10 ms, and high-volume per-zone firmware logs are
disabled at the normal info level.

## Test

```sh
UV_CACHE_DIR=/tmp/yello-uv-cache uv run --project server \
  python -m unittest discover -s server/tests -v
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

## Continuous integration

The GitHub Actions workflow runs server tests, syntax-checks the browser app,
builds downloadable ESP32 flash artifacts, and builds multi-architecture
container images. Pushes to `main` and `v*` tags publish the image to
`ghcr.io/<owner>/<repository>`.

See [AGENTS.md](AGENTS.md) for confirmed pin mappings, panel quirks, capture
instructions, and the wire-protocol constraints.

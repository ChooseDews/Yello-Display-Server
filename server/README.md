# Yello Server

The server exposes:

- `http://127.0.0.1:8080/` — browser designer
- `ws://0.0.0.0:8765/` — ESP32 display/touch gateway
- `/api/studio` — known screens, connection state, and available designs
- `/api/layout?design=<id>` — normalized design layout and save/apply endpoint
- `/api/layout/validate?design=<id>` — validate and normalize code-editor JSON without saving
- `/api/preview?design=<id>` — canonical portrait or landscape Pillow render
- `/api/live-touch?design=<id>` — browser live-view touch injection
- `/api/designs` — create a design by cloning an existing one
- `/api/screens/<id>` — rename a screen or assign its design
- `/api/screens/<id>/restart` — request an online device restart
- `/api/catalog` — normalized defaults for each block type
- `/api/integrations/home-assistant` — sanitized entity catalog and integration status
- `/api/status` — connected and known screen counts

Layouts use versioned schema `1`. Legacy prototype layouts are migrated during
load. Every block has a consistent top-left `frame` containing `x`, `y`, `w`,
and `h`; type-specific values live under `style` and `props`. Each design may
use portrait (`240×320`) or landscape (`320×240`) orientation.

Each firmware client identifies itself with its Wi-Fi station MAC in the
WebSocket query string. The server keeps independent render/diff/touch state
for every connection, and stores screen-to-design assignments in `studio.json`.
Saving a design refreshes only the connected screens assigned to that design.
The `design-link` block is exposed as a **Screen button** in the editor. Touching
one changes only that connection's active design, while preserving the device's
persistent home-design assignment. Browser Live view follows the same target.
Older firmware without an explicit ID remains supported with a remote-address
fallback and is migrated to the MAC identity after it reconnects with updated
firmware.

## Configuration

| Variable | Default | Purpose |
| --- | --- | --- |
| `YELLO_WEB_HOST` | `127.0.0.1` | Designer/API bind address |
| `YELLO_WEB_PORT` | `8080` | Designer/API port |
| `YELLO_DEVICE_HOST` | `0.0.0.0` | Device gateway bind address |
| `YELLO_DEVICE_WS_PORT` | `8765` | Device gateway port |
| `YELLO_DATA_DIR` | server directory | Persistent `studio.json`, legacy layout, and private settings directory |
| `YELLO_ALLOWED_HTTP_HOSTS` | empty | Comma-separated private hosts explicitly allowed for sources, images, or actions |
| `HA_URL` / `YELLO_HA_URL` | empty | Home Assistant base URL, such as `http://homeassistant.local:8123` |
| `HA_TOKEN` / `YELLO_HA_TOKEN` | empty | Home Assistant long-lived access token; never sent to the browser or device |

Outbound HTTP is restricted to HTTP(S), bounded by response size, and rejects
private, loopback, link-local, and reserved destinations unless their exact
hostname or IP appears in `YELLO_ALLOWED_HTTP_HOSTS`.

`layout.json` is retained as the one-time migration source for installations
created before multi-screen support. Once `studio.json` exists, it is the
authoritative persistent state.

The container sets `YELLO_DATA_DIR=/data`; mount that path or use the provided
named Compose volume to preserve designs across upgrades.

## Home Assistant

Configure the URL and long-lived token on the dashboard Settings page, or use
the environment variables above. Dashboard credentials are saved separately
in gitignored `secrets.json` with owner-only permissions. The **HA sensor** block displays an entity state or scalar attribute,
with optional unit and number formatting. The **HA toggle** block displays the
current on/off state and calls Home Assistant's `homeassistant.toggle` service
when touched. For safety, toggle blocks accept only `light`, `switch`,
`input_boolean`, and `fan` entities. Entity IDs are selected in the designer;
credentials remain only on the server. The entity catalog endpoint exposes
only names, IDs, states, and units used by the editor.

## Stream telemetry

Each connected screen in `/api/studio` includes rolling 10-second stream
statistics: rendered FPS, updated pixels per second, wire kilobits per second,
zone messages per second, and the most recent server push duration. Firmware
also sends a 10-second application heartbeat with uptime and WiFi RSSI. The
designer displays the primary rates directly on each screen card.

Zone delivery uses a tuned 2 ms cadence. Sending without any cadence can fill
the ESP32 TCP receive window faster than its WebSocket/render tasks drain it;
the original 10 ms cadence was stable but added 1.6 seconds to every full
240×320 transfer. The tuned value retains flow stability with one fifth of
that pacing overhead.

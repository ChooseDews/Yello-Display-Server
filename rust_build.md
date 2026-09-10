# Rust Server Rewrite - Complete

## Status Overview
The Rust implementation (`server-rs`) is fully functional, passes all unit and integration test suites, and provides full feature parity with the reference Python server.

## Implemented Modules

### 1. Protocol (`src/protocol/mod.rs`)
- Binary wire protocol byte-for-byte compatible with ESP32 firmware (`firmware/main/protocol.h`)
- Message packing/unpacking: `ZoneUpdate` (with big-endian RGB565 pixel payload and 16-byte header), `TouchEvent`, `DeviceConfig`, `DeviceStatus`, `DeviceCommand`
- RGB888 to RGB565 big-endian conversion utility

### 2. Layout Model & Validation (`src/layout_model/mod.rs`)
- Full schema version 1 validation and normalization
- Backward-compatible migration from legacy prototype formats
- Support for all element types: `text`, `clock`, `external-text`, `color-block`, `button`, `design-link`, `image`, `ha-state`, `ha-toggle`, `script`
- Bounded coordinates, dimensions, colors, identifiers, and cross-reference validation

### 3. Frame Diffing (`src/frame_diff/mod.rs`)
- Dirty tile region calculation with `max_area = 480 px`, tile-based change detection, row-tile coalescing, and full-frame refresh threshold

### 4. Renderer (`src/renderer/mod.rs`)
- Direct canvas rendering to RGB images using `image` crate (configured with `png,jpeg` features)
- Built-in scaleable 8x8 bitmap font renderer supporting multi-line text, wrapping, alignment (left/center/right, top/middle/bottom), and font sizing
- Formatted clock rendering, external text substitution, Home Assistant state & unit formatting
- Interactive elements: rounded rectangles, border rendering, pressed state color feedback, touch dot visualization

### 5. Script Runtime (`src/script_runtime/mod.rs`)
- Sandboxed AST interpreter for bounded Python-like expressions and statements
- Drawing commands: `clear()`, `rect()`, `line()`, `circle()`, `label()`, `text()`
- Built-in helpers: `ha()`, `now()`, `number()`, `clamp()`, `sin()`, `cos()`, `tan()`, `sqrt()`, `abs()`, `min()`, `max()`, `round()`, `pi`
- Formatted f-strings, conditional blocks, bounded range loops, AST node & operation limits, security restriction rejections

### 6. Safe HTTP & Home Assistant (`src/safe_http/mod.rs`, `src/home_assistant/mod.rs`)
- Bounded outbound requests with private IP / localhost restrictions and host allowlists
- Home Assistant REST API client for polling entity states and executing toggle services

### 7. Server & WebSocket Gateway (`src/server/mod.rs`, `src/main.rs`)
- Hyper-based HTTP REST API and static file server serving Yello Studio UI
- Full REST endpoints: `/api/studio`, `/api/layout`, `/api/layout/validate`, `/api/preview`, `/api/catalog`, `/api/designs`, `/api/screens`, `/api/status`, etc.
- WebSocket server managing ESP32 device connections: initial full push, dirty zone streaming, touch hit testing, button action triggering, screen navigation, and status telemetry

## Build & Test

```sh
# Build binary
cargo build --manifest-path server-rs/Cargo.toml

# Run all test suites
cargo test --manifest-path server-rs/Cargo.toml
```

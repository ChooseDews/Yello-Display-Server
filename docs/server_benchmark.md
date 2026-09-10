# Yello Server Benchmark: Python vs Rust

Comparison of the reference Python server (`server/server.py`, asyncio + aiohttp +
websockets + numpy/Pillow) against the Rust rewrite (`server-rs/`, tokio + hyper +
tokio-tungstenite + image crate), measured with fake ESP32 devices speaking the
real binary wire protocol (`protocol.py` / `src/protocol/mod.rs`).

- Harness: `bench/run_bench.py` — spawns each server on isolated ports with a
  bench-local copy of `server/studio.json` (external data sources stripped),
  connects WebSocket devices that consume zone updates and send touches/status,
  and samples server CPU time and RSS from `/proc` every 250 ms.
- Raw results: `bench/results/{python,rust}_results.json`, server logs in
  `bench/results/*_server.log`.
- Date: 2026-09-08.

## Environment

| | |
|---|---|
| CPU | AMD Ryzen 7 3800X (8C/16T), Fedora 44, kernel 6.x |
| RAM | 64 GiB |
| Layout | `default` design, portrait 240x320, clock element (1 s render tick) |
| Frame size | 76,800 px = 153,600 B payload + 160 zone headers ≈ 156,160 B |
| Zone pacing | 2 ms/zone (both implementations), max 480 px/zone |
| Devices | Fake clients on localhost — no Wi-Fi / ESP32 limits involved |

## Results

### 1. Connect burst — 20 devices connect simultaneously

| Metric | Python | Rust |
|---|---|---|
| Wall time until all connected | **0.08 s** | 9.31 s |
| Time to first frame, p50 | **0.01 s** | 4.65 s |
| Time to first frame, max | **0.02 s** | 9.30 s |
| Server CPU during burst | 9.4 % | 0.75 % |
| Server RSS max | 75.5 MB | 26.0 MB |
| Connect errors | 0 | 0 |

Both deliver the same bytes (~3.24 MB total), but Rust serializes initial full
pushes: `push_frame` holds the global device-map write lock for the entire
zone-send loop (including the 2 ms/zone pacing sleeps), so 20 full frames go out
one at a time (~320 ms each). Python pushes concurrently with a per-session
lock. With 20 devices Rust takes ~100x longer to paint every screen.

### 2. Steady state — 10 devices on the clock design, 20 s

| Metric | Python | Rust |
|---|---|---|
| Server CPU | 3.8 % | **0.6 %** |
| Server RSS avg | 70.2 MB | **21.8 MB** |
| Zone messages/s | 34.8 | 30.8 |
| Wire KB/s | 31.8 | 28.5 |

Rust idles at ~1/6 of Python's CPU and ~1/3 of its memory for identical work.
(Python's 70 MB baseline is dominated by interpreter + numpy + Pillow imports.)

### 3. Touch storm — 5 devices x 20 touches/s for 10 s (1,000 touches)

| Metric | Python | Rust |
|---|---|---|
| Server CPU | 59.2 % | **4.9 %** |
| Touch → echo-push latency p50 | **10.9 ms** | 27.2 ms |
| Touch → echo-push latency p95 | **18.8 ms** | 211.9 ms |
| Echo zone msgs back to devices | 4,623 | 3,214 |
| Wire KB/s | 350.9 | 247.2 |

Rust uses ~12x less CPU, but its p95 latency is ~11x worse: the same global
write lock makes a touch-triggered push wait behind the 1 s render-tick pushes
of every other device. Python's latency tail is tight and predictable.

### 4. Full-frame resync — 3 devices reconnect 8 times (full 240x320 push each)

| Metric | Python | Rust |
|---|---|---|
| Full frame delivered, p50 | **0.78 s** | 1.88 s |
| Full frame delivered, max | 1.32 s | 2.28 s |
| Bytes per round (3 devices) | 469.6 KB | 475.3 KB |
| Server CPU | 13.7 % | **0.5 %** |
| Timeouts | 0 | 0 |

Identical wire output per round; Rust again trades ~2.4x delivery latency for
~27x lower CPU.

### 5. Layout save — POST /api/layout, 8x

| Metric | Python | Rust |
|---|---|---|
| POST latency p50 | 378.8 ms | **2.1 ms** |
| POST latency max | 390.5 ms | 30.1 ms |

Not directly comparable: Python awaits the full (`full=true`) push inside the
request; Rust answers immediately and defers a dirty (`full=false`) push. This
is also a **parity gap**: Python re-pushes the full frame after every save,
Rust does not (and Rust's save handler ignores the `?design=` parameter and
always writes the default design).

### 6. Preview rendering — GET /api/designs/default/preview (PNG), 5 s

| Metric | Python | Rust |
|---|---|---|
| Throughput | 357.5 req/s | **1,848.5 req/s (5.2x)** |
| Latency p50 | 2.7 ms | **0.5 ms** |
| Latency p95 | 3.2 ms | **0.6 ms** |
| Server CPU | 96.6 % (saturated) | 83.3 % |
| PNG size avg | 9,236 B | 6,093 B |

The pure-render path is where Rust's advantage is largest — 5x throughput at
less CPU. PNG sizes differ because the two font rasterizers are not identical;
the servers render visibly different pixels for the same layout.

## Summary

Values are post-fix for Rust (original numbers in parentheses where the fix
changed them — see the section below).

| | Python | Rust | Winner |
|---|---|---|---|
| Idle memory (RSS) | ~70 MB | ~21–34 MB | Rust (2–3x) |
| Steady CPU, 10 devices | 3.8 % | 0.6–0.8 % | Rust (5x) |
| Burst fan-out (20 screens) | 0.08 s | 0.01 s (was 9.3 s) | Rust |
| Touch echo p50 / p95 | 11 / 19 ms | 1.8 / 3.8 ms (was 27 / 212 ms) | Rust (6x) |
| Full-frame delivery p50 | 0.78 s | 0.90 s (was 1.88 s) | ~tie (pacing-bound) |
| Save API latency | 379 ms | 2.1 ms | Rust (does less work) |
| Render throughput | 296–358 req/s | 1,849 req/s | Rust (5.2x) |
| Parity with firmware protocol | reference | matches, but save ≠ full push | Python |

## Interpretation

- **For the real deployment (a handful of ESP32 screens on Wi-Fi)** both
  servers are overwhelmingly fast enough — the device's LCD and radio are
  orders of magnitude slower than either server. The Rust rewrite's value here
  is memory footprint and near-idle CPU.
- **Rust's original weakness was architectural, not computational**: a single
  `RwLock` over the device map was held across entire paced zone-send loops,
  serializing all pushes (connect bursts, cross-device render ticks, touch
  echoes). This has been fixed — see the next section.
- **Python's main weakness is per-render cost**: it saturates a core at ~360
  previews/s and burns ~59 % CPU under the touch storm, but its concurrency
  model (per-session locks, concurrent `gather`) gives it good fan-out and
  latency behavior.
- Parity gaps found during benchmarking (worth fixing before promoting the
  Rust server): save does not trigger a full re-push, save ignores
  `?design=`, and rendered fonts/PNG output differ from the reference.

## Fix applied: per-device push locks + lock-free send path

Root cause of the Rust latency tail: `push_frame` acquired the global
`devices` write lock and held it for the *entire* push — including the 2 ms/zone
pacing sleeps (a full frame = 160 zones = 320 ms) — so every push on any device
blocked every other device.

The fix (in `server-rs/src/server/mod.rs`):

1. Added `push_lock: Arc<Mutex<()>>` to `DeviceSession` (mirrors Python's
   per-session `asyncio.Lock`). It is taken first, then held only by that
   device's push.
2. `push_frame` restructured into short lock phases: **snapshot** (clone
   sender/design id/last frame/pressed/dots under brief map locks) →
   **lock-free** render + RGB565 encode + paced send → **write-back** (store
   last frame + stream stats under a brief map lock). The global map is never
   held across a send.
3. `push_design` and the 1 s render-tick loop now spawn per-device pushes
   concurrently instead of looping sequentially.

Re-benchmark after the fix (same harness, same machine):

| Metric | Rust before | Rust after | Python (reference) |
|---|---|---|---|
| 20-device burst: wall time | 9.31 s | **0.01 s** | 0.08 s |
| Time to first frame p50 | 4.65 s | **0.00 s** | 0.01 s |
| Touch echo p50 | 27.2 ms | **1.8 ms** | 11.1 ms |
| Touch echo p95 | 211.9 ms | **3.8 ms** | 19.9 ms |
| Touch storm server CPU | 4.9 % | 9.1 % | 59.3 % |
| Full frame delivered p50 (3 dev) | 1.88 s | 0.90 s | 0.78 s |
| Steady CPU (10 devices) | 0.6 % | 0.8 % | 3.6 % |
| Steady RSS | 21.8 MB | 33.9 MB | 74.9 MB |
| Preview render throughput | 1,849 req/s | 1,848 req/s | 296 req/s |

After the fix Rust is now the best on essentially every axis: lowest touch
latency (6x better than Python), instant fan-out, ~4x lower CPU than Python
under load, and still ~2x less memory (RSS grew from ~21 MB to ~34 MB because
concurrent pushes hold per-connection encode buffers — an acceptable trade).
Full-frame delivery is now dominated by the shared 2 ms/zone pacing constant
(~320 ms/frame), putting it within ~15 % of Python.

### Second round of optimizations

1. **`last_frame: Arc<RgbImage>`** — the previous frame was cloned (230 KB)
   on every push for the dirty-region diff; it is now shared.
2. **Single-pass RGB565 encode** — replaced per-pixel `get_pixel()` +
   `to_be_bytes()` with one pass over the raw RGB buffer, writing big-endian
   bytes directly.
3. **Clock-aware render tick** (Python `has_clock` parity) — designs with a
   visible clock/script element tick every 1 s, static designs only every
   30 s, instead of always 1 s.
4. **External-value parity fix** — new external-text values now trigger an
   immediate `push_design` (Python behavior) instead of waiting for the next
   render tick.

Net effect (final run): touch echo p50 **1.0 ms** / p95 **2.4 ms**, burst
fan-out 0.01 s, steady CPU ~0.9 %, RSS ~32–34 MB, save API 0.9 ms, preview
~1,650 req/s. Full-frame delivery ~0.9–1.7 s across runs (pacing-bound).
During the encode rewrite an off-by-one (`row_start` relative to region
instead of chunk) was caught by the WebSocket integration test — test suites
kept passing after the fix.

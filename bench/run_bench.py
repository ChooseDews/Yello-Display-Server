#!/usr/bin/env python3
"""Benchmark harness: fake ESP32 devices against the Rust yello server.

Runs identical scenario phases against the server while sampling the
server process CPU time and RSS from /proc, and collecting device-side
wire-protocol statistics. Writes raw JSON to bench/results/.
"""

from __future__ import annotations

import asyncio
import json
import os
import shutil
import signal
import statistics
import struct
import subprocess
import sys
import time
from pathlib import Path

import aiohttp
import websockets

ROOT = Path(__file__).resolve().parent.parent
BENCH = ROOT / "bench"
RESULTS = BENCH / "results"
DATA = BENCH / "data"

ZONE_HEADER = struct.Struct("<BBBBHHHHI")
STATUS_MSG = struct.pack("<BBBBIhH", 0x59, 1, 4, 0, 0, -50, 0)
TOUCH_MSG = struct.Struct("<BBBBHHI")

CLK_TCK = os.sysconf("SC_CLK_TCK")
PORTS = {"rust": (8280, 8281)}


class ProcSampler:
    def __init__(self, pid: int):
        self.pid = pid
        self.samples: list[tuple[float, int, int]] = []
        self._stop = False
        self._task: asyncio.Task | None = None

    async def run(self):
        last = self._read()
        if last is None:
            return
        self.samples.append(last)
        while not self._stop:
            await asyncio.sleep(0.25)
            cur = self._read()
            if cur is None:
                break
            self.samples.append(cur)

    def _read(self) -> tuple[float, int, int] | None:
        try:
            stat = Path(f"/proc/{self.pid}/stat").read_bytes()
            fields = stat.rsplit(b")", 1)[1].split()
            utime, stime = int(fields[11]), int(fields[12])
            rss_kb = 0
            for line in Path(f"/proc/{self.pid}/status").read_text().splitlines():
                if line.startswith("VmRSS:"):
                    rss_kb = int(line.split()[1])
                    break
            return (time.monotonic(), utime + stime, rss_kb)
        except (OSError, IndexError):
            return None

    def phase_stats(self, start_idx: int) -> dict:
        if len(self.samples) - start_idx < 2:
            return {"cpu_percent": 0.0, "rss_avg_kb": 0, "rss_max_kb": 0, "duration_s": 0.0}
        window = self.samples[start_idx:]
        (t0, c0, _), (t1, c1, _) = window[0], window[-1]
        dur = max(t1 - t0, 1e-6)
        cpu = (c1 - c0) / CLK_TCK / dur * 100.0
        return {
            "cpu_percent": round(cpu, 2),
            "rss_avg_kb": round(statistics.mean(s[2] for s in window)),
            "rss_max_kb": max(s[2] for s in window),
            "duration_s": round(dur, 2),
        }

    def stop(self):
        self._stop = True


class Device:
    """Fake ESP32 device: consumes zone updates, sends status + touches."""

    def __init__(self, device_id: str, ws_port: int):
        self.device_id = device_id
        self.ws_port = ws_port
        self.ws = None
        self.zone_msgs = 0
        self.wire_bytes = 0
        self.pixels = 0
        self.touch_latencies: list[float] = []
        self._pending_touches: list[float] = []
        self._first_frame_event: asyncio.Event | None = None
        self._recv_task = None
        self._status_task = None
        self._running = False
        self._frame_gap = 0.05
        self._last_msg_t = 0.0
        self.frames = 0
        self._quiet_event: asyncio.Event | None = None
        self._quiet_since = 0.0

    async def connect(self):
        url = f"ws://127.0.0.1:{self.ws_port}/?id={self.device_id}&device_id={self.device_id}"
        self.ws = await websockets.connect(url, max_size=2**22, ping_interval=None)
        self._running = True
        self._recv_task = asyncio.create_task(self._recv_loop())
        self._status_task = asyncio.create_task(self._status_loop())

    async def wait_first_frame(self, timeout: float) -> float:
        ev = asyncio.Event()
        self._first_frame_event = ev
        if self.zone_msgs > 0:
            return 0.0
        t0 = time.perf_counter()
        try:
            await asyncio.wait_for(ev.wait(), timeout)
        except asyncio.TimeoutError:
            return -1.0
        return time.perf_counter() - t0

    async def _recv_loop(self):
        try:
            async for msg in self.ws:
                if not isinstance(msg, bytes) or len(msg) < 16:
                    continue
                now = time.perf_counter()
                if msg[2] == 1:  # ZONE_UPDATE
                    self.zone_msgs += 1
                    self.wire_bytes += len(msg)
                    self.pixels += struct.unpack_from("<H", msg, 8)[0] * struct.unpack_from("<H", msg, 10)[0]
                    if self._first_frame_event is not None:
                        self._first_frame_event.set()
                        self._first_frame_event = None
                    if now - self._last_msg_t > self._frame_gap:
                        self.frames += 1
                    self._last_msg_t = now
                    if self._pending_touches:
                        t_sent = self._pending_touches.pop(0)
                        self.touch_latencies.append(now - t_sent)
                    self._quiet_since = now
                    if self._quiet_event is not None:
                        self._quiet_event.set()
        except (websockets.exceptions.ConnectionClosed, asyncio.CancelledError):
            pass

    async def _status_loop(self):
        try:
            while self._running:
                await asyncio.sleep(1.0)
                await self.ws.send(STATUS_MSG)
        except (websockets.exceptions.ConnectionClosed, asyncio.CancelledError):
            pass

    async def send_touches(self, rate_per_s: float, duration_s: float, width=240, height=320, seed=0):
        import random

        rng = random.Random(seed)
        interval = 1.0 / rate_per_s
        end = time.perf_counter() + duration_s
        next_t = time.perf_counter()
        while time.perf_counter() < end:
            await asyncio.sleep(max(0.0, next_t - time.perf_counter()))
            next_t += interval
            x, y = rng.randrange(width), rng.randrange(height)
            self._pending_touches.append(time.perf_counter())
            msg = TOUCH_MSG.pack(0x59, 1, 2, 0, x, y, 0)
            await self.ws.send(msg)

    def reset_counters(self):
        self.zone_msgs = 0
        self.wire_bytes = 0
        self.pixels = 0
        self.frames = 0
        self.touch_latencies = []
        self._pending_touches = []

    async def wait_quiet(self, quiet_s: float, timeout: float):
        end = time.perf_counter() + timeout
        while time.perf_counter() < end:
            last = self._quiet_since
            await asyncio.sleep(quiet_s)
            if self._quiet_since == last and self._quiet_since > 0:
                return True
        return False

    async def wait_bytes(self, before: int, target: int, timeout: float) -> float:
        """Wait until wire_bytes grows by `target`, then 0.4 s of silence. Returns elapsed."""
        t0 = time.perf_counter()
        end = t0 + timeout
        while time.perf_counter() < end:
            if self.wire_bytes - before >= target:
                break
            await asyncio.sleep(0.02)
        else:
            return -1.0
        last = self.wire_bytes
        while time.perf_counter() < end:
            await asyncio.sleep(0.4)
            if self.wire_bytes == last:
                return time.perf_counter() - t0
            last = self.wire_bytes
        return -1.0

    async def close(self):
        self._running = False
        for t in (self._recv_task, self._status_task):
            if t:
                t.cancel()
        try:
            await self.ws.close()
        except Exception:
            pass


def spawn_server(impl: str) -> tuple[subprocess.Popen, dict]:
    web_port, ws_port = PORTS[impl]
    data_dir = DATA / impl
    env = dict(os.environ)
    env.update({
        "YELLO_WEB_HOST": "127.0.0.1",
        "YELLO_DEVICE_HOST": "127.0.0.1",
        "YELLO_WEB_PORT": str(web_port),
        "YELLO_DEVICE_WS_PORT": str(ws_port),
        "YELLO_STUDIO_PATH": str(data_dir / "studio.json"),
        "YELLO_SECRETS_PATH": str(data_dir / "secrets.json"),
        "YELLO_STATIC_DIR": str(ROOT / "server-rs/static"),
    })
    log = open(RESULTS / f"{impl}_server.log", "w")
    cmd = [str(ROOT / "server-rs/target/release/yello-server")]
    proc = subprocess.Popen(cmd, env=env, stdout=log, stderr=subprocess.STDOUT, cwd=str(ROOT))
    return proc, {"web_port": web_port, "ws_port": ws_port, "log": log}


async def wait_ready(web_port: int, timeout: float = 15.0):
    deadline = time.perf_counter() + timeout
    while time.perf_counter() < deadline:
        try:
            async with aiohttp.ClientSession() as s:
                async with s.get(f"http://127.0.0.1:{web_port}/api/status", timeout=aiohttp.ClientTimeout(total=1)) as r:
                    if r.status == 200:
                        return
        except Exception:
            pass
        await asyncio.sleep(0.2)
    raise RuntimeError("server did not become ready")


def phase(device_stats: list[Device], name: str, sampler: ProcSampler, start_idx: int, extra: dict) -> dict:
    agg_zones = sum(d.zone_msgs for d in device_stats)
    agg_bytes = sum(d.wire_bytes for d in device_stats)
    agg_pixels = sum(d.pixels for d in device_stats)
    lat = [x for d in device_stats for x in d.touch_latencies]
    result = {
        "phase": name,
        "server": sampler.phase_stats(start_idx),
        "devices": len(device_stats),
        "zone_messages": agg_zones,
        "wire_bytes": agg_bytes,
        "pixels": agg_pixels,
        "zone_msgs_per_s": round(agg_zones / max(sampler.phase_stats(start_idx)["duration_s"], 1e-6), 1),
        "kbytes_per_s": round(agg_bytes / 1024 / max(sampler.phase_stats(start_idx)["duration_s"], 1e-6), 1),
        "touch_latency_ms_p50": round(statistics.median(lat) * 1000, 1) if lat else None,
        "touch_latency_ms_p95": round(sorted(lat)[int(len(lat) * 0.95)] * 1000, 1) if len(lat) >= 20 else None,
        "touch_count": len(lat),
        **extra,
    }
    return result


async def run_suite(impl: str) -> dict:
    web_port, ws_port = PORTS[impl]
    proc, meta = spawn_server(impl)
    results = {"impl": impl, "web_port": web_port, "phases": []}
    try:
        await wait_ready(web_port)
        await asyncio.sleep(2.0)  # settle background loops
        sampler = ProcSampler(proc.pid)
        samp_task = asyncio.create_task(sampler.run())
        await asyncio.sleep(0.3)

        # ---- Phase 1: connect burst (20 devices) ----
        idx = len(sampler.samples)
        t0 = time.perf_counter()
        devices = [Device(f"bench-{i:02d}", ws_port) for i in range(20)]
        connect = await asyncio.gather(*(d.connect() for d in devices), return_exceptions=True)
        ttff = await asyncio.gather(*(d.wait_first_frame(20.0) for d in devices))
        connect_wall = time.perf_counter() - t0
        ok_ttff = [t for t in ttff if t >= 0]
        await asyncio.sleep(3.0)
        results["phases"].append(phase(devices, "connect_burst_20", sampler, idx, {
            "connect_wall_s": round(connect_wall, 2),
            "time_to_first_frame_s_p50": round(statistics.median(ok_ttff), 2) if ok_ttff else None,
            "time_to_first_frame_s_max": round(max(ok_ttff), 2) if ok_ttff else None,
            "connect_errors": sum(1 for c in connect if isinstance(c, Exception)) + sum(1 for t in ttff if t < 0),
        }))

        # ---- Phase 2: steady clock stream (10 devices, 20 s) ----
        for d in devices[10:]:
            await d.close()
        await asyncio.sleep(0.5)
        for d in devices:
            d.reset_counters()
        idx = len(sampler.samples)
        await asyncio.sleep(20.0)
        results["phases"].append(phase(devices[:10], "steady_clock_stream", sampler, idx, {}))

        # ---- Phase 3: touch storm (5 devices x 20 touches/s, 10 s) ----
        for d in devices:
            d.reset_counters()
        idx = len(sampler.samples)
        t0 = time.perf_counter()
        await asyncio.gather(*(devices[i].send_touches(20.0, 10.0, seed=i) for i in range(5)))
        storm_wall = time.perf_counter() - t0
        await asyncio.sleep(1.0)
        results["phases"].append(phase(devices[:5], "touch_storm_5x20hz", sampler, idx, {
            "touches_sent": 5 * 200,
            "storm_wall_s": round(storm_wall, 2),
        }))

        # ---- Phase 4: full-frame resync via reconnect (3 devices, 8 rounds) ----
        # Full push on connect is the only identical full-frame trigger
        # (POST /api/layout pushes full=false, so saves are benchmarked
        # separately below).
        keep = devices[:3]
        for d in devices:
            d.reset_counters()
        for d in devices[3:]:
            await d.close()
        await asyncio.sleep(0.5)
        full_frame_wire = 240 * 320 * 2 + 160 * 16  # one full frame per device
        deliver_lat = []
        full_bytes = []
        total_bytes = 0
        total_zones = 0
        idx = len(sampler.samples)
        for i in range(8):
            for d in keep:
                d.reset_counters()
            t0 = time.perf_counter()
            await asyncio.gather(*(d.close() for d in keep))
            await asyncio.gather(*(d.connect() for d in keep))
            delivered = await asyncio.gather(*(d.wait_bytes(0, full_frame_wire, 20.0) for d in keep))
            deliver_lat.append(max(delivered))
            full_bytes.append(sum(d.wire_bytes for d in keep))
            total_bytes += sum(d.wire_bytes for d in keep)
            total_zones += sum(d.zone_msgs for d in keep)
            await asyncio.sleep(0.3)
        results["phases"].append({
            "phase": "full_frame_resync_8x",
            "server": sampler.phase_stats(idx),
            "devices": 3,
            "full_frame_delivered_ms_p50": round(statistics.median(deliver_lat) * 1000, 1),
            "full_frame_delivered_ms_max": round(max(deliver_lat) * 1000, 1),
            "full_frame_bytes_avg": round(statistics.mean(full_bytes)),
            "total_wire_bytes": total_bytes,
            "total_zone_messages": total_zones,
            "deliver_timeouts": sum(1 for t in deliver_lat if t < 0),
        })

        # ---- Phase 4b: layout save latency (8 POSTs, no devices attached) ----
        async with aiohttp.ClientSession() as http:
            async with http.get(f"http://127.0.0.1:{web_port}/api/layout?design=default") as r:
                layout_doc = await r.json()
        post_lat = []
        statuses = []
        idx = len(sampler.samples)
        async with aiohttp.ClientSession() as http:
            for _ in range(8):
                t0 = time.perf_counter()
                async with http.post(
                    f"http://127.0.0.1:{web_port}/api/layout?design=default", json=layout_doc["layout"]
                ) as r:
                    statuses.append(r.status)
                    await r.read()
                post_lat.append(time.perf_counter() - t0)
                await asyncio.sleep(0.2)
        results["phases"].append({
            "phase": "layout_save_8x",
            "server": sampler.phase_stats(idx),
            "api_post_latency_ms_p50": round(statistics.median(post_lat) * 1000, 1),
            "api_post_latency_ms_max": round(max(post_lat) * 1000, 1),
            "post_statuses": statuses,
        })

        # ---- Phase 5: preview PNG render (time-boxed 5 s) ----
        lat = []
        idx = len(sampler.samples)
        sizes = []
        async with aiohttp.ClientSession() as http:
            end = time.perf_counter() + 5.0
            while time.perf_counter() < end:
                t0 = time.perf_counter()
                async with http.get(f"http://127.0.0.1:{web_port}/api/designs/default/preview") as r:
                    body = await r.read()
                lat.append(time.perf_counter() - t0)
                sizes.append(len(body))
        n = len(lat)
        results["phases"].append({
            "phase": "preview_render_5s",
            "server": sampler.phase_stats(idx),
            "requests": n,
            "req_per_s": round(n / sum(lat), 2),
            "latency_ms_p50": round(statistics.median(lat) * 1000, 1),
            "latency_ms_p95": round(sorted(lat)[int(n * 0.95)] * 1000, 1),
            "png_bytes_avg": round(statistics.mean(sizes)),
        })

        sampler.stop()
        try:
            await asyncio.wait_for(samp_task, 2.0)
        except asyncio.TimeoutError:
            samp_task.cancel()
        for d in devices:
            await d.close()
        results["rss_idle_start_kb"] = sampler.samples[0][2] if sampler.samples else None
    finally:
        proc.send_signal(signal.SIGINT)
        try:
            proc.wait(5)
        except subprocess.TimeoutExpired:
            proc.kill()
        meta["log"].close()
    return results


SEED_STUDIO = {
    "schema_version": 1,
    "designs": {
        "default": {
            "id": "default",
            "name": "Bench clock",
            "revision": 0,
            "layout": {
                "schemaVersion": 1,
                "name": "Bench clock",
                "orientation": "portrait",
                "profile": "ili9341-240x320",
                "background": "#0a0a1e",
                "actions": [],
                "dataSources": [],
                "elements": [
                    {
                        "id": "clock",
                        "name": "Clock",
                        "type": "clock",
                        "frame": {"x": 0, "y": 56, "w": 240, "h": 68},
                        "props": {"format": "%H:%M:%S", "timezone": ""},
                        "style": {
                            "align": "center",
                            "color": "#ffffff",
                            "fontSize": 48,
                            "verticalAlign": "middle",
                        },
                        "visible": True,
                        "locked": False,
                    },
                ],
            },
        }
    },
    "screens": {},
}


def prepare_data():
    if DATA.exists():
        shutil.rmtree(DATA)
    for impl in PORTS:
        d = DATA / impl
        d.mkdir(parents=True)
        (d / "studio.json").write_text(json.dumps(SEED_STUDIO, indent=2))
        (d / "secrets.json").write_text("{}")


async def main():
    RESULTS.mkdir(parents=True, exist_ok=True)
    prepare_data()
    all_results = {}
    for impl in PORTS:
        print(f"=== benchmarking {impl} ===", flush=True)
        all_results[impl] = await run_suite(impl)
        (RESULTS / f"{impl}_results.json").write_text(json.dumps(all_results[impl], indent=2))
        print(json.dumps(all_results[impl], indent=2), flush=True)
    print("\nDone. Raw results in bench/results/")


if __name__ == "__main__":
    asyncio.run(main())

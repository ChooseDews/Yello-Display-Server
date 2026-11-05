#!/usr/bin/env python3
"""Generate FramePatch test cases for the host patch_stream test.

Each case is a `<name>.packet` (the exact WebSocket message bytes the
server would send) plus a `<name>.bundle` (the uncompressed zone-record
bundle the packet must decode to). Packet construction mirrors
server-rs/src/protocol/mod.rs: 20-byte header, zlib windowBits=12 for
compressed payloads, raw payload when flags say so.
"""
import os
import random
import struct
import sys
import zlib

OUT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "cases")
HDR = struct.Struct("<BBBBIHHII")
REC = struct.Struct("<HHHH")
MAGIC, VERSION, MSG_PATCH = 0x59, 2, 6
FLAG_COMPRESSED, FLAG_FULL = 0x01, 0x02


def bundle(zones):
    out = bytearray()
    for (x, y, w, h, pixels) in zones:
        assert len(pixels) == w * h * 2
        out += REC.pack(x, y, w, h)
        out += pixels
    return bytes(out)


def packet(seq, full, zones, compress):
    raw = bundle(zones)
    if compress:
        co = zlib.compressobj(6, zlib.DEFLATED, 12)
        payload = co.compress(raw) + co.flush()
        flags = FLAG_COMPRESSED | (FLAG_FULL if full else 0)
    else:
        payload = raw
        flags = FLAG_FULL if full else 0
    hdr = HDR.pack(MAGIC, VERSION, MSG_PATCH, flags, seq, len(zones), 0, len(raw), len(payload))
    return hdr + payload, raw


def write(name, seq, full, zones, compress):
    pkt, raw = packet(seq, full, zones, compress)
    with open(os.path.join(OUT, name + ".packet"), "wb") as f:
        f.write(pkt)
    with open(os.path.join(OUT, name + ".bundle"), "wb") as f:
        f.write(raw)
    print(f"{name}: packet={len(pkt)} bundle={len(raw)} zones={len(zones)} compress={compress}")


def pattern_pixels(w, h, seed):
    rnd = random.Random(seed)
    px = bytearray()
    base = rnd.randrange(0, 0x10000)
    for i in range(w * h):
        v = base if (i // w) % 3 else (base ^ 0xFFFF)
        px += struct.pack(">H", v & 0xFFFF)
    return bytes(px)


def noise_pixels(w, h, seed):
    rnd = random.Random(seed)
    return bytes(rnd.randrange(256) for _ in range(w * h * 2))


def main():
    os.makedirs(OUT, exist_ok=True)
    # 1: UI-like full frame, one zone, compressed.
    write("ui_full", 0, True, [(0, 0, 240, 320, pattern_pixels(240, 320, 1))], True)
    # 2: multi-zone patch, raw (forced), assorted shapes incl. odd widths.
    write("zones_raw", 1, False, [
        (0, 0, 240, 12, pattern_pixels(240, 12, 2)),
        (13, 40, 13, 7, pattern_pixels(13, 7, 3)),
        (200, 100, 40, 1, pattern_pixels(40, 1, 4)),
        (0, 319, 240, 1, pattern_pixels(240, 1, 5)),
        (120, 200, 60, 33, noise_pixels(60, 33, 6)),
    ], False)
    # 3: multi-zone compressed, zones crossing strip boundaries.
    write("zones_zlib", 2, False, [
        (10, 10, 200, 37, pattern_pixels(200, 37, 7)),
        (0, 100, 320 - 80, 25, pattern_pixels(240, 25, 8)),
        (55, 200, 129, 64, pattern_pixels(129, 64, 9)),
    ], True)
    # 4: seq-gap probe (seq 9 when 3 is expected) then a full resync (seq 10).
    write("gap_patch", 9, False, [(0, 0, 100, 50, pattern_pixels(100, 50, 10))], True)
    write("resync_full", 10, True, [(0, 0, 240, 320, pattern_pixels(240, 320, 11))], True)
    # 5: corrupted compressed stream (flip bytes inside the payload).
    pkt, raw = packet(11, True, [(0, 0, 240, 320, pattern_pixels(240, 320, 12))], True)
    bad = bytearray(pkt)
    for i in range(30, 40):
        bad[i] ^= 0xFF
    with open(os.path.join(OUT, "corrupt.packet"), "wb") as f:
        f.write(bytes(bad))
    with open(os.path.join(OUT, "corrupt.bundle"), "wb") as f:
        f.write(raw)
    print(f"corrupt: packet={len(bad)}")
    # 6: windowBits=15 stream - the 4 KiB dictionary must reject it.
    raw_zones = [(0, 0, 240, 320, pattern_pixels(240, 320, 13))]
    raw = bundle(raw_zones)
    co = zlib.compressobj(6, zlib.DEFLATED, 15)
    payload = co.compress(raw) + co.flush()
    hdr = HDR.pack(MAGIC, VERSION, MSG_PATCH, FLAG_COMPRESSED | FLAG_FULL, 12, 1, 0, len(raw), len(payload))
    with open(os.path.join(OUT, "wbits15.packet"), "wb") as f:
        f.write(hdr + payload)
    with open(os.path.join(OUT, "wbits15.bundle"), "wb") as f:
        f.write(raw)
    print(f"wbits15: packet={len(hdr) + len(payload)}")
    # 7: truncated compressed stream (payload cut in half, header honest
    # about the full payload length so the WS layer would deliver less
    # than promised only in this synthetic end() test - here we keep the
    # header payload_len as the truncated length to simulate a stream
    # that simply ends early).
    pkt, raw = packet(13, True, [(0, 0, 240, 320, pattern_pixels(240, 320, 14))], True)
    trunc = pkt[: 20 + (len(pkt) - 20) // 2]
    thdr = HDR.pack(MAGIC, VERSION, MSG_PATCH, FLAG_COMPRESSED | FLAG_FULL, 13, 1, 0, len(raw),
                    len(trunc) - 20)
    with open(os.path.join(OUT, "truncated.packet"), "wb") as f:
        f.write(thdr + trunc[20:])
    with open(os.path.join(OUT, "truncated.bundle"), "wb") as f:
        f.write(raw)
    print(f"truncated: packet={len(trunc)}")
    # 8: photo frame from the codec bench, if present.
    photo = os.path.expanduser("~/workspace/yello-codec-bench/photo.raw")
    if os.path.exists(photo):
        with open(photo, "rb") as f:
            px = f.read()
        if len(px) == 240 * 320 * 2:
            write("photo_full", 14, True, [(0, 0, 240, 320, px)], True)


if __name__ == "__main__":
    sys.exit(main())

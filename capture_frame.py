#!/usr/bin/env python3
"""Grab a single frame from a webcam pointed at the device screen."""

import argparse
import sys
import time
from pathlib import Path

import cv2


def _device_mask(img):
    """Pixels that are saturated (orange PCB / blue screen) or dark (black bezel) — i.e. not the paper background."""
    hsv = cv2.cvtColor(img, cv2.COLOR_BGR2HSV)
    saturation, value = hsv[:, :, 1], hsv[:, :, 2]
    mask = ((saturation > 60) | (value < 80)).astype("uint8") * 255
    kernel = cv2.getStructuringElement(cv2.MORPH_RECT, (15, 15))
    mask = cv2.morphologyEx(mask, cv2.MORPH_CLOSE, kernel)
    mask = cv2.morphologyEx(mask, cv2.MORPH_OPEN, kernel)
    return mask


def _largest_contour(mask):
    contours, _ = cv2.findContours(mask, cv2.RETR_EXTERNAL, cv2.CHAIN_APPROX_SIMPLE)
    if not contours:
        return None
    return max(contours, key=cv2.contourArea)


def deskew_and_crop(frame, padding: int = 12):
    """Find the device against the background, straighten it, and crop tight around it.

    Falls back to the original frame if the device can't be confidently located
    (e.g. background isn't plain paper, or nothing contrasts against it).
    """
    contour = _largest_contour(_device_mask(frame))
    if contour is None or cv2.contourArea(contour) < 0.02 * frame.shape[0] * frame.shape[1]:
        return frame

    (_, _), (_, _), angle = cv2.minAreaRect(contour)
    if angle < -45:
        angle += 90

    h, w = frame.shape[:2]
    rot_matrix = cv2.getRotationMatrix2D((w / 2, h / 2), angle, 1.0)
    cos, sin = abs(rot_matrix[0, 0]), abs(rot_matrix[0, 1])
    new_w = int(h * sin + w * cos)
    new_h = int(h * cos + w * sin)
    rot_matrix[0, 2] += (new_w - w) / 2
    rot_matrix[1, 2] += (new_h - h) / 2
    rotated = cv2.warpAffine(
        frame, rot_matrix, (new_w, new_h), flags=cv2.INTER_CUBIC, borderMode=cv2.BORDER_REPLICATE
    )

    contour = _largest_contour(_device_mask(rotated))
    if contour is None:
        return rotated

    x, y, box_w, box_h = cv2.boundingRect(contour)
    x0, y0 = max(0, x - padding), max(0, y - padding)
    x1, y1 = min(rotated.shape[1], x + box_w + padding), min(rotated.shape[0], y + box_h + padding)
    cropped = rotated[y0:y1, x0:x1]

    # Camera is mounted looking at the device upside down and mirrored relative to how it's read.
    cropped = cv2.rotate(cropped, cv2.ROTATE_180)
    return cv2.flip(cropped, 1)


DEFAULT_SNAPSHOT_NAME = "device_snapshot.jpg"
HISTORY_DIR = Path("history/snapshots")


def grab_frame(device: str, warmup_frames: int, width: int | None, height: int | None):
    index_or_path = int(device) if device.isdigit() else device
    cap = cv2.VideoCapture(index_or_path)

    if width:
        cap.set(cv2.CAP_PROP_FRAME_WIDTH, width)
    if height:
        cap.set(cv2.CAP_PROP_FRAME_HEIGHT, height)

    if not cap.isOpened():
        sys.exit(f"error: could not open camera '{device}'")

    try:
        # Discard the first few frames: auto-exposure/auto-focus needs time to settle.
        frame = None
        for _ in range(warmup_frames + 1):
            ok, frame = cap.read()
            if not ok:
                sys.exit("error: failed to read frame from camera")
            time.sleep(0.03)
    finally:
        cap.release()

    return frame


def capture(device: str, out_path: Path, warmup_frames: int, width: int | None, height: int | None, raw: bool) -> None:
    frame = grab_frame(device, warmup_frames, width, height)
    if not raw:
        frame = deskew_and_crop(frame)

    out_path.parent.mkdir(parents=True, exist_ok=True)
    if not cv2.imwrite(str(out_path), frame):
        sys.exit(f"error: failed to write image to {out_path}")

    print(str(out_path))


def capture_default(device: str, warmup_frames: int, width: int | None, height: int | None, raw: bool) -> None:
    """Default flow: overwrite ./device_snapshot.jpg and keep a dated copy under ./history/snapshots/."""
    frame = grab_frame(device, warmup_frames, width, height)
    if not raw:
        frame = deskew_and_crop(frame)

    snapshot_path = Path(DEFAULT_SNAPSHOT_NAME).resolve()
    if not cv2.imwrite(str(snapshot_path), frame):
        sys.exit(f"error: failed to write image to {snapshot_path}")

    HISTORY_DIR.mkdir(parents=True, exist_ok=True)
    history_path = HISTORY_DIR / f"device_{time.strftime('%Y%m%d_%H%M%S')}.jpg"
    if not cv2.imwrite(str(history_path), frame):
        sys.exit(f"error: failed to write image to {history_path}")

    print(str(snapshot_path))


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "-o",
        "--output",
        default=None,
        help=(
            f"output image path. If omitted, writes {DEFAULT_SNAPSHOT_NAME} in the current "
            f"directory (replacing any existing one) plus a dated copy in {HISTORY_DIR}/"
        ),
    )
    parser.add_argument("-d", "--device", default="0", help="camera index or /dev/videoN path (default: 0)")
    parser.add_argument("--warmup", type=int, default=5, help="frames to discard before capturing (default: 5)")
    parser.add_argument("--width", type=int, default=None, help="requested capture width")
    parser.add_argument("--height", type=int, default=None, help="requested capture height")
    parser.add_argument(
        "--raw", action="store_true", help="skip auto deskew/crop to the device and save the full frame"
    )
    args = parser.parse_args()

    if args.output:
        capture(args.device, Path(args.output), args.warmup, args.width, args.height, args.raw)
    else:
        capture_default(args.device, args.warmup, args.width, args.height, args.raw)


if __name__ == "__main__":
    main()

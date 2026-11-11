"""Pixel-art audit: checks art against the style guide's hard rules and reports soft metrics.

python3 tools/art/audit.py [--frame 64] [--silhouettes OUT.png] FILE.png [FILE.png ...]

Hard rules (a failure exits non-zero, so CI can gate on it):
- every opaque pixel is one of the 81 palette colours
- alpha is 0 or 255
- no upscaled "mixels": art is at native resolution (if most colour runs have even
  length, the image was probably scaled 2x)

Soft metrics (reported, never fatal):
- orphans: opaque pixels whose colour matches none of their 8 neighbours (speckle)
- flat4: share of pixels matching all 4 neighbours (clean clusters; very low = noise)
- checker: share of pixels in a checkerboard dither pattern
- colours used, and per-frame silhouette size when --frame is given

--silhouettes writes every frame as a solid black shape on white, the fastest way to see
whether characters read apart without colour.
"""
import argparse
import os
import sys

import numpy as np
from PIL import Image

sys.path.insert(0, os.path.dirname(__file__))
from palette import ALL  # noqa: E402

PALETTE = {tuple(colour) for colour in ALL}


def colour_keys(rgb, opaque):
    key = (rgb[..., 0].astype(np.int64) << 16) | (rgb[..., 1].astype(np.int64) << 8) | rgb[..., 2]
    return np.where(opaque, key, -1)


def neighbour_equal(key, offsets):
    """For each offset, a mask of pixels whose neighbour at that offset has the same key."""
    padded = np.pad(key, 1, constant_values=-2)
    h, w = key.shape
    return [padded[1 + dy:1 + dy + h, 1 + dx:1 + dx + w] == key for dy, dx in offsets]


def even_run_share(rgb, opaque):
    """Share of horizontal same-colour runs (inside opaque areas) with even length."""
    even = total = 0
    for y in range(rgb.shape[0]):
        row, mask = rgb[y], opaque[y]
        change = np.any(row[1:] != row[:-1], axis=1) | (mask[1:] != mask[:-1])
        edges = np.r_[0, np.flatnonzero(change) + 1, row.shape[0]]
        lengths, inside = np.diff(edges), mask[edges[:-1]]
        total += int(inside.sum())
        even += int((inside & (lengths % 2 == 0)).sum())
    return even / total if total else 0.0


def background_mask(rgba):
    """Opaque mask; an image with no transparency treats its most common colour as background."""
    opaque = rgba[..., 3] > 0
    if opaque.all():
        rgb = rgba[..., :3].reshape(-1, 3)
        values, counts = np.unique(rgb, axis=0, return_counts=True)
        background = values[counts.argmax()]
        opaque = ~np.all(rgba[..., :3] == background, axis=-1)
    return opaque


def audit(path, frame=None):
    rgba = np.array(Image.open(path).convert("RGBA"))
    rgb, alpha = rgba[..., :3], rgba[..., 3]
    opaque = alpha > 0
    report = {"file": path, "size": f"{rgba.shape[1]}x{rgba.shape[0]}", "errors": []}

    partial = int(((alpha > 0) & (alpha < 255)).sum())
    if partial:
        report["errors"].append(f"{partial} pixels with partial alpha")
    used = {tuple(c) for c in np.unique(rgb[opaque], axis=0)} if opaque.any() else set()
    off = used - PALETTE
    if off:
        palette_keys = np.array([(r << 16) | (g << 8) | b for r, g, b in PALETTE])
        count = int((opaque & ~np.isin(colour_keys(rgb, opaque), palette_keys)).sum())
        report["errors"].append(f"{len(off)} off-palette colours on {count} pixels")
    content = background_mask(rgba)
    evens = even_run_share(rgb, content)
    if evens > 0.85:
        report["errors"].append(f"{evens:.0%} of colour runs are even: looks upscaled")

    key = colour_keys(rgb, content)
    eight = [(dy, dx) for dy in (-1, 0, 1) for dx in (-1, 0, 1) if dy or dx]
    any_same = np.logical_or.reduce(neighbour_equal(key, eight))
    four_same = np.logical_and.reduce(neighbour_equal(key, [(-1, 0), (1, 0), (0, -1), (0, 1)]))
    diag_same = np.logical_and.reduce(neighbour_equal(key, [(-1, -1), (-1, 1), (1, -1), (1, 1)]))
    four_diff = ~np.logical_or.reduce(neighbour_equal(key, [(-1, 0), (1, 0), (0, -1), (0, 1)]))
    pixels = max(int(content.sum()), 1)
    report["colours"] = len(used)
    report["even_runs"] = round(evens, 3)
    report["orphans_pct"] = round(100 * (content & ~any_same).sum() / pixels, 2)
    report["flat4_pct"] = round(100 * (content & four_same).sum() / pixels, 1)
    report["checker_pct"] = round(100 * (content & four_diff & diag_same).sum() / pixels, 2)

    if frame:
        frames = []
        for top in range(0, rgba.shape[0], frame):
            for left in range(0, rgba.shape[1], frame):
                mask = content[top:top + frame, left:left + frame]
                if not mask.any():
                    continue
                ys, xs = np.nonzero(mask)
                frames.append(f"{xs.max() - xs.min() + 1}x{ys.max() - ys.min() + 1}")
        report["frames"] = frames
    return report, content


def silhouette_sheet(contents, out):
    height = max(c.shape[0] for c in contents)
    width = sum(c.shape[1] for c in contents) + 4 * (len(contents) - 1)
    sheet = np.full((height, width), 235, np.uint8)
    x = 0
    for content in contents:
        sheet[:content.shape[0], x:x + content.shape[1]][content] = 20
        x += content.shape[1] + 4
    Image.fromarray(sheet).resize((width * 4, height * 4), Image.NEAREST).save(out)


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("files", nargs="+")
    parser.add_argument("--frame", type=int, help="frame size for per-frame silhouette stats")
    parser.add_argument("--silhouettes", help="write a solid-silhouette sheet to this path")
    args = parser.parse_args()
    failed = False
    contents = []
    for path in args.files:
        report, content = audit(path, args.frame)
        contents.append(content)
        errors = report.pop("errors")
        status = "FAIL" if errors else "ok"
        failed |= bool(errors)
        details = ", ".join(f"{k}={v}" for k, v in report.items() if k != "file")
        print(f"{status:4} {path}: {details}")
        for error in errors:
            print(f"     - {error}")
    if args.silhouettes:
        silhouette_sheet(contents, args.silhouettes)
        print("wrote", args.silhouettes)
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()

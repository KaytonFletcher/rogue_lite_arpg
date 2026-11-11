"""Cleans outside art (PixelLab, Retro Diffusion, an upscaled export, a photo bash) into
on-style pixel art: native resolution, binary alpha, palette-locked, no speckle.

python3 tools/art/clean.py IN.png OUT.png [--scale auto|N] [--ramps ash,earth,...]
                           [--background auto|none] [--keep-orphans] [--outline]

Steps, in order:
1. Background: with `auto`, an image with no transparency loses the colour that fills its
   corners (flood-filled from the edges, so the same colour inside the sprite survives).
2. Alpha: anything under half opacity becomes transparent, the rest fully opaque.
3. Grid: detects the native pixel size (the period of colour edges across the image) and its
   offset, then samples each cell by its most common colour. Handles whole-number scales; for
   drifting, non-integer AI grids run Pixel Snapper first (pip install pixel-snapper).
4. Palette: every pixel snaps to the nearest palette colour in OKLab. `--ramps` limits the
   snap to named ramps (say `cedar,gold,reed,river` for the People), so meaning-carrying
   colours like blight green or the one red can't leak in.
5. Speckle: single pixels that match none of their 8 neighbours take the most common
   neighbouring colour, unless --keep-orphans.
6. Outline (optional): transparent pixels touching the sprite get the darkest step of the
   ramp they touch, the style guide's selective outline.

The output always passes audit.py's hard rules. Composition, silhouettes and lighting are
still yours to judge: run audit.py --silhouettes and look.
"""
import argparse
import os
import sys
from collections import Counter

import numpy as np
from PIL import Image

sys.path.insert(0, os.path.dirname(__file__))
from palette import PAL, srgb_to_oklab  # noqa: E402


def remove_background(rgba, tolerance=10):
    """Flood-fills the corner colour from the image border to transparent."""
    if (rgba[..., 3] < 255).any():
        return rgba
    rgb = rgba[..., :3].astype(int)
    corners = [tuple(rgb[y, x]) for y in (0, -1) for x in (0, -1)]
    background = np.array(Counter(corners).most_common(1)[0][0])
    similar = np.abs(rgb - background).max(axis=-1) <= tolerance
    h, w = similar.shape
    filled = np.zeros_like(similar)
    stack = [(y, x) for y in range(h) for x in (0, w - 1)] + [(y, x) for x in range(w) for y in (0, h - 1)]
    while stack:
        y, x = stack.pop()
        if 0 <= y < h and 0 <= x < w and similar[y, x] and not filled[y, x]:
            filled[y, x] = True
            stack.extend(((y + 1, x), (y - 1, x), (y, x + 1), (y, x - 1)))
    out = rgba.copy()
    out[filled, 3] = 0
    return out


def edge_profile(rgba, axis):
    """Colour-change strength between neighbouring columns (axis=1) or rows (axis=0)."""
    lab = srgb_to_oklab(rgba[..., :3])
    lab[rgba[..., 3] < 128] = 9.0  # transparency is a strong edge too
    diff = np.abs(np.diff(lab, axis=axis)).sum(axis=-1)
    diff[diff < 0.06] = 0  # ignore compression and dither noise; keep real colour edges
    return diff.sum(axis=1 - axis)


def autocorrelation(profile, max_lag):
    centred = profile - profile.mean()
    energy = (centred * centred).sum() + 1e-9
    return np.array([(centred[:-lag] * centred[lag:]).sum() / energy
                     for lag in range(1, max_lag + 2)])


def detect_scale(rgba, max_scale=16):
    """Native pixel size and grid offset (x, y) of possibly upscaled art.

    Colour edges in art scaled by s repeat every s pixels, so the autocorrelation of the
    edge profile spikes at lag s (and its multiples) and sits low at the lags next to it.
    Art at native size decays smoothly with no spike and keeps scale 1. The offset is where
    the edges pile up. Pass --scale when you know it.
    """
    profiles = [edge_profile(rgba, 1), edge_profile(rgba, 0)]
    if min(len(p) for p in profiles) < (max_scale + 2) * 2:
        return 1, 0, 0
    corr = sum(autocorrelation(p, max_scale) for p in profiles) / 2  # corr[i] is lag i + 1
    for scale in range(2, max_scale + 1):
        here = corr[scale - 1]
        spike = here - max(corr[scale - 2], corr[scale])
        if here >= 0.3 and spike >= 0.25:
            offsets = []
            for profile in profiles:
                sums = [profile[o::scale].sum() for o in range(scale)]
                # Edge index i sits between pixels i and i+1, so cells start at offset + 1.
                offsets.append((int(np.argmax(sums)) + 1) % scale)
            return scale, offsets[0], offsets[1]
    return 1, 0, 0


def downsample(rgba, scale, offset_x, offset_y):
    """Most common colour of each cell's central pixels (edges of a cell are the blurriest)."""
    h, w = rgba.shape[:2]
    cols = (w - offset_x) // scale
    rows = (h - offset_y) // scale
    out = np.zeros((rows, cols, 4), np.uint8)
    inset = scale // 4
    for row in range(rows):
        for col in range(cols):
            y0, x0 = offset_y + row * scale, offset_x + col * scale
            cell = rgba[y0 + inset:y0 + scale - inset, x0 + inset:x0 + scale - inset]
            cell = cell.reshape(-1, 4)
            counts = Counter(map(tuple, cell))
            out[row, col] = counts.most_common(1)[0][0]
    return out


def lock_palette(rgba, ramps):
    colours = np.array([c for name in ramps for c in PAL[name]], np.uint8)
    lab_palette = srgb_to_oklab(colours)
    lab = srgb_to_oklab(rgba[..., :3]).reshape(-1, 1, 3)
    nearest = np.argmin(((lab - lab_palette[None]) ** 2).sum(axis=-1), axis=1)
    out = rgba.copy()
    out[..., :3] = colours[nearest].reshape(rgba.shape[:2] + (3,))
    return out


def kill_orphans(rgba):
    out = rgba.copy()
    h, w = rgba.shape[:2]
    for y in range(h):
        for x in range(w):
            if rgba[y, x, 3] == 0:
                continue
            neighbours = [tuple(rgba[y + dy, x + dx]) for dy in (-1, 0, 1) for dx in (-1, 0, 1)
                          if (dy or dx) and 0 <= y + dy < h and 0 <= x + dx < w]
            opaque = [n for n in neighbours if n[3] == 255]
            if len(opaque) >= 5 and tuple(rgba[y, x]) not in opaque:
                out[y, x] = Counter(opaque).most_common(1)[0][0]
    return out


def outline(rgba, ramps):
    """Gives transparent pixels that touch the sprite the darkest step of the touching ramp."""
    out = rgba.copy()
    lookup = {c: name for name in ramps for c in PAL[name]}
    h, w = rgba.shape[:2]
    for y in range(h):
        for x in range(w):
            if rgba[y, x, 3]:
                continue
            for dy, dx in ((0, 1), (1, 0), (0, -1), (-1, 0)):
                ny, nx = y + dy, x + dx
                if 0 <= ny < h and 0 <= nx < w and rgba[ny, nx, 3]:
                    ramp = lookup.get(tuple(int(v) for v in rgba[ny, nx, :3]))
                    if ramp:
                        out[y, x] = (*PAL[ramp][0], 255)
                        break
    return out


def clean(rgba, scale="auto", ramps=None, background="auto", keep_orphans=False,
          add_outline=False):
    ramps = ramps or list(PAL)
    if background == "auto":
        rgba = remove_background(rgba)
    rgba = rgba.copy()
    rgba[..., 3] = np.where(rgba[..., 3] >= 128, 255, 0)
    if scale == "auto":
        scale, offset_x, offset_y = detect_scale(rgba)
    else:
        scale, offset_x, offset_y = int(scale), 0, 0
    # Snap to the palette before sampling so a cell's blurred pixels vote for the same colour.
    rgba = lock_palette(rgba, ramps)
    if scale > 1:
        rgba = downsample(rgba, scale, offset_x, offset_y)
    rgba[rgba[..., 3] == 0] = 0
    if not keep_orphans:
        rgba = kill_orphans(rgba)
    if add_outline:
        rgba = outline(rgba, ramps)
    return rgba, scale


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("input")
    parser.add_argument("output")
    parser.add_argument("--scale", default="auto")
    parser.add_argument("--ramps", help="comma-separated ramp names; default all")
    parser.add_argument("--background", default="auto", choices=["auto", "none"])
    parser.add_argument("--keep-orphans", action="store_true")
    parser.add_argument("--outline", action="store_true")
    args = parser.parse_args()
    ramps = args.ramps.split(",") if args.ramps else None
    unknown = set(ramps or []) - set(PAL)
    if unknown:
        parser.error(f"unknown ramps {sorted(unknown)}; known: {', '.join(PAL)}")
    rgba = np.array(Image.open(args.input).convert("RGBA"))
    out, scale = clean(rgba, args.scale, ramps, args.background, args.keep_orphans, args.outline)
    Image.fromarray(out).save(args.output)
    print(f"wrote {args.output} ({out.shape[1]}x{out.shape[0]}, detected scale {scale})")


if __name__ == "__main__":
    main()

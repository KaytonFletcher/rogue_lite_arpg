"""Renders a test map with the dual-grid atlas, following the same rules as tiling.rs.

python3 tools/tileset/preview.py [out.png]

Glyphs: ~ water, . ground, , grass, c cobble, w wood, # wall, _ dead zone.
Mark overlay: f fissure, s scorch. Haze overlay: h.
"""
import os
import sys

import numpy as np
from PIL import Image

T, COLS = 32, 16
HERE = os.path.dirname(__file__)
atlas = np.array(Image.open(os.path.join(HERE, "..", "..", "assets", "tilesets", "tile_atlas.png")))

TERRAIN = [
    ",,,,,,,,,,,,,,,,,,,,",
    ",,,,~~~~,,,,,....,,,",
    ",,,~~~~~~,,,.......,",
    ",,~~~~~~~~,,....cc..",
    ",,,~~~~~~,,,...cccc.",
    ",,,,~~..,,,,...cc...",
    ",,,,,,..,,,,,,,,,...",
    ",~,,,,,,,,##w###,,,,",
    ",,,,,,,,,,#www#,,,,,",
    ",,,....,,,#www#,,~~,",
    ",,......,,,,,,,,,~~,",
    ",,,,,,,,,,,,,,,,,,,,",
]
MARKS = [
    "....................",
    "....................",
    "....................",
    "....................",
    "....................",
    "....................",
    "..........ss........",
    "....................",
    "..ff.............ss.",
    "...ff.......f...sss.",
    "....fff....fff......",
    "......f.............",
]
HAZE = [
    "....................",
    "....................",
    "..............hhh...",
    ".............hhhhh..",
    "..............hhh...",
] + ["...................."] * 7

H, W = len(TERRAIN), len(TERRAIN[0])
RANK = {"~": 0, ".": 1, "#": 1, "w": 1, "_": 1, ",": 2, "c": 3}
LAYERS = [(1, 16, 288), (2, 48, 304), (3, 80, 320)]  # (rank, transitions, variant B)
DECOR = {",": 336, ".": 368}  # first decor slot for grass and ground cells


def cell(grid, x, y):
    """Clamped lookup with +y north: row 0 of the text is the top (north) row."""
    x = min(max(x, 0), W - 1)
    y = min(max(y, 0), H - 1)
    return grid[H - 1 - y][x]


def variant(x, y, salt, count=8):
    return (x * 73856093 ^ y * 19349663 ^ salt * 83492791) % count


def corner_mask(test, i, j):
    """Dual tile (i, j) sits on the corner shared by cells (i-1..i, j-1..j)."""
    return (test(i - 1, j) | test(i, j) << 1 | test(i - 1, j - 1) << 2 | test(i, j - 1) << 3)


def dual_index(first, mask, i, j, salt, alt=None):
    if mask == 0:
        return None
    if mask == 15:
        return first + COLS + variant(i, j, salt)
    if alt is not None and variant(i, j, salt + 32, 2):
        return alt + mask
    return first + mask


def patch(x, y, salt):
    """Smooth value noise on a 4-cell lattice, 0..1: where decor clusters."""
    gx, gy = x / 4, y / 4
    x0, y0 = int(np.floor(gx)), int(np.floor(gy))
    fx, fy = gx - x0, gy - y0
    fx, fy = fx * fx * (3 - 2 * fx), fy * fy * (3 - 2 * fy)
    v = lambda a, b: variant(a, b, salt, 1000) / 999
    top = v(x0, y0) * (1 - fx) + v(x0 + 1, y0) * fx
    bottom = v(x0, y0 + 1) * (1 - fx) + v(x0 + 1, y0 + 1) * fx
    return top * (1 - fy) + bottom * fy


def decor_index(x, y):
    kind = cell(TERRAIN, x, y)
    if kind not in DECOR:
        return None
    if any(cell(TERRAIN, x + dx, y + dy) != kind for dx, dy in ((1, 0), (-1, 0), (0, 1), (0, -1))):
        return None
    density = min(max((patch(x, y, 61) - 0.4) * 2.5, 0.0), 1.0) * 0.7
    if variant(x, y, 62, 1000) / 1000 >= density:
        return None
    return DECOR[kind] + variant(x, y, 63, 32)


def blit(img, index, px, py):
    row, col = divmod(index, COLS)
    tile = atlas[row * T:(row + 1) * T, col * T:(col + 1) * T]
    region = img[py:py + T, px:px + T]
    opaque = tile[..., 3] == 255
    region[opaque] = tile[opaque]


img = np.zeros(((H + 1) * T, (W + 1) * T, 4), np.uint8)
# Dual layers are drawn half a tile up-left of the cell grid; image row 0 is north.
def dual_px(i, j):
    return i * T, (H - j) * T


for j in range(H + 1):
    for i in range(W + 1):
        blit(img, variant(i, j, 0), *dual_px(i, j))
for rank, first, alt in LAYERS:
    for j in range(H + 1):
        for i in range(W + 1):
            mask = corner_mask(lambda x, y: int(RANK[cell(TERRAIN, x, y)] >= rank), i, j)
            index = dual_index(first, mask, i, j, rank, alt)
            if index is not None:
                blit(img, index, *dual_px(i, j))
for y in range(H):
    for x in range(W):
        index = decor_index(x, y)
        if index is not None:
            blit(img, index, x * T + T // 2, (H - 1 - y) * T + T // 2)
for glyph, first in (("f", 160), ("s", 192)):
    for j in range(H + 1):
        for i in range(W + 1):
            mask = corner_mask(lambda x, y: int(cell(MARKS, x, y) == glyph), i, j)
            index = dual_index(first, mask, i, j, ord(glyph))
            if index is not None:
                blit(img, index, *dual_px(i, j))
# Cell layer: walls (simplified to straight faces), wood, void.
for y in range(H):
    for x in range(W):
        kind = cell(TERRAIN, x, y)
        px, py = x * T + T // 2, (H - 1 - y) * T + T // 2
        if kind == "w":
            blit(img, 112 + variant(x, y, 9), px, py)
        elif kind == "#":
            blit(img, 128 + 14, px, py)
        elif kind == "_":
            blit(img, 127, px, py)
for j in range(H + 1):
    for i in range(W + 1):
        mask = corner_mask(lambda x, y: int(cell(HAZE, x, y) == "h"), i, j)
        if mask:
            blit(img, 224 + mask * 4, *dual_px(i, j))

crop = img[T // 2:T // 2 + H * T, T // 2:T // 2 + W * T]
out = sys.argv[1] if len(sys.argv) > 1 else "/tmp/preview.png"
Image.fromarray(crop).save(out)
print("wrote", out)

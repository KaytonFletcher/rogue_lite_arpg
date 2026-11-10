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
LAYERS = [(1, 16), (2, 48), (3, 80)]  # (rank, first transition index)


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


def dual_index(first, mask, i, j, salt):
    if mask == 0:
        return None
    if mask == 15:
        return first + COLS + variant(i, j, salt)
    return first + mask


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
for rank, first in LAYERS:
    for j in range(H + 1):
        for i in range(W + 1):
            mask = corner_mask(lambda x, y: int(RANK[cell(TERRAIN, x, y)] >= rank), i, j)
            index = dual_index(first, mask, i, j, rank)
            if index is not None:
                blit(img, index, *dual_px(i, j))
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

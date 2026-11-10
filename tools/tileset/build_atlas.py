"""Builds assets/tilesets/tile_atlas.png: the dual-grid (corner Wang) tile atlas.

Run from the repo root:  python3 tools/tileset/build_atlas.py

Layout (32px tiles, 16 columns; `src/world/map/tiling.rs` mirrors these offsets):

    row  0  water fills (8)                         base layer, under everything
    row  1  ground transitions, index = corner mask (TL=1, TR=2, BL=4, BR=8)
    row  2  ground fills (8)                        used where all four corners are ground
    row  3  grass transitions      row  4  grass fills
    row  5  cobble transitions     row  6  cobble fills
    row  7  wood fills (8), col 15 = void (dead zone)
    rows 8-9  wall pieces (old atlas 100-119, same order)
    row 10  fissure transitions    row 11  fissure fills
    row 12  scorch transitions     row 13  scorch fills
    rows 14-17  haze: 16 masks x 4 animation frames, index = 224 + mask * 4 + frame

A transition tile paints only its upper terrain; the rest is transparent, so a lower layer
shows through. Every edge is cut from one tileable noise field, so two tiles that share a
corner pair always meet without a seam. Alpha is 0 or 255 and every pixel is on the palette.
"""
import os
import sys

import numpy as np
from PIL import Image

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "art"))
from palette import ALL, c  # noqa: E402

T = 32
COLS = 16
ROWS = 18
HERE = os.path.dirname(__file__)
SOURCE = os.path.join(HERE, "source_village.png")
OUT = os.path.join(HERE, "..", "..", "assets", "tilesets", "tile_atlas.png")

src = np.array(Image.open(SOURCE).convert("RGBA"))


def source_tile(index):
    """A 32x32 RGBA tile from the 10-column style-guide atlas."""
    row, col = divmod(index, 10)
    return src[row * T:(row + 1) * T, col * T:(col + 1) * T].copy()


# --- tileable noise ---------------------------------------------------------------------------

YY, XX = np.mgrid[0:T, 0:T]
U = (XX + 0.5) / T  # 0..1 left to right
V = (YY + 0.5) / T  # 0..1 top to bottom


def periodic_noise(rng, cells):
    """Smooth value noise in -0.5..0.5 that wraps every 32 pixels."""
    grid = rng.random((cells, cells))
    x, y = XX / T * cells, YY / T * cells
    x0, y0 = x.astype(int), y.astype(int)
    fx, fy = x - x0, y - y0
    x1, y1 = (x0 + 1) % cells, (y0 + 1) % cells
    fx, fy = fx * fx * (3 - 2 * fx), fy * fy * (3 - 2 * fy)
    top = grid[y0, x0] * (1 - fx) + grid[y0, x1] * fx
    bottom = grid[y1, x0] * (1 - fx) + grid[y1, x1] * fx
    return top * (1 - fy) + bottom * fy - 0.5


def edge_noise(seed, coarse=0.6, mid=0.3, fine=0.1):
    """Three octaves: big bays and points, mid wobble, pixel-scale ragged edge."""
    rng = np.random.default_rng(seed)
    return (periodic_noise(rng, 4) * coarse + periodic_noise(rng, 8) * mid
            + periodic_noise(rng, 16) * fine)


def interior_window():
    """0 at the tile border, 1 in the middle: variants only change pixels away from seams."""
    d = np.minimum.reduce([XX, YY, T - 1 - XX, T - 1 - YY]).astype(float)
    return np.clip((d - 3) / 6, 0, 1)


WINDOW = interior_window()


def field(mask, noise, amplitude):
    """Corner-blended coverage of the upper terrain, perturbed by noise. Above 0.5 = upper."""
    tl, tr, bl, br = (mask >> 0) & 1, (mask >> 1) & 1, (mask >> 2) & 1, (mask >> 3) & 1
    blend = tl * (1 - U) * (1 - V) + tr * U * (1 - V) + bl * (1 - U) * V + br * U * V
    return blend + noise * amplitude


BAYER4 = (np.array([[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]]) + 0.5) / 16
BAYER = BAYER4[YY % 4, XX % 4]


def paint(out, where, colour):
    out[where] = (*colour, 255)


# --- layer painters: each returns a 32x32 RGBA tile -------------------------------------------

def ground_over_water(g, fill):
    """Earth bank with a dark lip, a shadow on the water and a broken line of foam."""
    out = np.zeros((T, T, 4), np.uint8)
    land = g > 0.5
    out[land] = fill[land]
    paint(out, land & (g < 0.53), c("earth", 2))
    paint(out, land & (g >= 0.53) & (g < 0.555), c("earth", 3))
    paint(out, (g <= 0.5) & (g > 0.475), c("river", 1))
    foam = (g <= 0.455) & (g > 0.435) & (FOAM > 0.05)
    paint(out, foam, c("river", 4))
    return out


def grass_over_ground(g, fill):
    """Ragged grass fringe: dark root line on the grass, a soft shadow on the dirt."""
    out = np.zeros((T, T, 4), np.uint8)
    grass = g > 0.5
    out[grass] = fill[grass]
    paint(out, grass & (g < 0.52), c("reed", 2))
    paint(out, (g <= 0.5) & (g > 0.48), c("earth", 3))
    return out


def cobble_over_any(g, fill):
    """Set stones with a mortar-dark rim and a thin contact shadow."""
    out = np.zeros((T, T, 4), np.uint8)
    stone = g > 0.5
    out[stone] = fill[stone]
    paint(out, stone & (g < 0.525), c("ash", 2))
    paint(out, (g <= 0.5) & (g > 0.48), c("earth", 2))
    return out


def fissure(g):
    """Torn earth: a lit broken lip, a dark wall, black depth and a sick seam only at the bottom.

    Displaced clods and ash sit on the ground around it, so it reads as a wound in the
    ground rather than a glowing decal.
    """
    out = np.zeros((T, T, 4), np.uint8)
    paint(out, (g > 0.43) & (g <= 0.5) & (CLODS > 0.12), c("earth", 3))
    paint(out, (g > 0.43) & (g <= 0.5) & (CLODS > 0.2), c("earth", 5))
    paint(out, (g > 0.40) & (g <= 0.5) & (CLODS < -0.22), c("ash", 4))
    paint(out, (g > 0.5) & (g <= 0.53), c("earth", 5))
    paint(out, (g > 0.53) & (g <= 0.6), c("earth", 2))
    paint(out, (g > 0.6) & (g <= 0.68), c("earth", 1))
    paint(out, g > 0.68, c("ash", 0))
    paint(out, (g > 0.74) & (CLODS > 0.18), c("ash", 1))
    seam = g > 1.08
    paint(out, seam, c("blight", 0))
    paint(out, seam & (g > 1.14) & (GLINT > 0.26), c("blight", 2))
    return out


def scorch(g):
    """Burnt ground: ash clumps that thin out into scattered flecks, no outline."""
    out = np.zeros((T, T, 4), np.uint8)
    paint(out, (g > 0.4) & (g <= 0.5) & (CLODS > 0.12), c("ash", 3))
    body = (g > 0.5) & ~((g < 0.75) & (CLODS < -0.2))
    paint(out, body, c("ash", 3))
    paint(out, body & (g > 0.6) & (CLODS < 0.05), c("ash", 2))
    paint(out, body & (g > 0.8) & (CLODS < -0.1), c("ash", 1))
    paint(out, body & (GLINT > 0.3), c("ash", 5))
    return out


def haze(g, frame):
    """A dithered poison haze. Density follows the corners; the texture drifts each frame."""
    out = np.zeros((T, T, 4), np.uint8)
    drift = np.roll(np.roll(HAZE_NOISE, frame * 8, axis=1), frame * 4, axis=0)
    density = np.clip(g - 0.05, 0, 1) * (0.55 + drift)
    lit = density > BAYER * 1.4 + 0.05
    paint(out, lit, c("blight", 2))
    paint(out, lit & (density > 0.75), c("blight", 3))
    return out


# --- assemble ---------------------------------------------------------------------------------

BANK = edge_noise(11)
FRINGE = edge_noise(23, coarse=0.5, mid=0.35, fine=0.3)
SETTS = edge_noise(37, coarse=0.4, mid=0.3, fine=0.15)
CRACK = edge_noise(41, coarse=0.7, mid=0.4, fine=0.2)
BURN = edge_noise(53, coarse=0.6, mid=0.4, fine=0.3)
HAZE_NOISE = edge_noise(67, coarse=0.8, mid=0.3, fine=0.0)
FOAM = edge_noise(71, coarse=0.0, mid=0.4, fine=0.6)
CLODS = edge_noise(83, coarse=0.0, mid=0.5, fine=0.5)
GLINT = edge_noise(97, coarse=0.0, mid=0.2, fine=0.8)

atlas = np.zeros((ROWS * T, COLS * T, 4), np.uint8)


def put(index, tile):
    row, col = divmod(index, COLS)
    atlas[row * T:(row + 1) * T, col * T:(col + 1) * T] = tile


def dual_layer(first, painter, noise, amplitude, fill_variant=None, variant_seed=0):
    """Writes 16 transitions at `first` and 8 fills on the next row."""
    for mask in range(1, 16):
        put(first + mask, painter(field(mask, noise, amplitude)))
    for variant in range(8):
        if fill_variant is not None:
            put(first + COLS + variant, fill_variant(variant))
        else:
            # Fill variants differ only inside the window, so their borders match every transition.
            extra = edge_noise(variant_seed + variant)
            varied = noise * (1 - WINDOW) + extra * WINDOW
            put(first + COLS + variant, painter(field(15, varied, amplitude)))


for v in range(8):
    put(v, source_tile(60 + v))

ground_fill = source_tile(0)
grass_fill = source_tile(20)
cobble_fill = source_tile(40)
dual_layer(16, lambda g: ground_over_water(g, ground_fill), BANK, 0.55,
           fill_variant=lambda v: source_tile(0 + v))
dual_layer(48, lambda g: grass_over_ground(g, grass_fill), FRINGE, 0.6,
           fill_variant=lambda v: source_tile(20 + v))
dual_layer(80, lambda g: cobble_over_any(g, cobble_fill), SETTS, 0.45,
           fill_variant=lambda v: cobble_fill)
for v in range(8):
    put(112 + v, source_tile(80 + v))
put(127, np.full((T, T, 4), (*c("ash", 0), 255), np.uint8))
for old in range(100, 120):
    put(128 + old - 100, source_tile(old))
dual_layer(160, fissure, CRACK, 0.6, variant_seed=1000)
dual_layer(192, scorch, BURN, 0.6, variant_seed=2000)
for mask in range(1, 16):
    for frame in range(4):
        put(224 + mask * 4 + frame, haze(field(mask, edge_noise(5), 0.3), frame))

# --- checks -----------------------------------------------------------------------------------

alpha = atlas[..., 3]
assert set(np.unique(alpha)) <= {0, 255}, "alpha must be 0 or 255"
opaque = atlas[alpha == 255][:, :3]
palette = {tuple(colour) for colour in ALL}
off = {tuple(colour) for colour in np.unique(opaque, axis=0)} - palette
assert not off, f"off-palette colours: {sorted(off)}"

Image.fromarray(atlas).save(OUT, optimize=True)
print(f"wrote {os.path.normpath(OUT)} ({COLS * T}x{ROWS * T})")

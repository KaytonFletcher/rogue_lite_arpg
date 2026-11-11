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
    row 18  ground transitions, variant B   row 19  grass B   row 20  cobble B
    rows 21-22  grass decor (32)            rows 23-24  ground decor (32), cell grid

Fills are calm: a low-frequency ramp band pattern with dither only on band edges. Fill
variants 0-3 are plain, 4-5 carry a soft light patch, 6-7 a dark patch and clumps. Variant-B transitions share their
tile borders with variant A (they differ only inside the window), so either can sit next
to either. Decor sprites come from PixelLab (`tools/tileset/decor/`, cleaned with
`tools/art/clean.py`) and sit on their own sparse layer.

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
ROWS = 25
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
    """A mixed band rather than a line: dirt shows through near the grass lip, small tufts
    spill out onto the dirt, and the lip itself is a dithered half-step, not an outline."""
    out = np.zeros((T, T, 4), np.uint8)
    holes = (g > 0.5) & (g < 0.6) & (HOLES > 0.2)
    grass = (g > 0.5) & ~holes
    out[grass] = fill[grass]
    paint(out, grass & (g < 0.53) & (BAYER < 0.5), c("reed", 1))
    tufts = (g > 0.38) & (g <= 0.5) & (TUFTS > 0.2)
    paint(out, (g <= 0.5) & (g > 0.48) & ~tufts, c("earth", 3))
    paint(out, holes & (g < 0.53), c("earth", 3))
    paint(out, tufts, c("reed", 2))
    paint(out, tufts & (TUFTS > 0.3), c("reed", 3))
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


# --- calm fills ------------------------------------------------------------------------------

def calm_fill(ramp, base, dark, light, seed, variant):
    """A near-flat fill. The border ring is pure `base`, so variants and transitions meet
    without seams and no tile-sized pattern can form. Inside, each variant gets a few short
    dashes; variants 4-5 add one soft light patch, 6-7 a dark patch and a couple of lit clumps.
    Only the patch rims are dithered."""
    rng = np.random.default_rng(seed * 31 + variant)
    out = np.zeros((T, T, 4), np.uint8)
    out[...] = (*c(ramp, base), 255)
    inner = WINDOW > 0.99
    if variant >= 4:
        cy, cx = rng.integers(12, T - 12, size=2)
        r = rng.uniform(4.5, 6.5)
        d = np.hypot((YY - cy) * 1.3, XX - cx) / r
        step = light if variant < 6 else dark
        paint(out, inner & ((d < 0.75) | ((d < 1.05) & (BAYER > (d - 0.75) / 0.3))), c(ramp, step))
    for _ in range(rng.integers(3, 6)):
        y, x = rng.integers(9, T - 10, size=2)
        out[y, x:x + 2] = (*c(ramp, dark), 255)
        out[y - 1, x + 2] = (*c(ramp, light), 255)
    if variant >= 6:
        for _ in range(2):
            y, x = rng.integers(9, T - 11, size=2)
            for dy, dx, f in CLUMP:
                out[y + dy, x + dx] = (*c(ramp, f(base)), 255)
    return out


# A small lit clump: two light pixels over a dark base, NW-lit like everything else.
CLUMP = [(0, 1, lambda b: b + 2), (1, 0, lambda b: b + 1), (1, 1, lambda b: b + 1),
         (1, 2, lambda b: b - 1), (2, 1, lambda b: b - 1)]


def fill_variants(ramp, base, dark, light, seed):
    return [calm_fill(ramp, base, dark, light, seed, v) for v in range(8)]


# --- assemble ---------------------------------------------------------------------------------

BANK = edge_noise(11)
FRINGE = edge_noise(23, coarse=0.6, mid=0.3, fine=0.08)
SETTS = edge_noise(37, coarse=0.4, mid=0.3, fine=0.15)
CRACK = edge_noise(41, coarse=0.7, mid=0.4, fine=0.2)
BURN = edge_noise(53, coarse=0.6, mid=0.4, fine=0.3)
HAZE_NOISE = edge_noise(67, coarse=0.8, mid=0.3, fine=0.0)
FOAM = edge_noise(71, coarse=0.0, mid=0.4, fine=0.6)
CLODS = edge_noise(83, coarse=0.0, mid=0.5, fine=0.5)
GLINT = edge_noise(97, coarse=0.0, mid=0.2, fine=0.8)
HOLES = edge_noise(103, coarse=0.0, mid=0.6, fine=0.4)
TUFTS = edge_noise(109, coarse=0.0, mid=0.4, fine=0.6)

atlas = np.zeros((ROWS * T, COLS * T, 4), np.uint8)


def put(index, tile):
    row, col = divmod(index, COLS)
    atlas[row * T:(row + 1) * T, col * T:(col + 1) * T] = tile


def dual_layer(first, painter, noise, amplitude, fill_variant=None, variant_seed=0, alt_first=None):
    """Writes 16 transitions at `first` and 8 fills on the next row. With `alt_first`, a
    second set of transitions that keeps the tile borders and reshapes only the middle."""
    for mask in range(1, 16):
        put(first + mask, painter(field(mask, noise, amplitude)))
    if alt_first is not None:
        alt = noise * (1 - WINDOW) + edge_noise(variant_seed + 500) * WINDOW
        for mask in range(1, 16):
            put(alt_first + mask, painter(field(mask, alt, amplitude)))
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

GROUND_FILLS = fill_variants("earth", 4, 3, 5, seed=301)
GRASS_FILLS = fill_variants("reed", 2, 1, 3, seed=401)
ground_fill = GROUND_FILLS[0]
grass_fill = GRASS_FILLS[0]
cobble_fill = source_tile(40)
dual_layer(16, lambda g: ground_over_water(g, ground_fill), BANK, 0.55,
           fill_variant=lambda v: GROUND_FILLS[v], variant_seed=3000, alt_first=288)
dual_layer(48, lambda g: grass_over_ground(g, grass_fill), FRINGE, 0.6,
           fill_variant=lambda v: GRASS_FILLS[v], variant_seed=4000, alt_first=304)
dual_layer(80, lambda g: cobble_over_any(g, cobble_fill), SETTS, 0.45,
           fill_variant=lambda v: cobble_fill, variant_seed=5000, alt_first=320)
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

DECOR = os.path.join(HERE, "decor")
for kind, first in (("grass", 336), ("ground", 368)):
    for v in range(32):
        put(first + v, np.array(Image.open(os.path.join(DECOR, f"{kind}_{v:02d}.png")).convert("RGBA")))

# --- checks -----------------------------------------------------------------------------------

alpha = atlas[..., 3]
assert set(np.unique(alpha)) <= {0, 255}, "alpha must be 0 or 255"
opaque = atlas[alpha == 255][:, :3]
palette = {tuple(colour) for colour in ALL}
off = {tuple(colour) for colour in np.unique(opaque, axis=0)} - palette
assert not off, f"off-palette colours: {sorted(off)}"

Image.fromarray(atlas).save(OUT, optimize=True)
print(f"wrote {os.path.normpath(OUT)} ({COLS * T}x{ROWS * T})")

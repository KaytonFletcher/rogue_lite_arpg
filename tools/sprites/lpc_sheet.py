"""Assembles PixelLab character animations into the game's LPC sheet (13 columns x 21 rows of
64x64 frames, the layout `AnimationSheet::default()` reads), then cleans it onto the palette.

python3 tools/sprites/lpc_sheet.py DIR OUT.png --ramps steel,ash,earth,...

DIR holds the animation strips as PixelLab's `pixelart_workbench join ... --as rows` returns
them (an 18px label header, then one row per direction in the order south, east, north, west):

    rot.png      the four rotations, one row (join ... --as sheet --columns 4)
    walk.png     8 frames per direction            -> rows 8-11, the rotation + 8 frames
                 (skeleton-v3 walks keep the weapon; template walks tend to drop it)
    idle.png     4+ frames per direction           -> rows 12-15, --idle-frames (default 0,1,2)
    attack.png   9 frames (reference + 8)          -> rows 16-19
    death.png    7 frames, south only              -> row 20, 5 of them
    tend.png     7 frames (optional)               -> rows 0-3 (the spellcast slot)

Loose specks (sparkle effects) under 24 pixels that don't touch the body are dropped.
Strips drawn on a bigger canvas (v3 animations come back at 80x80) are cropped to 64x64 at the
offset where their first frame best overlaps the rotation, so the feet stay put. LPC rows run
up, left, down, right.
"""
import argparse
import os
import subprocess
import sys

import numpy as np
from PIL import Image

HEADER = 18
BACKGROUND = (31, 37, 44)
CELL = 64
COLUMNS, ROWS = 13, 21
DIRECTIONS = ["south", "east", "north", "west"]  # row order in the PixelLab strips
LPC_ORDER = ["north", "west", "south", "east"]  # LPC row order: up, left, down, right
HERE = os.path.dirname(__file__)


def strip(path, rows):
    """Frames of a joined strip as [row][frame] RGBA arrays, background made transparent."""
    sheet = np.array(Image.open(path).convert("RGBA"))[HEADER:]
    # The strip is flattened onto a dark slate, so soft glow pixels came back as near-slate
    # colours; they count as background too, or they glue sparkles to the body.
    distance = np.abs(sheet[..., :3].astype(int) - BACKGROUND).max(-1)
    background = distance <= 10
    sheet[background] = 0
    size = sheet.shape[0] // rows
    columns = sheet.shape[1] // size
    return [[sheet[r * size:(r + 1) * size, c * size:(c + 1) * size] for c in range(columns)]
            for r in range(rows)]


def drop_specks(frame, smallest=24):
    """Clears loose bits (sparkles, stray effects) that don't touch the body: any connected
    piece under `smallest` pixels apart from the largest one."""
    from scipy import ndimage

    solid = frame[..., 3] > 0
    labels, count = ndimage.label(solid, structure=np.ones((3, 3)))
    if count < 2:
        return frame
    sizes = ndimage.sum(solid, labels, range(1, count + 1))
    out = frame.copy()
    for index, size in enumerate(sizes, start=1):
        if size < smallest and size < sizes.max():
            out[labels == index] = 0
    return out


def fit(frames, rotation):
    """Crops a row of frames to 64x64 at the offset where frame 0 best matches the rotation."""
    size = frames[0].shape[0]
    if size == CELL:
        return frames
    target = rotation[..., 3] > 0
    best, offset = -1, (0, 0)
    for oy in range(size - CELL + 1):
        for ox in range(size - CELL + 1):
            window = frames[0][oy:oy + CELL, ox:ox + CELL, 3] > 0
            score = (window & target).sum() - (window ^ target).sum()
            if score > best:
                best, offset = score, (oy, ox)
    oy, ox = offset
    return [frame[oy:oy + CELL, ox:ox + CELL] for frame in frames]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("dir")
    parser.add_argument("out")
    parser.add_argument("--ramps", required=True)
    parser.add_argument("--death-frames", default="0,2,4,5,6")
    parser.add_argument("--cap", action="append", default=[], metavar="RAMP=STEP",
                        help="after cleaning, pull steps above STEP on RAMP down to STEP-1 "
                             "(e.g. blight=3 keeps the brightest blight for small seams)")
    parser.add_argument("--idle-frames", default="0,1,2",
                        help="which idle frames fill the 3 LPC idle columns")
    args = parser.parse_args()
    src = lambda name: os.path.join(args.dir, name)

    rotations = dict(zip(DIRECTIONS, strip(src("rot.png"), 1)[0]))
    sheet = np.zeros((ROWS * CELL, COLUMNS * CELL, 4), np.uint8)

    def put(row, column, frame):
        sheet[row * CELL:(row + 1) * CELL, column * CELL:(column + 1) * CELL] = frame

    def place(name, first_row, pick, lead_with_rotation=False):
        path = src(name)
        if not os.path.exists(path):
            print(f"skipping {name}: not found")
            return
        rows = dict(zip(DIRECTIONS, strip(path, 4)))
        for offset, direction in enumerate(LPC_ORDER):
            frames = fit(rows[direction], rotations[direction])
            frames = [drop_specks(frames[i]) for i in pick(len(frames))]
            if lead_with_rotation:
                frames = [rotations[direction]] + frames
            for column, frame in enumerate(frames[:COLUMNS]):
                put(first_row + offset, column, frame)

    place("tend.png", 0, lambda n: range(n))
    place("walk.png", 8, lambda n: range(n), lead_with_rotation=True)
    idle = [int(i) for i in args.idle_frames.split(",")]
    place("idle.png", 12, lambda n: [i for i in idle if i < n])
    place("attack.png", 16, lambda n: range(n))
    death = fit(strip(src("death.png"), 1)[0], rotations["south"])
    for column, index in enumerate(int(i) for i in args.death_frames.split(",")):
        put(20, column, death[index])

    raw = args.out + ".raw.png"
    Image.fromarray(sheet).save(raw)
    clean = os.path.join(HERE, "..", "art", "clean.py")
    subprocess.run([sys.executable, clean, raw, args.out, "--scale", "1", "--background", "none",
                    "--ramps", args.ramps], check=True)
    os.remove(raw)
    if args.cap:
        sys.path.insert(0, os.path.join(HERE, "..", "art"))
        from palette import PAL

        out = np.array(Image.open(args.out).convert("RGBA"))
        for cap in args.cap:
            ramp, step = cap.split("=")
            for high in range(int(step) + 1, len(PAL[ramp])):
                hit = (out[..., :3] == PAL[ramp][high]).all(-1) & (out[..., 3] > 0)
                out[hit, :3] = PAL[ramp][int(step) - 1]
        Image.fromarray(out).save(args.out)


if __name__ == "__main__":
    main()

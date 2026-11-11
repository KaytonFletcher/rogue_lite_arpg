"""Animated preview of an LPC character sheet: every state in all four directions, as a GIF.

python3 tools/sprites/sheet_preview.py SHEET.png OUT.gif [--scale 3]

States follow `AnimationSheet::default()`: idle (rows 12-15, 3 frames), walk (8-11, 9), attack
(16-19, 9), tend (0-3, 7) and dying (row 20, 5). Rows run up, left, down, right; the preview
shows them down, right, up, left like the game's direction order.
"""
import argparse

from PIL import Image, ImageDraw

CELL = 64
STATES = [("idle", 12, 3), ("walk", 8, 9), ("attack", 16, 9), ("tend", 0, 7), ("dying", 20, 5)]
SHOW = [2, 3, 0, 1]  # down, right, up, left as offsets from the state's first row
BACKGROUND = (92, 84, 78, 255)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("sheet")
    parser.add_argument("out")
    parser.add_argument("--scale", type=int, default=3)
    args = parser.parse_args()
    sheet = Image.open(args.sheet).convert("RGBA")
    longest = max(count for _, _, count in STATES)
    frames = []
    for tick in range(longest * 2):
        canvas = Image.new("RGBA", (4 * CELL, len(STATES) * (CELL + 10)), BACKGROUND)
        draw = ImageDraw.Draw(canvas)
        for row, (name, first, count) in enumerate(STATES):
            y = row * (CELL + 10)
            draw.text((2, y), name, fill=(255, 240, 200, 255))
            for column, offset in enumerate(SHOW):
                sheet_row = first if name == "dying" else first + offset
                if name == "dying" and column:
                    continue
                frame = tick % count
                cell = sheet.crop((frame * CELL, sheet_row * CELL,
                                   (frame + 1) * CELL, (sheet_row + 1) * CELL))
                canvas.alpha_composite(cell, (column * CELL, y + 10))
        frames.append(canvas.resize((canvas.width * args.scale, canvas.height * args.scale),
                                    Image.NEAREST).convert("P", palette=Image.ADAPTIVE))
    frames[0].save(args.out, save_all=True, append_images=frames[1:], duration=120, loop=0,
                   disposal=2)
    print("wrote", args.out)


if __name__ == "__main__":
    main()

"""Draws the People (River-caller, Fire-keeper, Elder-Seer) as 64x64 front-facing frames.

python3 tools/sprites/people.py OUT.png

Each sprite is built from flat shapes, back to front. A shape is one palette ramp at one base
step; lighting comes from the north-west and only moves pixels along that ramp: a 1px lit band
on a shape's north-west edge, a 1-2px shade band on its south-east edge. The outer silhouette
then gets a selective outline (each edge pixel drops to the darkest steps of its own ramp).
Large flat clusters, few colours per material, no noise: detail goes into shapes instead.

Silhouette rules from the style guide's audit:
- River-caller: tall and lean; paddle across the back (blade over the left shoulder), a
  coiled cord on the right hip and a topknot. The diagonal paddle is her read.
- Fire-keeper: short, very broad, wide stance; ladle held up high, a pot on the left hip.
- Elder-Seer: bent forward (head low between hunched shoulders), cloak flaring to a shaggy
  hem wider than the shoulders, and a leaning staff with river stones hanging off it.
"""
import os
import sys

import numpy as np
from PIL import Image, ImageDraw

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "art"))
from palette import ALL, PAL  # noqa: E402

N = 64
FEET = 58


class Sprite:
    def __init__(self):
        self.ramp = np.full((N, N), "", dtype=object)  # ramp name per pixel
        self.step = np.zeros((N, N), int)  # step per pixel
        self.part = np.zeros((N, N), int)  # part id per pixel, 0 = empty
        self.parts = 0
        self.flat = np.zeros((N, N), bool)  # pixels that skip lighting (eyes, marks)

    # --- shapes -----------------------------------------------------------------------------
    def mask(self, draw_fn):
        img = Image.new("1", (N, N), 0)
        draw_fn(ImageDraw.Draw(img))
        return np.array(img, bool)

    def shape(self, mask, ramp, step, light=True, shade=True):
        """Paints a shape over what is there, with north-west light and south-east shade."""
        self.parts += 1
        pid = self.parts
        self.ramp[mask] = ramp
        self.part[mask] = pid
        top = len(PAL[ramp]) - 1
        steps = np.full((N, N), step)

        def inside(dy, dx):
            shifted = np.zeros_like(mask)
            ys = slice(max(dy, 0), N + min(dy, 0))
            yd = slice(max(-dy, 0), N + min(-dy, 0))
            xs = slice(max(dx, 0), N + min(dx, 0))
            xd = slice(max(-dx, 0), N + min(-dx, 0))
            shifted[yd, xd] = mask[ys, xs]
            return shifted

        if light:
            lit = mask & (~inside(-1, 0) | ~inside(0, -1))
            steps[lit] = np.minimum(step + 1, top)
        if shade:
            dark1 = mask & (~inside(1, 0) | ~inside(0, 1))
            dark2 = mask & (~inside(2, 0) | ~inside(0, 2)) & ~dark1
            steps[dark2] = np.maximum(step - 1, 0)
            steps[dark1] = np.maximum(step - 1, 0)
            if light:
                steps[mask & dark1 & (~inside(-1, 0) | ~inside(0, -1))] = step
        self.step[mask] = steps[mask]
        self.flat[mask] = False
        return mask

    def poly(self, points, ramp, step, **kw):
        return self.shape(self.mask(lambda d: d.polygon(points, fill=1)), ramp, step, **kw)

    def rect(self, x0, y0, x1, y1, ramp, step, **kw):
        return self.shape(self.mask(lambda d: d.rectangle((x0, y0, x1, y1), fill=1)), ramp, step, **kw)

    def ellipse(self, x0, y0, x1, y1, ramp, step, **kw):
        return self.shape(self.mask(lambda d: d.ellipse((x0, y0, x1, y1), fill=1)), ramp, step, **kw)

    def line(self, points, width, ramp, step, **kw):
        return self.shape(self.mask(lambda d: d.line(points, fill=1, width=width)), ramp, step, **kw)

    def px(self, x, y, ramp, step):
        """A single hand-placed detail pixel (eye, bead, stitch); not relit."""
        self.ramp[y, x] = ramp
        self.step[y, x] = step
        self.flat[y, x] = True
        if self.part[y, x] == 0:
            self.parts += 1
            self.part[y, x] = self.parts

    # --- finish -----------------------------------------------------------------------------
    def render(self):
        filled = self.part > 0
        out = np.zeros((N, N, 4), np.uint8)
        steps = self.step.copy()
        # Selective outline: silhouette edge pixels drop to the bottom of their own ramp,
        # one step lighter on the lit (north/west) side so the outline isn't flat black.
        pad = np.pad(filled, 1)
        n, s = ~pad[:-2, 1:-1], ~pad[2:, 1:-1]
        w, e = ~pad[1:-1, :-2], ~pad[1:-1, 2:]
        edge = filled & (n | s | w | e)
        steps[edge & (s | e)] = 0
        steps[edge & ~(s | e)] = 1
        # Inner part borders: the part in front darkens one step on its south-east side.
        part = self.part
        below = np.zeros_like(part)
        below[:-1] = part[1:]
        right = np.zeros_like(part)
        right[:, :-1] = part[:, 1:]
        seam = filled & ~edge & (((below != part) & (below > 0)) | ((right != part) & (right > 0)))
        seam &= ~self.flat
        steps[seam] = np.maximum(steps[seam] - 1, 0)
        for y in range(N):
            for x in range(N):
                if filled[y, x]:
                    ramp = PAL[self.ramp[y, x]]
                    out[y, x, :3] = ramp[min(steps[y, x], len(ramp) - 1)]
                    out[y, x, 3] = 255
        return out


def legs(s, cx, gap, top, ramp, step, width=3):
    s.rect(cx - gap - width, top, cx - gap - 1, FEET, ramp, step)
    s.rect(cx + gap, top, cx + gap + width - 1, FEET, ramp, step)


def face(s, cx, top, width=8, height=8, skin=("rust", 3)):
    """A face with two eye pixels and a hint of mouth, centred on cx."""
    x0 = cx - width // 2
    s.rect(x0, top, x0 + width - 1, top + height - 1, *skin)
    s.px(cx - 2, top + 3, "cedar", 0)
    s.px(cx + 1, top + 3, "cedar", 0)
    s.px(cx, top + 6, "rust", 1)


def river_caller():
    s = Sprite()
    cx = 31
    # Paddle across the back: shaft from right hip up past the left shoulder, blade on top.
    s.line([(cx + 9, 44), (cx - 11, 8)], 2, "cedar", 3)
    s.poly([(cx - 9, 13), (cx - 15, 2), (cx - 12, 0), (cx - 6, 11)], "cedar", 4)
    legs(s, cx, 1, 44, "rust", 3)
    s.rect(cx - 4, FEET - 1, cx - 2, FEET, "rust", 1)  # feet
    s.rect(cx + 1, FEET - 1, cx + 3, FEET, "rust", 1)
    # River-dyed wrap to the knee, a little flared.
    s.poly([(cx - 6, 24), (cx + 5, 24), (cx + 7, 45), (cx - 8, 45)], "river", 3)
    s.rect(cx - 6, 33, cx + 6, 34, "river", 5)  # woven band
    for y in range(30, 45):  # river-dyed diagonal weave, 2px bands every 5px
        for x in range(cx - 7, cx + 7):
            if (x + y) % 5 == 0 and s.ramp[y, x] == "river" and y not in (33, 34):
                s.px(x, y, "river", 4)
    # Reed cape over the shoulders, wider than the body with a fringe.
    s.poly([(cx - 7, 21), (cx + 6, 21), (cx + 9, 28), (cx - 10, 28)], "reed", 3)
    for x in range(cx - 9, cx + 9, 2):
        s.px(x, 29, "reed", 2)
    # Arms: left free at the side, right holding the paddle shaft.
    s.rect(cx - 9, 29, cx - 8, 39, "rust", 3)
    s.rect(cx + 7, 29, cx + 8, 36, "rust", 3)
    # Coiled canoe cord on the right hip: a ring that sticks out of the silhouette.
    s.ellipse(cx + 6, 37, cx + 12, 43, "reed", 4)
    s.ellipse(cx + 8, 39, cx + 10, 41, "river", 3, light=False, shade=False)
    s.part[40, cx + 9] = 0  # hole in the coil
    # Neck, head, hair and the topknot with its river-stone bead.
    s.rect(cx - 1, 19, cx + 1, 21, "rust", 2)
    face(s, cx, 11)
    s.poly([(cx - 5, 10), (cx + 4, 10), (cx + 4, 12), (cx - 5, 12)], "cedar", 1)
    s.rect(cx - 5, 11, cx - 5, 18, "cedar", 1)  # hair falls down one side
    s.px(cx - 5, 18, "river", 5)  # bead
    s.ellipse(cx - 2, 6, cx + 1, 9, "cedar", 1)
    s.px(cx, 6, "river", 5)
    return s


def fire_keeper():
    s = Sprite()
    cx = 32
    # Ladle held up high in the right hand: shaft and bowl above head height.
    s.line([(cx + 14, 30), (cx + 16, 9)], 2, "cedar", 3)
    s.ellipse(cx + 13, 4, cx + 20, 10, "steel", 2)
    s.rect(cx + 14, 5, cx + 19, 6, "ash", 1, light=False, shade=False)
    # Wide stance.
    s.poly([(cx - 9, 44), (cx - 4, 44), (cx - 5, FEET), (cx - 11, FEET)], "rust", 2)
    s.poly([(cx + 3, 44), (cx + 8, 44), (cx + 10, FEET), (cx + 4, FEET)], "rust", 2)
    # Broad body in a cedar wrap.
    s.poly([(cx - 11, 22), (cx + 10, 22), (cx + 12, 46), (cx - 13, 46)], "cedar", 2)
    # Ochre apron with a hem stripe.
    s.poly([(cx - 7, 27), (cx + 6, 27), (cx + 8, 46), (cx - 9, 46)], "gold", 2)
    s.rect(cx - 8, 43, cx + 7, 44, "rust", 3, light=False)
    s.rect(cx - 4, 34, cx + 2, 38, "gold", 1)  # pocket
    for x in range(cx - 4, cx + 3, 2):
        s.px(x, 34, "gold", 3)  # stitching
    s.px(cx + 4, 30, "ash", 2)  # soot smudges
    s.px(cx + 5, 31, "ash", 2)
    s.px(cx - 6, 40, "ash", 3)
    # Arms: right raised to the ladle, left down to the pot. Soot to the elbows.
    s.poly([(cx + 9, 23), (cx + 13, 23), (cx + 16, 31), (cx + 12, 32)], "rust", 3)
    s.rect(cx + 12, 27, cx + 16, 32, "ash", 2)
    s.poly([(cx - 12, 23), (cx - 9, 23), (cx - 10, 36), (cx - 14, 36)], "rust", 3)
    s.rect(cx - 14, 32, cx - 10, 36, "ash", 2)
    # Clay pot hanging at the left hip.
    s.ellipse(cx - 20, 35, cx - 11, 44, "rust", 3)
    s.rect(cx - 18, 34, cx - 13, 35, "rust", 1)
    # Head: low and broad, with a wrapped headcloth.
    s.rect(cx - 2, 20, cx + 1, 22, "rust", 2)
    face(s, cx, 12, width=10, height=9)
    s.poly([(cx - 6, 9), (cx + 5, 9), (cx + 5, 13), (cx - 6, 13)], "earth", 6)
    s.rect(cx + 4, 13, cx + 6, 15, "earth", 5)  # knot
    return s


def elder_seer():
    s = Sprite()
    cx = 32
    # Shaggy reed cloak: hunched shoulders up around a low head, flaring to a wide hem.
    s.poly([(cx - 7, 18), (cx + 7, 18), (cx + 10, 26), (cx + 13, FEET - 1), (cx - 13, FEET - 1),
            (cx - 9, 26)], "reed", 2)
    # Shredded hem: alternating tatters break the bottom edge.
    for x in range(cx - 13, cx + 14):
        if x % 3 == 0:
            s.px(x, FEET, "reed", 1)
        if x % 3 == 1:
            s.part[FEET - 1, x] = 0
    # Vertical shag strands so the cloak reads as fibre, not noise.
    for x in range(cx - 9, cx + 10, 4):
        s.line([(x, 28), (x + (1 if x > cx else -1), FEET - 3)], 1, "reed", 1, light=False,
               shade=False)
        s.line([(x + 1, 30), (x + 1 + (1 if x > cx else -1), FEET - 6)], 1, "reed", 3,
               light=False, shade=False)
    # Leaning staff in front, out past the cloak, with river stones hanging off the top.
    s.line([(cx - 9, FEET), (cx - 17, 9)], 2, "cedar", 3)
    s.rect(cx - 18, 8, cx - 15, 11, "reed", 4)  # lashing
    for i, (x, y) in enumerate([(cx - 20, 13), (cx - 16, 15), (cx - 19, 18)]):
        s.px(x, y - 1, "reed", 3)
        s.ellipse(x - 2, y, x + 1, y + 3, "river", 3 + i % 2)
    # Gnarled hand on the staff.
    s.ellipse(cx - 15, 27, cx - 11, 31, "rust", 3)
    # Head bowed low, between the shoulders, under a hood.
    s.ellipse(cx - 7, 13, cx + 6, 26, "reed", 3)
    face(s, cx, 18, width=7, height=7)
    s.rect(cx - 4, 16, cx + 3, 17, "ash", 6)  # white hair under the hood
    s.rect(cx - 3, 24, cx + 2, 25, "ash", 6)  # beard tip
    return s


def main():
    out = sys.argv[1] if len(sys.argv) > 1 else "/tmp/people.png"
    sheet = np.zeros((N, N * 3, 4), np.uint8)
    for i, build in enumerate((river_caller, fire_keeper, elder_seer)):
        sheet[:, i * N:(i + 1) * N] = build().render()
    palette = {tuple(colour) for colour in ALL}
    opaque = sheet[sheet[..., 3] == 255][:, :3]
    off = {tuple(c) for c in np.unique(opaque, axis=0)} - palette
    assert not off, f"off-palette: {off}"
    Image.fromarray(sheet).save(out)
    print("wrote", out)


if __name__ == "__main__":
    main()

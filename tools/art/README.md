# Art harness

Scripts that keep every asset on the style guide (`art/style_guide_mvp.md` in the project):
one 81-colour palette, alpha 0 or 255, native resolution, clean clusters. They need only
Python 3 with `numpy` and `pillow`.

| Script | What it does |
|---|---|
| `palette.py` | The palette by ramp, `c("earth", 3)` lookups, and sRGB to OKLab. |
| `audit.py` | Checks art. Hard rules fail the run (palette, alpha, upscaling); soft metrics are reported (speckle, clusters, dither). `--silhouettes out.png` draws every frame as a solid shape. |
| `clean.py` | Turns outside art into on-style art: removes a flat background, makes alpha binary, finds and undoes upscaling, snaps to the palette (optionally only some ramps), removes speckle, and can add the selective outline. |
| `../tileset/build_atlas.py` | Builds `assets/tilesets/tile_atlas.png`, the dual-grid terrain, mark and haze atlas. `preview.py` renders a test map with it. |
| `../sprites/people.py` | Draws the People from flat palette shapes with ramp lighting. |

CI runs `audit.py` on the generated atlas, so off-palette or soft-edged pixels can't land.

## Bringing in generated art (PixelLab, Retro Diffusion, anything else)

1. **Generate** at the final size (characters 64×64 frames, tiles 32×32), on a transparent
   or flat background. Give the generator a style-guide plate as its reference image.
2. **Clean**: `python3 tools/art/clean.py raw.png clean.png --ramps <ramps>`.
   Pick ramps by meaning, so colours can't leak into the wrong role:
   - People: `rust,cedar,gold,reed,river,ash,earth`
   - Kingdom: `steel,banner,fire,ash,earth,rust`
   - Unreturned: `blight,bruise,flesh,ash,earth`
   - Terrain: `ash,earth,rust,reed,river`
   Add `--outline` if the generator drew no outline. For grids that drift (non-integer
   scale, common with general image models), run Pixel Snapper first
   (`pip install pixel-snapper`).
3. **Audit**: `python3 tools/art/audit.py clean.png --frame 64 --silhouettes sil.png`.
   Each role should read from its silhouette alone.
4. **Hand pass** in Aseprite (indexed mode, `palette.gpl`): faces, hands, anything the
   cleanup simplified.
5. **Terrain is different**: generate only fill textures, then let `build_atlas.py` cut the
   edges. Generated Wang tilesets use their own layout, which won't match `tiling.rs`.

## PixelLab from Claude

PixelLab is connected as an MCP server in Claude (Tier 1 account). The full recipe, the
character ids, the commands that rebuild each sheet, and the cloud-session workarounds
(no upload or download URLs from the sandbox: push inputs to the `art/pixellab-inputs`
branch and use raw GitHub URLs; read results inline with `pixelart_workbench join`) are in
the project doc `art/art_direction_state.md`. The rules are in `art/style_guide_mvp.md`.

| Script | What it does |
|---|---|
| `recompose.py` | Swaps refined props into a scene: finds each old prop by its ramp+step pattern and draws the new one with the same per-pixel lighting offset. |
| `refined/`, `scenes/` | PixelLab-refined props (cleaned) and the two recomposed 640x360 scenes. |
| `../sprites/lpc_sheet.py` | PixelLab animation strips (`../sprites/pixellab/<name>/`) to the 13x21 LPC sheet `AnimationSheet::default()` reads, cleaned onto the palette. |
| `../sprites/sheet_preview.py` | A GIF of every state of an LPC sheet in four directions. |
| `../tileset/decor/` | The 64 decor sprites (PixelLab, cleaned) placed by the decor layer. |

Generated PNGs under `tools/art`, `tools/tileset`, `tools/sprites`, the tile atlas and the
three character sheets are kept out of Git LFS (`.gitattributes`), because cloud sessions
can't upload LFS objects. Add an exemption before committing a generated PNG elsewhere.

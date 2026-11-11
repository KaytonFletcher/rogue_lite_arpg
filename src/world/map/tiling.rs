//! Picks tile atlas slots for the map's render layers.
//!
//! Terrain is drawn on a dual grid: every rendered tile sits on the corner where four cells
//! meet and picks one of 16 pieces from which of those cells hold its terrain (the corner
//! mask: bit0 north-west, bit1 north-east, bit2 south-west, bit3 south-east). Terrains stack
//! by rank (water < ground < grass < cobblestone): each [`TerrainLayer`] draws wherever a
//! cell's rank reaches its own, so any two terrains meet with a rounded edge and inside
//! corners come for free, from 16 pieces per terrain instead of a 47-tile blob set.
//!
//! Walls, wood floors and dead zones stay on the cell grid, drawn above the terrain. Ground
//! marks (fissures, scorch) and haze are more dual-grid layers over their own cell masks.
//!
//! Each terrain has two sets of transition pieces that share their tile borders, so a long
//! edge alternates between them without seams. A sparse decor layer (tufts, pebbles, twigs)
//! sits on the cell grid inside open grass and ground, clustered into patches by smooth value
//! noise rather than sprinkled evenly.
//!
//! `tools/tileset/build_atlas.py` builds the atlas these offsets point into, and
//! `tools/tileset/preview.py` renders a test map with the same rules. Pure grid math with no
//! backend detail; spawning stays in `spawn.rs`.

use bevy::prelude::{IVec2, UVec2};

use super::tiles::{GroundMark, TileGrid, TileKind};

/// Tiles per atlas row.
const ATLAS_COLUMNS: u32 = 16;
/// Fill variants per dual layer, stored on the row after its 16 transition pieces.
const FILL_VARIANTS: u16 = 8;
/// First of the water fill variants (the base layer).
const WATER_FILLS: u32 = 0;
/// First of the wood floor variants.
const WOOD_FILLS: u32 = 112;
/// Opaque tile drawn over dead-zone cells.
const VOID: u32 = 127;
/// First wall piece; the offsets below index from here.
const WALL_FIRST: u32 = 128;
/// Wall caps for vertical runs, keyed by flank mask (bit0 exposed west, bit1 exposed east):
/// blank, west cap, east cap, both caps.
const WALL_TOP_MID: [u32; 4] = [13, 11, 10, 12];
/// Wall caps for run tops, same flank mask: top, top-left, top-right, all.
const WALL_TOP_EDGE: [u32; 4] = [0, 1, 2, 3];
/// Wall face pieces keyed by side mask (bit0 wall west, bit1 wall east): isolated, east end,
/// west end. Mask 3 (both sides) rolls a straight variant instead.
const WALL_FACE_ENDS: [u32; 3] = [19, 18, 17];
/// Straight face variants with an outlined bottom row.
const WALL_FACE_STRAIGHTS: [u32; 3] = [14, 15, 16];
/// First haze piece. Piece `mask` holds [`HAZE_FRAMES`] consecutive animation frames.
const HAZE_FIRST: u32 = 224;
/// Animation frames per haze piece.
pub(crate) const HAZE_FRAMES: u32 = 4;
/// First grass decor sprite; [`DECOR_VARIANTS`] of them follow.
const GRASS_DECOR: u32 = 336;
/// First ground decor sprite.
const GROUND_DECOR: u32 = 368;
/// Decor sprites per terrain.
const DECOR_VARIANTS: u16 = 32;
/// Decor patch lattice spacing, in cells: patches are a few cells across.
const PATCH_CELLS: i32 = 4;
/// Patch noise below this leaves the ground bare; above it decor thickens.
const PATCH_THRESHOLD: f32 = 0.4;
/// How fast decor thickens past the threshold.
const PATCH_RAMP: f32 = 2.5;
/// Chance of decor in the thickest part of a patch.
const PATCH_MAX_DENSITY: f32 = 0.7;
/// Corner mask with all four cells covered: a fill tile rather than a transition piece.
const FULL: u8 = 15;

/// Variant salts, so layers sharing a corner don't roll in lockstep.
const WATER_SALT: u32 = 0;
const WOOD_SALT: u32 = 7;
const WALL_SALT: u32 = 8;
/// Added to a layer's salt when rolling transition set A or B.
const ALT_SALT: u32 = 32;
const PATCH_SALT: u32 = 61;
const DECOR_ROLL_SALT: u32 = 62;
const DECOR_PICK_SALT: u32 = 63;

/// The four cells around a dual-grid corner and their mask bits. The dual tile at `corner`
/// covers cells `corner - (1, 1)` through `corner`; `+y` is north.
const CORNERS: [(IVec2, u8); 4] = [
    (IVec2::new(-1, 0), 1),  // north-west
    (IVec2::new(0, 0), 2),   // north-east
    (IVec2::new(-1, -1), 4), // south-west
    (IVec2::new(0, -1), 8),  // south-east
];

/// A terrain drawn on the dual grid over the water base, in draw order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TerrainLayer {
    Ground,
    Grass,
    Cobblestone,
}

impl TerrainLayer {
    /// Every terrain layer, bottom to top.
    pub(crate) const ALL: [Self; 3] = [Self::Ground, Self::Grass, Self::Cobblestone];

    /// Lowest cell rank this layer covers; see [`terrain_rank`].
    fn rank(self) -> u8 {
        match self {
            Self::Ground => 1,
            Self::Grass => 2,
            Self::Cobblestone => 3,
        }
    }

    /// First atlas slot of the layer's 16 transition pieces.
    fn first(self) -> u32 {
        match self {
            Self::Ground => 16,
            Self::Grass => 48,
            Self::Cobblestone => 80,
        }
    }

    /// First atlas slot of the layer's second transition set (same borders, new middles).
    fn alt_first(self) -> u32 {
        match self {
            Self::Ground => 288,
            Self::Grass => 304,
            Self::Cobblestone => 320,
        }
    }

    /// Entity name for the layer's tilemap.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Ground => "Ground Layer",
            Self::Grass => "Grass Layer",
            Self::Cobblestone => "Cobblestone Layer",
        }
    }
}

/// How high a cell's terrain stacks. Walls, wood and dead zones sit on ground, so ground is
/// drawn under their cell-grid pieces and rims any water next to them.
fn terrain_rank(tile_kind: TileKind) -> u8 {
    match tile_kind {
        TileKind::Water => 0,
        TileKind::Ground | TileKind::Wall | TileKind::Wood | TileKind::DeadZone => 1,
        TileKind::Grass => 2,
        TileKind::Cobblestone => 3,
    }
}

/// First atlas slot of a ground mark's 16 transition pieces.
fn mark_first(mark: GroundMark) -> u32 {
    match mark {
        GroundMark::Fissure => 160,
        GroundMark::Scorch => 192,
    }
}

/// Size of the dual grid for a map: one more corner than cells on each axis.
pub(crate) fn dual_size(map_size: UVec2) -> UVec2 {
    map_size + UVec2::ONE
}

/// Returns which of the four cells around `corner` satisfy `covers`. Cells past the map edge
/// read as the nearest edge cell, so layers run off the map without drawing a border.
fn corner_mask<T: Copy>(grid: &TileGrid<T>, corner: IVec2, covers: impl Fn(T) -> bool) -> u8 {
    let last_cell = grid.size().as_ivec2() - IVec2::ONE;
    CORNERS.iter().fold(0, |mask, &(offset, bit)| {
        let cell = (corner + offset).clamp(IVec2::ZERO, last_cell);
        let value = grid.get(cell).expect("a clamped cell lies inside the grid");
        if covers(value) { mask | bit } else { mask }
    })
}

/// Picks the piece for a corner mask: nothing when no corner is covered, a seeded fill
/// variant when all four are, and the matching transition piece otherwise. With `alt_first`,
/// transitions roll between the two sets.
fn dual_piece(
    first: u32,
    alt_first: Option<u32>,
    mask: u8,
    seed: u64,
    salt: u32,
    corner: IVec2,
) -> Option<u32> {
    match mask {
        0 => None,
        FULL => {
            Some(first + ATLAS_COLUMNS + u32::from(tile_variant(seed, salt, corner, FILL_VARIANTS)))
        }
        _ => {
            let set = match alt_first {
                Some(alt) if tile_variant(seed, salt + ALT_SALT, corner, 2) == 1 => alt,
                _ => first,
            };
            Some(set + u32::from(mask))
        }
    }
}

/// Water fill for the base layer, only where a neighbouring cell is water (every other
/// corner is covered by an opaque ground fill).
pub(crate) fn water_piece(tiles: &TileGrid<TileKind>, seed: u64, corner: IVec2) -> Option<u32> {
    let mask = corner_mask(tiles, corner, |kind| terrain_rank(kind) == 0);
    (mask != 0)
        .then(|| WATER_FILLS + u32::from(tile_variant(seed, WATER_SALT, corner, FILL_VARIANTS)))
}

/// Piece for one terrain layer at a dual-grid corner, or `None` where it draws nothing.
pub(crate) fn terrain_piece(
    tiles: &TileGrid<TileKind>,
    seed: u64,
    layer: TerrainLayer,
    corner: IVec2,
) -> Option<u32> {
    let mask = corner_mask(tiles, corner, |kind| terrain_rank(kind) >= layer.rank());
    dual_piece(
        layer.first(),
        Some(layer.alt_first()),
        mask,
        seed,
        u32::from(layer.rank()),
        corner,
    )
}

/// Piece for one ground mark at a dual-grid corner, or `None` where it draws nothing.
pub(crate) fn mark_piece(
    marks: &TileGrid<Option<GroundMark>>,
    seed: u64,
    mark: GroundMark,
    corner: IVec2,
) -> Option<u32> {
    let covered = corner_mask(marks, corner, |cell| cell == Some(mark));
    dual_piece(
        mark_first(mark),
        None,
        covered,
        seed,
        16 + mark as u32,
        corner,
    )
}

/// First animation frame of the haze piece at a dual-grid corner, or `None` without haze.
pub(crate) fn haze_piece(haze: &TileGrid<bool>, corner: IVec2) -> Option<u32> {
    let mask = corner_mask(haze, corner, |hazy| hazy);
    (mask != 0).then(|| HAZE_FIRST + u32::from(mask) * HAZE_FRAMES)
}

/// Decor sprite for a cell, or `None` for bare ground. Only cells of open grass or ground
/// (all four neighbours the same kind) can hold decor, so it never sits on a transition edge.
/// Smooth patch noise decides how thick decor is around the cell, then a per-cell roll
/// decides whether this one gets a sprite.
pub(crate) fn decor_piece(tiles: &TileGrid<TileKind>, seed: u64, tile: IVec2) -> Option<u32> {
    let kind = tiles.get(tile)?;
    let first = match kind {
        TileKind::Grass => GRASS_DECOR,
        TileKind::Ground => GROUND_DECOR,
        _ => return None,
    };
    let open = [IVec2::X, IVec2::NEG_X, IVec2::Y, IVec2::NEG_Y]
        .into_iter()
        .all(|offset| tiles.get(tile + offset) == Some(kind));
    if !open {
        return None;
    }
    let density = ((patch_noise(seed, tile) - PATCH_THRESHOLD) * PATCH_RAMP).clamp(0.0, 1.0)
        * PATCH_MAX_DENSITY;
    let roll = f32::from(tile_variant(seed, DECOR_ROLL_SALT, tile, 1000)) / 1000.0;
    (roll < density)
        .then(|| first + u32::from(tile_variant(seed, DECOR_PICK_SALT, tile, DECOR_VARIANTS)))
}

/// Smooth value noise in `0..=1` on a [`PATCH_CELLS`] lattice: neighbouring cells read
/// nearly the same value, so decor gathers in patches with bare ground between them.
fn patch_noise(seed: u64, tile: IVec2) -> f32 {
    let lattice = IVec2::new(
        tile.x.div_euclid(PATCH_CELLS),
        tile.y.div_euclid(PATCH_CELLS),
    );
    let smooth = |offset: i32| {
        let t = offset as f32 / PATCH_CELLS as f32;
        t * t * (3.0 - 2.0 * t)
    };
    let fx = smooth(tile.x.rem_euclid(PATCH_CELLS));
    let fy = smooth(tile.y.rem_euclid(PATCH_CELLS));
    let value =
        |offset: IVec2| f32::from(tile_variant(seed, PATCH_SALT, lattice + offset, 1000)) / 999.0;
    let south = value(IVec2::ZERO) * (1.0 - fx) + value(IVec2::X) * fx;
    let north = value(IVec2::Y) * (1.0 - fx) + value(IVec2::ONE) * fx;
    south * (1.0 - fy) + north * fy
}

/// Cell-grid piece drawn above the terrain: walls, wood floors and the dead-zone void.
pub(crate) fn cell_piece(tiles: &TileGrid<TileKind>, seed: u64, tile: IVec2) -> Option<u32> {
    match tiles.get(tile)? {
        TileKind::Wall => Some(WALL_FIRST + wall_piece(tiles, seed, tile)),
        TileKind::Wood => {
            Some(WOOD_FILLS + u32::from(tile_variant(seed, WOOD_SALT, tile, FILL_VARIANTS)))
        }
        TileKind::DeadZone => Some(VOID),
        TileKind::Ground | TileKind::Grass | TileKind::Water | TileKind::Cobblestone => None,
    }
}

/// Picks a side-view wall piece from wall neighbours (`+y` is north). Off-map reads as wall,
/// closing the map border ring.
///
/// A wall with more mass to the south shows its cap; north-south runs show only their caps
/// (the camera never sees their faces); otherwise the brick face with a cap lip shows. Caps
/// border only exposed edges: run tops always lip the north side, flanks only where no wall
/// neighbour sits.
fn wall_piece(tiles: &TileGrid<TileKind>, seed: u64, tile: IVec2) -> u32 {
    let wall = |offset: IVec2| {
        tiles
            .get(tile + offset)
            .is_none_or(|kind| kind == TileKind::Wall)
    };
    let north_wall = wall(IVec2::new(0, 1));
    let south_wall = wall(IVec2::new(0, -1));
    let east_wall = wall(IVec2::new(1, 0));
    let west_wall = wall(IVec2::new(-1, 0));
    // Flank mask: bit0 exposed west, bit1 exposed east.
    let flank = usize::from(!west_wall) | (usize::from(!east_wall) << 1);
    if north_wall && south_wall {
        // Cap continues vertically: border only the exposed flanks.
        WALL_TOP_MID[flank]
    } else if south_wall {
        // Run top (no wall north): the north lip always shows; flanks only when exposed.
        WALL_TOP_EDGE[flank]
    } else {
        // Only outlined-base pieces show: a face implies no wall below.
        let side = usize::from(west_wall) | (usize::from(east_wall) << 1);
        if side == 3 {
            let pick = tile_variant(seed, WALL_SALT, tile, WALL_FACE_STRAIGHTS.len() as u16);
            WALL_FACE_STRAIGHTS[usize::from(pick)]
        } else {
            WALL_FACE_ENDS[side]
        }
    }
}

/// Returns a deterministic variant in `0..variant_count` for a tile or corner.
///
/// Identical `(seed, salt, tile)` triples always map to the same variant; adjacent tiles get
/// decorrelated values. The splitmix64 finalizer is what spreads the low bits of
/// `tile.x`/`tile.y` so neighbouring tiles don't all pick the same variant. Each layer passes
/// its own salt, so layers sharing a corner don't roll in lockstep.
fn tile_variant(seed: u64, salt: u32, tile: IVec2, variant_count: u16) -> u16 {
    debug_assert!(
        variant_count > 0,
        "the hash is reduced modulo variant_count, which divides by zero"
    );

    let mut hash = seed
        ^ u64::from(salt).wrapping_mul(0x9E37_79B1_85EB_CA87)
        ^ u64::from(tile.x as u32).wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
        ^ u64::from(tile.y as u32).wrapping_mul(0x1656_67B1_9E37_79F9);

    hash ^= hash >> 30;
    hash = hash.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    hash ^= hash >> 27;
    hash = hash.wrapping_mul(0x94D0_49BB_1331_11EB);
    hash ^= hash >> 31;

    (hash % u64::from(variant_count)) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a grid from text rows, top row north. `~` water, `.` ground, `,` grass.
    fn grid(rows: &[&str]) -> TileGrid<TileKind> {
        let size = UVec2::new(rows[0].len() as u32, rows.len() as u32);
        let mut tiles = TileGrid::new(size, TileKind::Ground);
        for (row_index, row) in rows.iter().enumerate() {
            for (x, glyph) in row.chars().enumerate() {
                let y = rows.len() - 1 - row_index;
                let kind = TileKind::from_glyph(glyph).expect("test glyphs are tile glyphs");
                tiles.set(IVec2::new(x as i32, y as i32), kind);
            }
        }
        tiles
    }

    /// The corner mask a grass transition piece draws, from either transition set.
    fn grass_mask(piece: u32) -> u32 {
        let first = TerrainLayer::Grass.first();
        let alt = TerrainLayer::Grass.alt_first();
        if (alt..alt + 16).contains(&piece) {
            piece - alt
        } else {
            assert!(
                (first..first + 16).contains(&piece),
                "{piece} is not a grass transition piece"
            );
            piece - first
        }
    }

    #[test]
    fn transitions_use_both_sets() {
        // A long straight grass edge: corners along it should not all pick the same set.
        let tiles = grid(&[",,,,,,,,,,,,,,,,", "................"]);
        let alt = TerrainLayer::Grass.alt_first();
        let from_alt = (1..16)
            .filter_map(|x| terrain_piece(&tiles, 9, TerrainLayer::Grass, IVec2::new(x, 1)))
            .filter(|&piece| piece >= alt)
            .count();
        assert!(
            (1..15).contains(&from_alt),
            "a long edge mixes both transition sets, {from_alt} of 15 from set B"
        );
    }

    #[test]
    fn decor_stays_in_open_ground_and_grass() {
        let size = UVec2::new(48, 48);
        let mut tiles = TileGrid::new(size, TileKind::Grass);
        for x in 0..48 {
            tiles.set(IVec2::new(x, 24), TileKind::Water);
        }
        let mut placed = 0;
        for x in 0..48 {
            for y in 0..48 {
                let tile = IVec2::new(x, y);
                if let Some(piece) = decor_piece(&tiles, 5, tile) {
                    placed += 1;
                    assert!(
                        (GRASS_DECOR..GRASS_DECOR + u32::from(DECOR_VARIANTS)).contains(&piece),
                        "grass cells take grass decor, got {piece}"
                    );
                    assert!(
                        (y - 24).abs() > 1,
                        "decor never sits next to the water edge at {tile}"
                    );
                }
            }
        }
        assert!(placed > 0, "a large open field gets some decor");
        assert!(placed < 48 * 48 / 3, "decor stays sparse, placed {placed}");
    }

    #[test]
    fn patch_noise_is_smooth_and_bounded() {
        for x in -8..8 {
            for y in -8..8 {
                let here = patch_noise(7, IVec2::new(x, y));
                let next = patch_noise(7, IVec2::new(x + 1, y));
                assert!(
                    (0.0..=1.0).contains(&here),
                    "patch noise {here} is out of range"
                );
                assert!(
                    (here - next).abs() < 0.5,
                    "neighbouring cells read close values, {here} vs {next}"
                );
            }
        }
    }

    #[test]
    fn corner_mask_bits_follow_compass_corners() {
        // Grass only in the north-west cell of a 2x2 map: the middle corner sees bit0.
        let tiles = grid(&[",.", ".."]);
        let mask = corner_mask(&tiles, IVec2::new(1, 1), |kind| kind == TileKind::Grass);
        assert_eq!(mask, 1, "only the north-west cell is grass");
        let tiles = grid(&["..", ".,"]);
        let mask = corner_mask(&tiles, IVec2::new(1, 1), |kind| kind == TileKind::Grass);
        assert_eq!(mask, 8, "only the south-east cell is grass");
    }

    #[test]
    fn single_cell_gets_four_rounded_corner_pieces() {
        let tiles = grid(&["...", ".,.", "..."]);
        let mut masks: Vec<_> = (0..4)
            .flat_map(|y| (0..4).map(move |x| IVec2::new(x, y)))
            .filter_map(|corner| terrain_piece(&tiles, 0, TerrainLayer::Grass, corner))
            .map(grass_mask)
            .collect();
        masks.sort_unstable();
        assert_eq!(
            masks,
            [1, 2, 4, 8],
            "a lone cell is drawn by its four corner pieces"
        );
    }

    #[test]
    fn inside_corner_uses_three_corner_piece() {
        // An L of grass: the corner inside the L sees three grass cells.
        let tiles = grid(&[",,", ",."]);
        let piece = terrain_piece(&tiles, 0, TerrainLayer::Grass, IVec2::new(1, 1))
            .expect("three grass cells draw a transition");
        assert_eq!(
            grass_mask(piece),
            7,
            "north-west, north-east and south-west are grass"
        );
    }

    #[test]
    fn full_corners_pick_a_fill_variant() {
        let tiles = grid(&[",,", ",,"]);
        let piece = terrain_piece(&tiles, 3, TerrainLayer::Grass, IVec2::new(1, 1))
            .expect("grass covers every corner");
        let fills = TerrainLayer::Grass.first() + ATLAS_COLUMNS;
        assert!(
            (fills..fills + u32::from(FILL_VARIANTS)).contains(&piece),
            "a covered corner draws a fill, got {piece}"
        );
    }

    #[test]
    fn grass_also_draws_ground_under_it() {
        // Grass ranks above ground, so the ground layer covers grass cells too and rims water.
        let tiles = grid(&["~,"]);
        let ground = terrain_piece(&tiles, 0, TerrainLayer::Ground, IVec2::new(1, 1))
            .expect("the east cells draw ground");
        let first = TerrainLayer::Ground.first();
        let alt = TerrainLayer::Ground.alt_first();
        assert!(
            ground == first + 2 + 8 || ground == alt + 2 + 8,
            "the east cells (grass) are ground or higher, got {ground}"
        );
        assert!(
            water_piece(&tiles, 0, IVec2::new(1, 1)).is_some(),
            "water shows under the bank"
        );
        assert_eq!(
            water_piece(&tiles, 0, IVec2::new(2, 1)),
            None,
            "no water drawn where no cell is water"
        );
    }

    #[test]
    fn map_edge_extends_the_edge_cells() {
        let tiles = grid(&[",,", ",,"]);
        let piece = terrain_piece(&tiles, 0, TerrainLayer::Grass, IVec2::ZERO);
        assert!(
            piece.is_some_and(|slot| slot >= TerrainLayer::Grass.first() + ATLAS_COLUMNS),
            "the outer corner reads the edge cells, so grass fills it"
        );
    }

    #[test]
    fn haze_frames_follow_the_mask() {
        let mut haze = TileGrid::new(UVec2::new(2, 2), false);
        haze.set(IVec2::new(0, 1), true);
        assert_eq!(
            haze_piece(&haze, IVec2::new(1, 1)),
            Some(HAZE_FIRST + HAZE_FRAMES),
            "mask 1 starts one block of frames in"
        );
        assert_eq!(
            haze_piece(&haze, IVec2::new(2, 0)),
            None,
            "no haze, no tile"
        );
    }

    #[test]
    fn variants_are_deterministic_and_in_range() {
        for x in -4..4 {
            for y in -4..4 {
                let tile = IVec2::new(x, y);
                let variant = tile_variant(42, 3, tile, FILL_VARIANTS);
                assert!(variant < FILL_VARIANTS, "variant {variant} is out of range");
                assert_eq!(
                    variant,
                    tile_variant(42, 3, tile, FILL_VARIANTS),
                    "same inputs roll the same variant"
                );
            }
        }
    }
}

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
/// Corner mask with all four cells covered: a fill tile rather than a transition piece.
const FULL: u8 = 15;

/// Variant salts, so layers sharing a corner don't roll in lockstep.
const WATER_SALT: u32 = 0;
const WOOD_SALT: u32 = 7;
const WALL_SALT: u32 = 8;

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
/// variant when all four are, and the matching transition piece otherwise.
fn dual_piece(first: u32, mask: u8, seed: u64, salt: u32, corner: IVec2) -> Option<u32> {
    match mask {
        0 => None,
        FULL => {
            Some(first + ATLAS_COLUMNS + u32::from(tile_variant(seed, salt, corner, FILL_VARIANTS)))
        }
        _ => Some(first + u32::from(mask)),
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
    dual_piece(layer.first(), mask, seed, u32::from(layer.rank()), corner)
}

/// Piece for one ground mark at a dual-grid corner, or `None` where it draws nothing.
pub(crate) fn mark_piece(
    marks: &TileGrid<Option<GroundMark>>,
    seed: u64,
    mark: GroundMark,
    corner: IVec2,
) -> Option<u32> {
    let covered = corner_mask(marks, corner, |cell| cell == Some(mark));
    dual_piece(mark_first(mark), covered, seed, 16 + mark as u32, corner)
}

/// First animation frame of the haze piece at a dual-grid corner, or `None` without haze.
pub(crate) fn haze_piece(haze: &TileGrid<bool>, corner: IVec2) -> Option<u32> {
    let mask = corner_mask(haze, corner, |hazy| hazy);
    (mask != 0).then(|| HAZE_FIRST + u32::from(mask) * HAZE_FRAMES)
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
        let mut pieces: Vec<_> = (0..4)
            .flat_map(|y| (0..4).map(move |x| IVec2::new(x, y)))
            .filter_map(|corner| terrain_piece(&tiles, 0, TerrainLayer::Grass, corner))
            .collect();
        let first = TerrainLayer::Grass.first();
        let expected = [first + 1, first + 2, first + 4, first + 8];
        pieces.sort_unstable();
        assert_eq!(
            pieces, expected,
            "a lone cell is drawn by its four corner pieces"
        );
    }

    #[test]
    fn inside_corner_uses_three_corner_piece() {
        // An L of grass: the corner inside the L sees three grass cells.
        let tiles = grid(&[",,", ",."]);
        let piece = terrain_piece(&tiles, 0, TerrainLayer::Grass, IVec2::new(1, 1));
        assert_eq!(
            piece,
            Some(TerrainLayer::Grass.first() + 7),
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
        let ground = terrain_piece(&tiles, 0, TerrainLayer::Ground, IVec2::new(1, 1));
        assert_eq!(
            ground,
            Some(TerrainLayer::Ground.first() + 2 + 8),
            "the east cells (grass) are ground or higher"
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

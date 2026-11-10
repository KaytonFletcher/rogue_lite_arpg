use std::collections::BTreeMap;

use avian2d::prelude::{Collider, CollisionLayers, RigidBody};
use bevy::prelude::*;
use bevy_ecs_tilemap::prelude::*;

use super::blueprint::{MapBlueprint, PlacementKind};
use super::tiles::{GroundMark, TileGrid, TileKind, TileRect};
use super::tiling::{
    HAZE_FRAMES, TerrainLayer, cell_piece, dual_size, haze_piece, mark_piece, terrain_piece,
    water_piece,
};
use crate::prelude::*;

pub(super) fn plugin(app: &mut App) {
    app.add_plugins(TilemapPlugin);
    app.init_resource::<TileToWorld>();
    app.add_systems(
        OnEnter(AppState::SpawnMap),
        (
            set_tile_to_world_map_size,
            populate_map_tiles,
            spawn_colliders,
            spawn_placements,
            transition_to_playing,
        )
            .chain(),
    );
}

fn transition_to_playing(mut next_app_state: ResMut<NextState<AppState>>) {
    next_app_state.set(AppState::Playing);
}

fn set_tile_to_world_map_size(
    blueprint: Single<&MapBlueprint>,
    mut coordinates: ResMut<TileToWorld>,
) {
    coordinates.map_size = blueprint.map_size();
}

fn populate_map_tiles(
    mut commands: Commands,
    blueprint: Single<(&MapBlueprint, Entity)>,
    coordinates: Res<TileToWorld>,
    tiles: Res<TileAssets>,
) {
    let (blueprint, map) = blueprint.into_inner();
    spawn_tiles(&mut commands, map, blueprint, &tiles, &coordinates);
}

fn spawn_tiles(
    commands: &mut Commands,
    map: Entity,
    blueprint: &MapBlueprint,
    tiles: &TileAssets,
    coordinates: &TileToWorld,
) {
    let _span = info_span!("map::spawn_tiles").entered();
    let mut layers = MapLayers {
        commands,
        map,
        texture: &tiles.atlas,
        tile_size: coordinates.tile_size,
    };
    let map_size = blueprint.map_size();
    let dual_size = dual_size(map_size);
    let seed = blueprint.seed;
    let terrain = &blueprint.tiles;

    // Dual-grid layers sit on cell corners, so their grid is one larger and, centred like the
    // cell grid, lands half a tile down-left of it. Draw order rises with z.
    layers.spawn("Water Layer", dual_size, 0.0, None, |corner| {
        water_piece(terrain, seed, corner)
    });
    for (index, layer) in TerrainLayer::ALL.into_iter().enumerate() {
        let z = 0.1 * (index + 1) as f32;
        layers.spawn(layer.name(), dual_size, z, None, |corner| {
            terrain_piece(terrain, seed, layer, corner)
        });
    }
    for (index, mark) in GroundMark::ALL.into_iter().enumerate() {
        let z = 0.5 + 0.05 * index as f32;
        layers.spawn("Mark Layer", dual_size, z, None, |corner| {
            mark_piece(&blueprint.marks, seed, mark, corner)
        });
    }
    layers.spawn("Cell Layer", map_size, 0.7, None, |tile| {
        cell_piece(terrain, seed, tile)
    });
    let haze_animation = Some(HAZE_FRAMES);
    layers.spawn(
        "Haze Layer",
        dual_size,
        ZLayer::Haze.z(),
        haze_animation,
        |corner| haze_piece(&blueprint.haze, corner),
    );
}

/// Haze drift speed, in animation frames per second.
const HAZE_FRAMES_PER_SECOND: f32 = 3.0;

/// Spawns the map's tilemap layers as children of the map entity.
struct MapLayers<'a, 'w, 's> {
    commands: &'a mut Commands<'w, 's>,
    map: Entity,
    texture: &'a Handle<Image>,
    tile_size: TilemapTileSize,
}

impl MapLayers<'_, '_, '_> {
    /// Spawns one layer of `size` tiles at height `z`, asking `piece` for each tile's atlas
    /// slot. With `frames`, each tile cycles through that many consecutive slots. A layer
    /// with no tiles is skipped.
    fn spawn(
        &mut self,
        name: &'static str,
        size: UVec2,
        z: f32,
        frames: Option<u32>,
        mut piece: impl FnMut(IVec2) -> Option<u32>,
    ) {
        let tilemap_entity = self.commands.spawn_empty().id();
        let mut storage = TileStorage::empty(size.into());
        let mut placed = false;

        for x in 0..size.x {
            for y in 0..size.y {
                let Some(texture_index) = piece(IVec2::new(x as i32, y as i32)) else {
                    continue;
                };
                let position = TilePos { x, y };
                let mut tile = self.commands.spawn((
                    Name::new("Tile"),
                    TileBundle {
                        position,
                        tilemap_id: TilemapId(tilemap_entity),
                        texture_index: TileTextureIndex(texture_index),
                        ..default()
                    },
                    ChildOf(tilemap_entity),
                ));
                if let Some(frames) = frames {
                    tile.insert(AnimatedTile {
                        start: texture_index,
                        end: texture_index + frames,
                        speed: HAZE_FRAMES_PER_SECOND,
                    });
                }
                storage.set(&position, tile.id());
                placed = true;
            }
        }

        if !placed {
            self.commands.entity(tilemap_entity).despawn();
            return;
        }
        self.commands.entity(tilemap_entity).insert((
            Name::new(name),
            TilemapBundle {
                grid_size: self.tile_size.into(),
                size: size.into(),
                storage,
                map_type: TilemapType::Square,
                texture: TilemapTexture::Single(self.texture.clone()),
                tile_size: self.tile_size,
                anchor: TilemapAnchor::Center,
                transform: Transform::from_xyz(0.0, 0.0, z),
                ..default()
            },
            ChildOf(self.map),
            // The one exception to "only NextMap::set despawns map pieces": game over clears
            // the tiles so its screen shows no half-empty map. The map entity itself stays
            // until the next NextMap::set.
            DespawnOnExit::<AppState>(AppState::Playing),
        ));
    }
}

// ---------------------------------------------------------------------------
// Colliders
// ---------------------------------------------------------------------------

/// Collider marker for map geometry that is removed when gameplay ends.
#[derive(Component, Default, Clone)]
#[require(DespawnOnExit::<AppState>(AppState::Playing))]
pub struct MapCollider;

fn spawn_colliders(
    mut commands: Commands,
    blueprint: Single<&MapBlueprint>,
    coordinates: Res<TileToWorld>,
) {
    for rect in derive_collider_rects(&blueprint.tiles) {
        let (position, size) = coordinates.tile_rectangle_to_world_rectangle(rect);

        commands.spawn_scene(bsn! {
            MapCollider
            ~RigidBody::Static
            ~Collider::rectangle(size.x, size.y)
            Transform::from_xyz(position.x, position.y, {ZLayer::OnGround.z()})
            ~CollisionLayers::new(
                GameCollisionLayer::HighObstacle,
                GameCollisionLayer::HIGH_OBSTACLE_FILTERS,
            )
        });
    }
}

fn derive_collider_rects(tiles: &TileGrid<TileKind>) -> Vec<TileRect> {
    let _span = info_span!("map::derive_collider_rects").entered();
    let map_width = i32::try_from(tiles.size().x).expect("map width fits in i32");
    let map_height = i32::try_from(tiles.size().y).expect("map height fits in i32");

    let mut completed_collider_rects = Vec::new();
    let mut previous_runs: BTreeMap<(i32, i32), TileRect> = BTreeMap::new();

    for y in 0..map_height {
        let runs = blocking_runs(tiles, y, map_width);
        let mut continued_runs = BTreeMap::new();

        for (start_x, end_x) in runs {
            let tree_key = (start_x, end_x);

            let rect = if let Some(mut rect) = previous_runs.remove(&tree_key) {
                rect.size.y += 1;
                rect
            } else {
                TileRect {
                    min: IVec2::new(start_x, y),
                    size: UVec2::new((end_x - start_x) as u32, 1),
                }
            };

            continued_runs.insert(tree_key, rect);
        }

        completed_collider_rects.extend(previous_runs.into_values());
        previous_runs = continued_runs;
    }

    completed_collider_rects.extend(previous_runs.into_values());
    completed_collider_rects
}

fn blocking_runs(tiles: &TileGrid<TileKind>, y: i32, map_width: i32) -> Vec<(i32, i32)> {
    let mut runs = Vec::new();
    let mut x = 0;

    while x < map_width {
        while x < map_width
            && !tiles
                .get(IVec2::new(x, y))
                .is_some_and(TileKind::blocks_ground)
        {
            x += 1;
        }

        let start = x;

        while x < map_width
            && tiles
                .get(IVec2::new(x, y))
                .is_some_and(TileKind::blocks_ground)
        {
            x += 1;
        }

        if start < x {
            runs.push((start, x));
        }
    }

    runs
}

// ---------------------------------------------------------------------------
// Placements
// ---------------------------------------------------------------------------

fn spawn_placements(
    mut commands: Commands,
    sprites: Res<SpriteAssets>,
    blueprint: Single<&MapBlueprint>,
    coordinates: Res<TileToWorld>,
    mut player: Query<&mut Transform, With<Player>>,
) -> Result {
    let mut player_transform = player.single_mut()?;
    let mut enemies = Vec::new();
    let mut npcs = Vec::new();
    let mut chests = Vec::new();

    for placement in &blueprint.placements {
        let world_spawn_position = coordinates.tile_center_to_world_position(placement.tile);

        match &placement.kind {
            PlacementKind::PlayerStart => {
                player_transform.translation =
                    world_spawn_position.extend(player_transform.translation.z);
            }
            PlacementKind::Enemy(enemy_kind) => {
                enemies.push(EnemySpawn {
                    position: world_spawn_position,
                    kind: *enemy_kind,
                });
            }
            PlacementKind::Npc(npc_kind) => {
                npcs.push(NpcSpawn {
                    position: world_spawn_position,
                    kind: *npc_kind,
                });
            }
            PlacementKind::Chest => chests.push(world_spawn_position),
            PlacementKind::Exit => {
                commands.spawn_scene(portal(&sprites, world_spawn_position));
            }
        }
    }

    if !enemies.is_empty() {
        commands.trigger(SpawnEnemies(enemies));
    }
    if !npcs.is_empty() {
        commands.trigger(SpawnNpcs(npcs));
    }
    if !chests.is_empty() {
        commands.trigger(SpawnChests(chests));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tile <-> world conversion
// ---------------------------------------------------------------------------

/// Largest map width or height, in tiles, that recipe validation accepts.
pub const MAX_MAP_TILES: u32 = 256;
/// Width and height of a square map tile, in world units.
pub const TILE_SIZE: u32 = 32;

/// Converts tile coordinates and dimensions to world-space positions.
#[derive(Resource, Clone, Copy)]
pub struct TileToWorld {
    /// Width and height of one tile, in world units.
    pub tile_size: TilemapTileSize,
    /// Map width and height, in tiles.
    pub map_size: UVec2,
}

impl Default for TileToWorld {
    fn default() -> Self {
        Self {
            tile_size: TilemapTileSize::new(TILE_SIZE as f32, TILE_SIZE as f32),
            map_size: UVec2::ZERO,
        }
    }
}

impl TileToWorld {
    fn units_per_tile(&self) -> Vec2 {
        self.tile_size.into()
    }

    fn bottom_left_offset(&self) -> Vec2 {
        self.units_per_tile() * self.map_size.as_vec2() * -0.5
    }

    /// Returns the tile containing a world-space position, or `None` outside the map.
    pub fn world_to_tile_position(&self, world_position: Vec2) -> Option<IVec2> {
        let map_position_in_units = world_position - self.bottom_left_offset();
        let tile_index = (map_position_in_units / self.units_per_tile())
            .floor()
            .as_ivec2();

        (tile_index.cmpge(IVec2::ZERO).all() && tile_index.cmplt(self.map_size.as_ivec2()).all())
            .then_some(tile_index)
    }

    /// Converts tile-space coordinates to world-space coordinates.
    pub fn tile_space_to_world_position(&self, tile_coordinates: Vec2) -> Vec2 {
        tile_coordinates * self.units_per_tile() + self.bottom_left_offset()
    }

    /// Converts a tile index to the center of that tile in world space.
    pub fn tile_center_to_world_position(&self, tile_index: IVec2) -> Vec2 {
        self.tile_space_to_world_position(tile_index.as_vec2() + Vec2::splat(0.5))
    }

    /// Converts a tile index to its lower-left corner in world space.
    pub fn tile_corner_to_world_position(&self, tile_index: IVec2) -> Vec2 {
        self.tile_space_to_world_position(tile_index.as_vec2())
    }

    /// Returns a tile rectangle's world-space center and dimensions.
    pub fn tile_rectangle_to_world_rectangle(&self, tile_rectangle: TileRect) -> (Vec2, Vec2) {
        let rectangle_size_in_units = tile_rectangle.size.as_vec2() * self.units_per_tile();
        let rectangle_minimum_world_position =
            self.tile_corner_to_world_position(tile_rectangle.min);
        let rectangle_center = rectangle_minimum_world_position + rectangle_size_in_units / 2.0;
        (rectangle_center, rectangle_size_in_units)
    }
}

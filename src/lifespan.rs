use bevy::prelude::*;

use crate::prelude::*;

pub(super) fn plugin(app: &mut App) {
    app.add_systems(
        Update,
        (despawn_expired_entities, remove_scheduled_components)
            .in_set(InGameSystems::DespawnEntities),
    );
}

/// Represents an entity that will be despawned after time elapsed.
#[derive(Component, Deref, DerefMut, Clone)]
pub struct Lifespan(pub Timer);

impl Lifespan {
    pub fn new(duration_secs: f32) -> Self {
        Self(Timer::from_seconds(duration_secs, TimerMode::Once))
    }
}

impl Default for Lifespan {
    fn default() -> Self {
        Self::new(2.0)
    }
}

fn despawn_expired_entities(
    mut commands: Commands,
    mut duration_query: Query<(Entity, &mut Lifespan)>,
    time: Res<Time>,
) {
    for (entity, mut duration) in &mut duration_query {
        duration.tick(time.delta());

        if duration.is_finished() {
            commands.entity(entity).despawn();
        }
    }
}

/// Observer that despawns every entity with component `C` whenever `T` is triggered.
///
/// Uses the batched [`Commands::despawn_all`] rather than one despawn command per entity.
pub fn despawn_all<T: Event, C: Component>(_: On<T>, mut commands: Commands) {
    commands.despawn_all::<With<C>>();
}

/// A pending component removal, held as a child so several can wait on one entity at once.
#[derive(Component)]
struct ScheduledRemoval {
    timer: Timer,
    remover: Option<Box<dyn FnOnce(&mut EntityCommands) + Send + Sync>>,
}

fn remove_scheduled_components(
    mut commands: Commands,
    mut removal_query: Query<(Entity, &ChildOf, &mut ScheduledRemoval)>,
    time: Res<Time>,
) {
    for (removal_entity, child_of, mut removal) in &mut removal_query {
        removal.timer.tick(time.delta());
        if removal.timer.is_finished()
            && let Some(remover) = removal.remover.take()
        {
            let mut entity_commands = commands.entity(child_of.parent());
            remover(&mut entity_commands);
            commands.entity(removal_entity).despawn();
        }
    }
}

pub fn schedule_component_removal<C: Component>(
    commands: &mut Commands,
    entity: Entity,
    seconds: f32,
) {
    let remover = Box::new(|entity_commands: &mut EntityCommands| {
        entity_commands.remove::<C>();
    });

    commands.entity(entity).with_child(ScheduledRemoval {
        timer: Timer::from_seconds(seconds, TimerMode::Once),
        remover: Some(remover),
    });
}

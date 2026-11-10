//! What a brain knows: vision tracks a target and aggro picks one.

use std::f32::consts::FRAC_PI_4;

use avian2d::prelude::{RayCaster, RayHits};
use bevy::prelude::*;

use super::BrainSystems;
use crate::prelude::*;

pub(super) fn plugin(app: &mut App) {
    app.add_observer(on_damage_aggro)
        .add_observer(on_targeting_inserted)
        .add_observer(on_targeting_removed)
        .add_systems(
            Update,
            (
                (update_target_info, is_target_in_sight)
                    .chain()
                    .in_set(BrainSystems::Perception),
                (should_target_watched, should_stop_targeting).in_set(BrainSystems::Targeting),
                update_aim_position.in_set(BrainSystems::Aim),
            ),
        );
}

/// How wide an entity's vision cone is.
#[derive(Component)]
#[component(immutable)]
#[require(Vision, TargetInfo)]
pub(crate) struct VisionCone {
    /// In radians; `PI / 4.0` is a 90° cone.
    pub half_angle: f32,
}

impl Default for VisionCone {
    fn default() -> Self {
        Self {
            half_angle: FRAC_PI_4,
        }
    }
}

/// Seconds a newly targeted entity is kept without line of sight while the
/// sight ray turns to it.
const SIGHT_GRACE: f32 = 0.2;

/// The entity this one fights, added on sight, when damaged, or when the brain
/// picks another hostile.
#[derive(Component)]
#[relationship(relationship_target = TargetedBy)]
pub(crate) struct Targeting(pub Entity);

/// Inverse of [`Targeting`].
#[derive(Component)]
#[relationship_target(relationship = Targeting)]
pub(crate) struct TargetedBy(Vec<Entity>);

/// The entity this one watches for, to target once sighted.
#[derive(Component, FromTemplate, Clone)]
#[relationship(relationship_target = WatchedBy)]
pub(in crate::character) struct Watching(pub Entity);

/// Inverse of [`Watching`].
#[derive(Component)]
#[relationship_target(relationship = Watching)]
pub(in crate::character) struct WatchedBy(Vec<Entity>);

/// Keeps `Targeting` without sight until removed. Damage aggro schedules its removal.
#[derive(Component)]
pub(crate) struct TargetLock;

// ---------------------
// VISION + PERCEPTION
// ---------------------

/// Aims vision at the target while targeting, otherwise along the facing direction.
fn update_aim_position(
    mut character_query: Query<
        (&mut Vision, &TargetInfo, Has<Targeting>, &FacingDirection),
        Without<Player>,
    >,
) {
    character_query.par_iter_mut().for_each(
        |(mut vision, target_info, has_target, facing_direction)| {
            vision.aim_direction = if has_target {
                target_info.direction
            } else {
                facing_direction.to_vec2()
            };
        },
    );
}

/// Tracks direction and distance to the target (or the watched entity without
/// one) and points the sight ray there.
fn update_target_info(
    mut npc_query: Query<
        (
            &mut TargetInfo,
            &mut RayCaster,
            &Transform,
            &Watching,
            Option<&Targeting>,
        ),
        Without<Player>,
    >,
    target_query: Query<(&Transform, Option<&Kinematics>)>,
) {
    npc_query.par_iter_mut().for_each(
        |(mut target_info, mut ray_caster, transform, watching, targeting)| {
            let target_entity = targeting.map_or(watching.0, |t| t.0);

            if let Ok((target_transform, target_kinematics)) = target_query.get(target_entity) {
                let target_position = target_transform.translation.xy();
                let position = transform.translation.xy();

                let target_direction = (target_position - position).normalize_or_zero();

                target_info.direction = target_direction;
                target_info.distance = target_position.distance(position);
                target_info.position = target_position;
                target_info.velocity = target_kinematics.map_or(Vec2::ZERO, Kinematics::velocity);

                ray_caster.direction = Dir2::new(target_direction).unwrap_or(Dir2::X);
            }
        },
    );
}

/// Updates line of sight (the ray hit the target or one of its children) and
/// whether the target is inside the vision cone.
fn is_target_in_sight(
    mut npc_query: Query<(
        &mut TargetInfo,
        &RayHits,
        &Vision,
        &VisionCone,
        &Watching,
        Option<&Targeting>,
    )>,
    target_query: Query<Option<&Children>, Or<(With<WatchedBy>, With<TargetedBy>)>>,
) {
    npc_query.par_iter_mut().for_each(
        |(mut target_info, ray_hits, vision, vision_cone, watching, targeting)| {
            let vision_cone_dot = vision_cone.half_angle.cos();
            target_info.in_vision_cone =
                target_info.direction.dot(vision.aim_direction) > vision_cone_dot;

            let target_entity = targeting.map_or(watching.0, |t| t.0);

            let target_children = target_query.get(target_entity).ok().flatten();
            target_info.line_of_sight = ray_hits.iter().any(|hit| {
                hit.entity == target_entity
                    || target_children.is_some_and(|children| children.contains(&hit.entity))
            });
        },
    );
}

// ---------------------
// AGGRO / TARGETING
// ---------------------

/// Targets the attacker on any hit, ignoring sight, or the watched entity for
/// attacker-less damage.
fn on_damage_aggro(
    damage_dealt: On<DamageDealt>,
    mut commands: Commands,
    target_query: Query<&Watching>,
) {
    let damaged_entity = damage_dealt.entity;

    let attacker_or_watched = damage_dealt.attacker.or_else(|| {
        target_query
            .get(damaged_entity)
            .ok()
            .map(|watching| watching.0)
    });

    if let Some(target) = attacker_or_watched {
        debug!("I've been hit: {damaged_entity}, attacking: {target}");
        commands
            .entity(damaged_entity)
            .insert((TargetLock, Targeting(target)));

        schedule_component_removal::<TargetLock>(&mut commands, damaged_entity, 6.0);
    }
}

/// Starts the targeting clock when a new entity is targeted; re-inserts of the
/// same target from repeated hits keep it. When the sight ray was aimed at
/// another entity, also turns the tracking, aim and ray to the new target at
/// once, so motions and actions this frame aim at it. Its line of sight reads
/// false until the turned ray reports, which [`SIGHT_GRACE`] covers.
fn on_targeting_inserted(
    inserted: On<Insert<Targeting>>,
    time: Res<Time<Virtual>>,
    mut trackers: Query<(
        &mut TargetInfo,
        &Transform,
        &Targeting,
        Option<&Watching>,
        Option<&mut Vision>,
        Option<&mut RayCaster>,
    )>,
    targets: Query<(&Transform, Option<&Kinematics>)>,
) {
    let Ok((mut target_info, transform, targeting, watching, vision, ray_caster)) =
        trackers.get_mut(inserted.entity)
    else {
        return;
    };
    let target = targeting.0;
    if target_info.targeted == Some(target) {
        return;
    }
    let tracked = target_info.targeted.or(watching.map(|watching| watching.0));
    target_info.targeted = Some(target);
    target_info.targeted_at = time.elapsed_secs();
    if tracked == Some(target) {
        return;
    }
    let Ok((target_transform, target_kinematics)) = targets.get(target) else {
        return;
    };
    let position = transform.translation.xy();
    let target_position = target_transform.translation.xy();
    let direction = (target_position - position).normalize_or_zero();
    target_info.direction = direction;
    target_info.distance = target_position.distance(position);
    target_info.position = target_position;
    target_info.velocity = target_kinematics.map_or(Vec2::ZERO, Kinematics::velocity);
    target_info.line_of_sight = false;
    if let Some(mut vision) = vision {
        vision.aim_direction = direction;
    }
    if let Some(mut ray_caster) = ray_caster {
        ray_caster.direction = Dir2::new(direction).unwrap_or(Dir2::X);
    }
}

/// Forgets the targeted entity, so targeting it again restarts the clock.
fn on_targeting_removed(removed: On<Remove<Targeting>>, mut target_infos: Query<&mut TargetInfo>) {
    if let Ok(mut target_info) = target_infos.get_mut(removed.entity) {
        target_info.targeted = None;
    }
}

/// Starts targeting the watched entity if it is in sight and in the vision cone.
fn should_target_watched(
    mut commands: Commands,
    npc_query: Query<(&TargetInfo, &Watching, Entity), Without<Targeting>>,
) {
    for (target_info, watching, entity) in &npc_query {
        if target_info.line_of_sight && target_info.in_vision_cone {
            commands.entity(entity).insert(Targeting(watching.0));
        }
    }
}

/// Stops targeting if the entity loses sight of the target or the target lock has expired.
/// A target switched to within [`SIGHT_GRACE`] is kept, since the sight ray only turns to it
/// after the next physics step.
fn should_stop_targeting(
    mut commands: Commands,
    time: Res<Time<Virtual>>,
    npc_query: Query<(&TargetInfo, Has<TargetLock>, Entity), With<Targeting>>,
) {
    let now = time.elapsed_secs();
    for (target_info, has_lock, entity) in &npc_query {
        let settling = now - target_info.targeted_at < SIGHT_GRACE;
        if !target_info.line_of_sight && !has_lock && !settling {
            commands.entity(entity).remove::<Targeting>();
        }
    }
}

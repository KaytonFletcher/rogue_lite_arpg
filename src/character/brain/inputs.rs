//! What scoring reads, answered on demand: [`DecisionInputs`] reads each
//! consideration's input (for one candidate target), capability profile and
//! availability straight from the world when the decision asks for it, so
//! nothing is copied up front.

use avian2d::prelude::{RayCaster, SpatialQuery};
use bevy::ecs::query::QueryData;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use super::Brain;
use super::action::{Ability, Action, find_carried_potion};
use super::capability::{Capabilities, CapabilityProfile};
use super::engine::{Behavior, MAX_TARGETS, TargetPools};
use super::motion::{HomeArea, Motion};
use super::perception::{SIGHT_GRACE, TargetLock, Targeting};
use crate::prelude::*;

/// The raw fact a consideration reads; see [`DecisionInputs::read`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ConsiderationInput {
    HasTarget,
    LineOfSight,
    TargetDistance,
    MissingHealth,
    ManaFraction,
    /// Whether this motion's kind is the one running; its payload is ignored.
    MotionRunning(Motion),
    /// Whether the running motion holds the target at a distance.
    Circling,
    /// How far past its home area range the character is.
    HomeAreaPull,
    AttackFinished,
    /// Target distance over this motion's reach; at most `1.0` is in reach.
    MotionReachFraction(Motion),
    /// Target distance over this action's reach; at most `1.0` is in reach.
    ActionReachFraction(Action),
    SecondsSinceTargeted,
}

impl ConsiderationInput {
    /// Whether this input is about the target, so it reads differently per target.
    pub const fn is_about_target(self) -> bool {
        match self {
            Self::HasTarget
            | Self::LineOfSight
            | Self::TargetDistance
            | Self::MotionReachFraction(_)
            | Self::ActionReachFraction(_)
            | Self::SecondsSinceTargeted => true,
            Self::MissingHealth
            | Self::ManaFraction
            | Self::MotionRunning(_)
            | Self::Circling
            | Self::HomeAreaPull
            | Self::AttackFinished => false,
        }
    }
}

/// The brain's own earlier choices, as scoring reads them.
#[derive(Clone, Copy, Debug)]
pub(super) struct BrainMemory {
    pub motion: Option<Motion>,
    /// An attack finished since the last decision.
    pub attack_finished: bool,
}

/// The components one decision reads. Capability components are not listed:
/// their profiles arrive through the [`Capabilities`] cache.
#[derive(QueryData)]
pub(super) struct DecisionInputComponents {
    entity: Entity,
    transform: &'static Transform,
    targeting: Option<&'static Targeting>,
    locked: Has<TargetLock>,
    target_info: Option<&'static TargetInfo>,
    side: Option<&'static CombatSide>,
    ray_caster: Option<&'static RayCaster>,
    health: Option<&'static Health>,
    mana: Option<&'static Mana>,
    home_area: Option<&'static HomeArea>,
    kinematics: Option<&'static Kinematics>,
    items: Option<&'static Items>,
    capabilities: &'static Capabilities,
}

/// The world a decision reads beyond its own entity, by concern. A new check
/// goes into the group it belongs to, or a new group here.
#[derive(SystemParam)]
pub(super) struct DecisionQueries<'w, 's> {
    uses: UseChecks<'w, 's>,
    targets: TargetScan<'w, 's>,
}

/// Whether the world lets an action run now.
#[derive(SystemParam)]
pub(super) struct UseChecks<'w, 's> {
    equipment: EquipmentRead<'w, 's>,
    consumables: Query<'w, 's, (), With<Consumable>>,
    items_in_use: Query<'w, 's, (), With<EquipmentInUse>>,
}

impl UseChecks<'_, '_> {
    /// Whether the world lets `entity`, carrying `items`, run `action` now:
    /// usable equipment (or a held use already running), and a carried potion
    /// for `Heal`.
    fn allow(&self, entity: Entity, items: Option<&Items>, action: Action) -> bool {
        match action {
            Action::Use(slot) => {
                let held_item_in_use = !slot.is_instant_use()
                    && self
                        .equipment
                        .item_in_slot(entity, slot)
                        .is_some_and(|item| self.items_in_use.contains(item));
                held_item_in_use || self.equipment.can_use(entity, slot).is_ok()
            }
            Action::Ability(Ability::Heal) => {
                find_carried_potion(items, &self.consumables).is_some()
            }
            Action::Ability(Ability::Breathe | Ability::Bite) | Action::Idle => true,
        }
    }
}

/// The characters a decision may pick as targets besides its focus, and the
/// sight-ray check they must pass.
#[derive(SystemParam)]
pub(super) struct TargetScan<'w, 's> {
    /// Every living character: each one carries a `CombatSide`.
    characters: Query<'w, 's, (Entity, &'static Transform, &'static CombatSide), With<Health>>,
    children: Query<'w, 's, &'static Children>,
    spatial: SpatialQuery<'w, 's>,
}

impl TargetScan<'_, '_> {
    /// Each living character not on `side` within `range` of `position`, with
    /// its position and distance.
    fn opposing_within(
        &self,
        side: CombatSide,
        position: Vec2,
        range: f32,
    ) -> impl Iterator<Item = (Entity, Vec2, f32)> {
        self.characters
            .iter()
            .filter(move |&(_, _, &other_side)| other_side != side)
            .map(move |(entity, transform, _)| {
                let target_position = transform.translation.xy();
                (entity, target_position, target_position.distance(position))
            })
            .filter(move |&(_, _, distance)| distance <= range)
    }

    /// Whether a ray with the sight ray's settings reaches `target` or one of
    /// its child colliders, as the vision system checks the focus.
    fn sight_ray_reaches(&self, ray_caster: &RayCaster, target: Entity, position: Vec2) -> bool {
        let origin = ray_caster.global_origin();
        let Ok(direction) = Dir2::new(position - origin) else {
            // Standing on the target: nothing can be in the way.
            return true;
        };
        self.spatial
            .cast_ray(
                origin,
                direction,
                ray_caster.max_distance,
                ray_caster.solid,
                &ray_caster.query_filter,
            )
            .is_some_and(|hit| {
                hit.entity == target
                    || self
                        .children
                        .get(target)
                        .is_ok_and(|children| children.contains(&hit.entity))
            })
    }
}

/// What one decision knows about a target it may pick.
#[derive(Clone, Copy, Debug)]
pub(super) struct TargetFacts {
    distance: f32,
    line_of_sight: bool,
    seconds_since_targeted: f32,
}

/// The targets one decision may pick from and their facts, gathered once: the
/// focus, then up to [`MAX_TARGETS`] in all of the opposing side in sight.
pub(super) struct Hostiles {
    entities: [Entity; MAX_TARGETS],
    facts: [TargetFacts; MAX_TARGETS],
    len: usize,
}

impl Hostiles {
    fn push(&mut self, entity: Entity, facts: TargetFacts) {
        if self.len < MAX_TARGETS {
            self.entities[self.len] = entity;
            self.facts[self.len] = facts;
            self.len += 1;
        }
    }

    pub fn as_slice(&self) -> &[Entity] {
        &self.entities[..self.len]
    }

    /// The facts about `target`; `None` for no target or one not gathered.
    pub fn facts(&self, target: Option<Entity>) -> Option<TargetFacts> {
        let index = self
            .as_slice()
            .iter()
            .position(|&entity| Some(entity) == target)?;
        Some(self.facts[index])
    }
}

/// One decision's view of the world: references only, read on demand.
pub(super) struct DecisionInputs<'a, 'w, 's, 'qw, 'qs> {
    pub item: &'a DecisionInputComponentsItem<'w, 's>,
    pub queries: &'a DecisionQueries<'qw, 'qs>,
    pub now: f32,
}

impl DecisionInputs<'_, '_, '_, '_, '_> {
    /// The focus target's tracking while targeting one at a finite distance.
    /// `TargetInfo` also tracks a watched entity, so every focus read goes
    /// through here.
    pub fn target(&self) -> Option<&TargetInfo> {
        self.item
            .target_info
            .filter(|info| self.item.targeting.is_some() && info.distance.is_finite())
    }

    /// The focus target, while [`Self::target`] tracks it.
    pub fn focus(&self) -> Option<Entity> {
        self.target()
            .and(self.item.targeting)
            .map(|targeting| targeting.0)
    }

    /// The targets this brain may pick while it has a focus: the focus first,
    /// then, when `gather_others` (a behavior scores hostiles), other living
    /// characters of the opposing side its sight ray reaches. Only the focus
    /// while an aggro [`TargetLock`] holds it.
    pub fn hostiles(&self, gather_others: bool) -> Hostiles {
        let unknown = TargetFacts {
            distance: f32::INFINITY,
            line_of_sight: false,
            seconds_since_targeted: 0.0,
        };
        let mut hostiles = Hostiles {
            entities: [Entity::PLACEHOLDER; MAX_TARGETS],
            facts: [unknown; MAX_TARGETS],
            len: 0,
        };
        let (Some(focus), Some(info)) = (self.focus(), self.target()) else {
            return hostiles;
        };
        let seconds_since_targeted = self.now - info.targeted_at;
        let ray_caster = self.item.ray_caster;
        // A focus switched to moments ago has no line of sight until the turned
        // sight ray reports, so cast for it rather than let an older target win.
        let line_of_sight = match ray_caster {
            Some(ray_caster) if seconds_since_targeted < SIGHT_GRACE => self
                .queries
                .targets
                .sight_ray_reaches(ray_caster, focus, info.position),
            _ => info.line_of_sight,
        };
        hostiles.push(
            focus,
            TargetFacts {
                distance: info.distance,
                line_of_sight,
                seconds_since_targeted,
            },
        );
        let (Some(side), Some(ray_caster)) = (self.item.side, ray_caster) else {
            return hostiles;
        };
        if !gather_others || self.item.locked {
            return hostiles;
        }
        let scan = &self.queries.targets;
        let position = self.item.transform.translation.xy();
        for (entity, target_position, distance) in
            scan.opposing_within(*side, position, ray_caster.max_distance)
        {
            if hostiles.len == MAX_TARGETS {
                break;
            }
            if entity != focus && scan.sight_ray_reaches(ray_caster, entity, target_position) {
                hostiles.push(
                    entity,
                    TargetFacts {
                        distance,
                        line_of_sight: true,
                        // Switching to it would start its clock now.
                        seconds_since_targeted: 0.0,
                    },
                );
            }
        }
        hostiles
    }

    /// One decision's target pools over `hostiles`.
    pub fn pools<'h>(&self, hostiles: &'h Hostiles) -> TargetPools<'h, Entity> {
        TargetPools {
            focus: self.focus(),
            hostiles: hostiles.as_slice(),
        }
    }

    pub fn entity(&self) -> Entity {
        self.item.entity
    }

    /// Whether the brain has a focus or the hostile it tracks is within `distance`.
    pub fn is_engaged_or_within(&self, distance: f32) -> bool {
        self.item.targeting.is_some()
            || self
                .item
                .target_info
                .is_some_and(|info| info.distance <= distance)
    }

    pub fn stunned(&self) -> bool {
        self.item.kinematics.is_some_and(Kinematics::is_stunned)
    }

    /// The profile of the capability that carries out `motion`; `None` when the
    /// entity lacks it. Motions steered without a component are always owned.
    pub fn motion_profile(&self, motion: Motion) -> Option<CapabilityProfile> {
        self.item.capabilities.get(motion)
    }

    /// The profile of the capability that carries out `action`: the equipped
    /// item's reach or the ability's profile, as cached in [`Capabilities`].
    /// `Idle` needs none.
    pub fn action_profile(&self, action: Action) -> Option<CapabilityProfile> {
        self.item.capabilities.get(action)
    }

    /// Whether `behavior` may start now: each part's capability is owned, off its
    /// cooldown on `brain`, and allowed by the world.
    pub fn is_available(&self, behavior: &Behavior, brain: &Brain) -> bool {
        let motion_available = |motion: Motion| {
            self.motion_profile(motion).is_some() && brain.is_cooldown_ready(motion, self.now)
        };
        let action_available = |action: Action| {
            self.action_profile(action).is_some()
                && brain.is_cooldown_ready(action, self.now)
                && self
                    .queries
                    .uses
                    .allow(self.item.entity, self.item.items, action)
        };
        behavior.motion.is_none_or(motion_available) && behavior.action.is_none_or(action_available)
    }

    /// Answers `input` about the target `facts` describe; `None` when it is
    /// absent, which vetoes the behavior. Inputs about the character itself
    /// ignore `target`.
    pub fn read(
        &self,
        input: ConsiderationInput,
        target: Option<TargetFacts>,
        memory: BrainMemory,
    ) -> Option<f32> {
        match input {
            ConsiderationInput::HasTarget => Some(f32::from(target.is_some())),
            ConsiderationInput::LineOfSight => target.map(|target| f32::from(target.line_of_sight)),
            ConsiderationInput::TargetDistance => target.map(|target| target.distance),
            ConsiderationInput::MissingHealth => {
                Some(1.0 - self.item.health.and_then(Health::fraction).unwrap_or(1.0))
            }
            ConsiderationInput::ManaFraction => {
                Some(self.item.mana.and_then(Mana::fraction).unwrap_or(1.0))
            }
            ConsiderationInput::MotionRunning(motion) => Some(f32::from(
                memory
                    .motion
                    .is_some_and(|running| running.index() == motion.index()),
            )),
            ConsiderationInput::Circling => Some(f32::from(matches!(
                memory.motion,
                Some(Motion::Orbit(_) | Motion::KeepDistance(_))
            ))),
            ConsiderationInput::HomeAreaPull => self.home_area_pull(memory),
            ConsiderationInput::AttackFinished => Some(f32::from(memory.attack_finished)),
            ConsiderationInput::MotionReachFraction(motion) => {
                target.map(|target| reach_fraction(target.distance, self.motion_profile(motion)))
            }
            ConsiderationInput::ActionReachFraction(action) => {
                target.map(|target| reach_fraction(target.distance, self.action_profile(action)))
            }
            ConsiderationInput::SecondsSinceTargeted => {
                target.map(|target| target.seconds_since_targeted)
            }
        }
    }

    /// How far past its home area range the character is. Once already heading
    /// home it reads full until arrival, so crossing back inside the range
    /// mid-return cannot hand the character straight back to wandering.
    fn home_area_pull(&self, memory: BrainMemory) -> Option<f32> {
        let home_area = self.item.home_area?;
        let position = self.item.transform.translation.xy();
        let overshoot_fraction = home_area.overshoot_fraction(position)?;
        if memory.motion == Some(Motion::ReturnToHome) {
            let arrived = home_area.distance_from(position) <= home_area.arrival_radius();
            Some(f32::from(!arrived))
        } else {
            Some(overshoot_fraction)
        }
    }
}

/// `distance` over the capability's reach; at most `1.0` is in reach.
/// `f32::MAX` for a capability that is missing or has no positive reach.
fn reach_fraction(distance: f32, profile: Option<CapabilityProfile>) -> f32 {
    profile
        .and_then(|profile| profile.reach)
        .filter(|&reach| reach > 0.0)
        .map_or(f32::MAX, |reach| distance / reach)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_reach(reach: Option<f32>) -> CapabilityProfile {
        CapabilityProfile {
            reach,
            ..CapabilityProfile::default()
        }
    }

    #[test]
    fn reach_fraction_falls_back_to_far() {
        assert!(
            (reach_fraction(50.0, Some(with_reach(Some(100.0)))) - 0.5).abs() < f32::EPSILON,
            "half the reach away"
        );
        for (profile, case) in [
            (None, "missing"),
            (Some(with_reach(None)), "reachless"),
            (Some(with_reach(Some(0.0))), "zero-reach"),
        ] {
            assert!(
                reach_fraction(50.0, profile) >= f32::MAX,
                "a {case} capability reads as far"
            );
        }
    }
}

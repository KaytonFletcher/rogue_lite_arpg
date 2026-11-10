//! The decision pipeline: drop a stale committed behavior, score with inputs read on
//! demand, and commit. This is the only code that changes which behaviors a
//! [`Brain`] holds.

use bevy::ecs::lifecycle::HookContext;
use bevy::ecs::system::ParallelCommands;
use bevy::ecs::world::DeferredWorld;
use bevy::prelude::*;
use rand::RngExt;

use super::capability::MotionActionTable;
#[cfg(feature = "dev_tools")]
use super::diagnostics::DecisionStats;
use super::engine::{
    Behavior, BehaviorScoreExplanation, assign_behaviors_to_slots, explain_behavior,
};
use super::inputs::{ConsiderationInput, DecisionInputComponents, DecisionInputs, DecisionQueries};
use super::motion::{Motion, MotionFinished, mark_running_motion};
use super::{Action, ActionFinished, ActionOutcome, Brain, Targeting};

/// Seconds between a brain's scheduled decisions while it is engaged or a
/// hostile is near.
const DECISION_PERIOD: f32 = 0.1;
/// Seconds between decisions for a brain far from any hostile, which has
/// nothing to react to quickly: its level of detail.
const FAR_DECISION_PERIOD: f32 = 0.5;
/// Distance to the watched hostile beyond which a brain is far. Well past every
/// sight range, so a brain is back to full rate before it can spot anyone.
const NEAR_DISTANCE: f32 = 1000.0;

type DecidingBrains<'world, 'state> = Query<
    'world,
    'state,
    (
        &'static mut Brain,
        DecisionInputComponents,
        Option<&'static mut ScoreTrace>,
    ),
>;

/// Debug readout of a brain's last decision: one explanation per behavior, in
/// definition order. Only brains carrying it pay for the explanations.
#[derive(Component, Debug, Default)]
pub struct ScoreTrace {
    pub behavior_explanations: Vec<BehaviorScoreExplanation<Entity>>,
}

type PacedBrains<'world, 'state> = Query<
    'world,
    'state,
    (
        &'static mut Brain,
        DecisionInputComponents,
        Option<&'static mut ScoreTrace>,
        &'static mut DecisionClock,
    ),
>;

/// When a brain next decides on its own schedule. Kept apart from [`Brain`], so
/// ticking it does not mark the brain changed.
#[derive(Component, Debug, Default)]
#[component(on_add = schedule_first_decision)]
pub struct DecisionClock {
    next_decision_at: f32,
}

impl DecisionClock {
    /// Whether a decision is due at `now`. A due clock moves to its next
    /// `period` boundary after `now`, keeping its phase without a burst of
    /// catch-up decisions after a long frame.
    fn tick(&mut self, now: f32, period: f32) -> bool {
        if now < self.next_decision_at {
            return false;
        }
        let periods_passed = ((now - self.next_decision_at) / period).floor() + 1.0;
        self.next_decision_at += periods_passed * period;
        // Rounding near an exact multiple can land the boundary on `now`.
        if self.next_decision_at <= now {
            self.next_decision_at += period;
        }
        true
    }
}

/// Puts a new brain's first decision at a random point in the next period, so brains spawned
/// together don't all decide on the same frame.
fn schedule_first_decision(mut world: DeferredWorld, context: HookContext) {
    let now = world
        .get_resource::<Time<Virtual>>()
        .map_or(0.0, Time::elapsed_secs);
    if let Some(mut clock) = world.get_mut::<DecisionClock>(context.entity) {
        clock.next_decision_at = now + rand::rng().random_range(0.0..DECISION_PERIOD);
    }
}

/// Starts a finished motion's cooldown and, if its run is still current, decides again at once.
pub(super) fn on_release_finished_motion(
    finished: On<MotionFinished>,
    time: Res<Time<Virtual>>,
    queries: DecisionQueries,
    mut brains: DecidingBrains,
    mut commands: Commands,
    #[cfg(feature = "dev_tools")] decision_stats: Res<DecisionStats>,
) {
    let Ok((mut brain, item, mut score_trace)) = brains.get_mut(finished.entity) else {
        return;
    };
    let inputs = DecisionInputs {
        item: &item,
        queries: &queries,
        now: time.elapsed_secs(),
    };
    let cooldown = inputs
        .motion_profile(finished.motion)
        .map_or(0.0, |profile| profile.cooldown);
    brain.start_cooldown(finished.motion, inputs.now + cooldown);
    let held_motion = brain.motion();
    if brain.finish_committed_behavior(finished.behavior_run_id) {
        #[cfg(feature = "tracy")]
        let _span = info_span!("decide::after_motion").entered();
        redecide_now(
            &mut brain,
            held_motion,
            &inputs,
            &mut commands,
            score_trace.as_deref_mut(),
        );
        #[cfg(feature = "dev_tools")]
        decision_stats.record_redecision();
    }
}

/// Applies an [`ActionFinished`], current or stale. An ability starts its
/// cooldown even if it never started, so the next decision can't pick it
/// straight back.
pub(super) fn on_finish_action(
    finished: On<ActionFinished>,
    time: Res<Time<Virtual>>,
    queries: DecisionQueries,
    mut brains: DecidingBrains,
    mut commands: Commands,
    #[cfg(feature = "dev_tools")] decision_stats: Res<DecisionStats>,
) {
    let Ok((mut brain, item, mut score_trace)) = brains.get_mut(finished.entity) else {
        return;
    };
    let inputs = DecisionInputs {
        item: &item,
        queries: &queries,
        now: time.elapsed_secs(),
    };
    match finished.action {
        Action::Use(slot)
            if slot.is_instant_use() && finished.outcome == ActionOutcome::Completed =>
        {
            brain.attack_finish_pending = true;
        }
        Action::Ability(ability) => {
            let cooldown = inputs
                .action_profile(finished.action)
                .map_or(0.0, |profile| profile.cooldown);
            brain.start_cooldown(ability, inputs.now + cooldown);
        }
        // A held or unstarted use is no attack, and nothing reports an idle
        // action's finish.
        Action::Use(_) | Action::Idle => {}
    }
    let held_motion = brain.motion();
    if brain.finish_committed_behavior(finished.behavior_run_id) {
        #[cfg(feature = "tracy")]
        let _span = info_span!("decide::after_action").entered();
        redecide_now(
            &mut brain,
            held_motion,
            &inputs,
            &mut commands,
            score_trace.as_deref_mut(),
        );
        #[cfg(feature = "dev_tools")]
        decision_stats.record_redecision();
    }
}

/// Decides for every brain whose clock is due, in parallel. A brain far from
/// any hostile decides at [`FAR_DECISION_PERIOD`] instead of [`DECISION_PERIOD`].
pub(super) fn decide_behavior(
    time: Res<Time<Virtual>>,
    queries: DecisionQueries,
    mut brains: PacedBrains,
    par_commands: ParallelCommands,
    #[cfg(feature = "dev_tools")] decision_stats: Res<DecisionStats>,
) {
    let now = time.elapsed_secs();
    brains
        .par_iter_mut()
        .for_each(|(mut brain, item, mut score_trace, mut clock)| {
            let inputs = DecisionInputs {
                item: &item,
                queries: &queries,
                now,
            };
            let period = if inputs.is_engaged_or_within(NEAR_DISTANCE) {
                DECISION_PERIOD
            } else {
                FAR_DECISION_PERIOD
            };
            if !clock.tick(now, period) {
                return;
            }
            #[cfg(feature = "tracy")]
            let _span = info_span!("decide::paced").entered();
            par_commands.command_scope(|mut commands| {
                let held_motion = brain.motion();
                redecide_now(
                    &mut brain,
                    held_motion,
                    &inputs,
                    &mut commands,
                    score_trace.as_deref_mut(),
                );
            });
            #[cfg(feature = "dev_tools")]
            decision_stats.record_paced_decision();
        });
}

/// Drops committed behaviors that lost their target, were stunned or timed
/// out, then fills the slots, each behavior against the best target in its
/// pool. A winning finite motion rolls its start chance; a miss waits out its
/// cooldown and the next best behavior takes the slot. A winner against a
/// hostile other than the focus makes it the new focus. `held_motion` is the
/// motion its [`Running`](super::motion::Running) marker shows, read before a
/// finish cleared it.
fn redecide_now(
    brain: &mut Brain,
    held_motion: Option<Motion>,
    inputs: &DecisionInputs,
    commands: &mut Commands,
    score_trace: Option<&mut ScoreTrace>,
) {
    let (has_target, stunned) = (inputs.target().is_some(), inputs.stunned());
    for running in brain.running_behaviors.clone() {
        if running.should_drop_committed_behavior(has_target, stunned, inputs.now) {
            brain.drop_committed_behavior(running, inputs);
        }
    }
    // Read after the drops, which change the running motion and cooldowns.
    let memory = brain.memory();
    let definition = brain.definition();
    let running = brain.running_picks();
    let hostiles = inputs.hostiles();
    let pools = inputs.pools(&hostiles);
    let read = |input: ConsiderationInput, target: Option<Entity>| {
        inputs.read(input, hostiles.facts(target), memory)
    };
    let available = |behavior: &Behavior| inputs.is_available(behavior, brain);
    if let Some(score_trace) = score_trace {
        #[cfg(feature = "tracy")]
        let _span = info_span!("decide::explain").entered();
        score_trace.behavior_explanations.clear();
        score_trace.behavior_explanations.extend(
            (0..definition.behaviors.len()).map(|index| {
                explain_behavior(definition, index, &pools, &read, &available, &running)
            }),
        );
    }
    let mut missed_start_cooldowns = MotionActionTable::<Option<f32>>::default();
    let winners = {
        #[cfg(feature = "tracy")]
        let _span = info_span!("decide::assign").entered();
        assign_behaviors_to_slots(definition, &pools, &read, &available, &running, |index| {
            passes_start_roll(
                inputs,
                definition.behaviors[index],
                &mut missed_start_cooldowns,
            )
        })
    };
    #[cfg(feature = "tracy")]
    let _span = info_span!("decide::commit").entered();
    brain.commit_decision(&winners, &missed_start_cooldowns, inputs);
    let mut entity = commands.entity(inputs.entity());
    mark_running_motion(&mut entity, held_motion, brain.motion());
    let target = winners.iter().find_map(|winner| winner.target);
    if let Some(target) = target.filter(|&target| pools.focus != Some(target)) {
        entity.insert(Targeting(target));
    }
}

fn passes_start_roll(
    inputs: &DecisionInputs,
    behavior: Behavior,
    missed_start_cooldowns: &mut MotionActionTable<Option<f32>>,
) -> bool {
    let Some(motion) = behavior.motion.filter(|motion| motion.is_finite()) else {
        return true;
    };
    let Some(capability_profile) = inputs.motion_profile(motion) else {
        return false;
    };
    if capability_profile.chance >= 1.0 || rand::rng().random::<f32>() < capability_profile.chance {
        return true;
    }
    missed_start_cooldowns[motion] = Some(inputs.now + capability_profile.cooldown);
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clock_keeps_its_phase_without_catching_up() {
        let mut clock = DecisionClock {
            next_decision_at: 1.05,
        };
        assert!(!clock.tick(1.0, 0.1), "not due before its time");
        assert!(clock.tick(1.37, 0.1), "due once late");
        assert!(
            (clock.next_decision_at - 1.45).abs() < 1e-5,
            "the next boundary after now on the old phase, got {}",
            clock.next_decision_at
        );
        assert!(!clock.tick(1.40, 0.1), "one late frame is one decision");
    }
}

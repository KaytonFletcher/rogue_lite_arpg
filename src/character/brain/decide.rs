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

/// Seconds between a brain's scheduled decisions.
const DECISION_PERIOD: f32 = 0.1;
/// A brain far from any hostile, with nothing to react to quickly, decides on
/// one in this many scheduled ticks: its level of detail.
const FAR_TICKS_PER_DECISION: u8 = 5;
/// Distance to the tracked hostile beyond which a brain is far. Well past every
/// sight range, so a brain is back to full rate before it can spot anyone.
const NEAR_DISTANCE: f32 = 1000.0;

type DecidingBrains<'world, 'state> = Query<
    'world,
    'state,
    (
        &'static mut Brain,
        DecisionInputComponents,
        Option<&'static mut ScoreTrace>,
        &'static mut DecisionClock,
    ),
>;

/// Debug readout of a brain's last decision: one explanation per behavior, in
/// definition order. Only brains carrying it pay for the explanations.
#[derive(Component, Debug, Default)]
pub struct ScoreTrace {
    pub behavior_explanations: Vec<BehaviorScoreExplanation<Entity>>,
}

/// When a brain next decides on its own schedule, kept apart from the
/// decisions it records in [`Brain`].
#[derive(Component, Debug, Default)]
#[component(on_add = schedule_first_decision)]
pub struct DecisionClock {
    next_tick_at: f32,
    /// Counts a far brain's ticks; it decides when this wraps to zero.
    far_ticks: u8,
}

impl DecisionClock {
    /// Whether a decision is due at `now`. The clock ticks every
    /// [`DECISION_PERIOD`], keeping its phase without a burst of catch-up ticks
    /// after a long frame, and a `far` brain decides on one tick in
    /// [`FAR_TICKS_PER_DECISION`]. A brain that comes near decides on its next
    /// tick.
    fn tick(&mut self, now: f32, far: bool) -> bool {
        if now < self.next_tick_at {
            return false;
        }
        let periods_passed = ((now - self.next_tick_at) / DECISION_PERIOD).floor() + 1.0;
        self.next_tick_at += periods_passed * DECISION_PERIOD;
        // Rounding near an exact multiple can land the boundary on `now`.
        if self.next_tick_at <= now {
            self.next_tick_at += DECISION_PERIOD;
        }
        if !far {
            self.far_ticks = 0;
            return true;
        }
        self.far_ticks = (self.far_ticks + 1) % FAR_TICKS_PER_DECISION;
        self.far_ticks == 0
    }
}

/// Puts a new brain's first tick at a random point in the next period, and its
/// far decisions on a random tick, so brains spawned together don't all decide
/// on the same frame.
fn schedule_first_decision(mut world: DeferredWorld, context: HookContext) {
    let now = world
        .get_resource::<Time<Virtual>>()
        .map_or(0.0, Time::elapsed_secs);
    if let Some(mut clock) = world.get_mut::<DecisionClock>(context.entity) {
        let mut rng = rand::rng();
        clock.next_tick_at = now + rng.random_range(0.0..DECISION_PERIOD);
        clock.far_ticks = rng.random_range(0..FAR_TICKS_PER_DECISION);
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
    let Ok((mut brain, item, mut score_trace, _)) = brains.get_mut(finished.entity) else {
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
    let Ok((mut brain, item, mut score_trace, _)) = brains.get_mut(finished.entity) else {
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

/// Decides for every brain whose clock is due, in parallel.
pub(super) fn decide_behavior(
    time: Res<Time<Virtual>>,
    queries: DecisionQueries,
    mut brains: DecidingBrains,
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
            if !clock.tick(now, !inputs.is_engaged_or_within(NEAR_DISTANCE)) {
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
    let (focus, stunned) = (inputs.focus(), inputs.stunned());
    for running in brain.running_behaviors.clone() {
        if running.should_drop_committed_behavior(focus, stunned, inputs.now) {
            brain.drop_committed_behavior(running, inputs);
        }
    }
    // Read after the drops, which change the running motion and cooldowns.
    let memory = brain.memory();
    let definition = brain.definition();
    let running = brain.running_picks();
    let hostiles = inputs.hostiles(definition.scores_hostiles());
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

    fn clock_at(next_tick_at: f32) -> DecisionClock {
        DecisionClock {
            next_tick_at,
            far_ticks: 0,
        }
    }

    #[test]
    fn clock_keeps_its_phase_without_catching_up() {
        let mut clock = clock_at(1.05);
        assert!(!clock.tick(1.0, false), "not due before its time");
        assert!(clock.tick(1.37, false), "due once late");
        assert!(
            (clock.next_tick_at - 1.45).abs() < 1e-5,
            "the next boundary after now on the old phase, got {}",
            clock.next_tick_at
        );
        assert!(!clock.tick(1.40, false), "one late frame is one decision");
    }

    #[test]
    fn clock_never_lands_on_now() {
        let mut clock = clock_at(1.0);
        assert!(clock.tick(1.0, false), "due exactly on time");
        assert!(clock.next_tick_at > 1.0, "the next tick is after now");
    }

    #[test]
    fn far_brain_decides_on_one_tick_in_five_and_comes_back_at_once() {
        let mut clock = clock_at(0.0);
        // Halfway between boundaries, so each frame is exactly one tick.
        let decisions = (0_u8..10)
            .filter(|&tick| clock.tick(f32::from(tick) * DECISION_PERIOD + 0.05, true))
            .count();
        assert_eq!(decisions, 2, "ten far ticks make two decisions");
        assert!(clock.tick(1.05, false), "the first near tick decides");
    }
}

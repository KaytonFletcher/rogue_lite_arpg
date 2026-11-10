//! Fills the motion and action slots: skips what may not run, scores the rest, and
//! takes winners from the highest score down.

use super::behavior::{Behavior, BrainDefinition, MAX_BEHAVIORS};
use super::scoring::score_behavior;
use super::targets::{MAX_TARGETS, Pick, TargetPools};
use crate::character::brain::ConsiderationInput;

/// Score multiplier a behavior keeps over challengers while it holds a slot, so
/// a near-tie does not flip the character back and forth between ticks.
pub(crate) const MOMENTUM: f32 = 1.25;

/// Why a behavior sits out a decision unscored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SkipReason {
    /// A slot it claims is held by a committed behavior it does not outrank.
    SlotCommitted,
    /// This is the committed behavior holding its slots, so it is not re-scored.
    RunningCommitted,
    /// It may not start now: a missing capability, a cooldown, or the world.
    Unavailable,
}

impl SkipReason {
    /// Short text for the debug overlay.
    pub const fn label(self) -> &'static str {
        match self {
            Self::SlotCommitted => "slot committed",
            Self::RunningCommitted => "running committed",
            Self::Unavailable => "unavailable",
        }
    }
}

/// Scores every behavior not skipped (see [`SkipReason`]) against each target in
/// its pool, then takes them from the highest score down while their slots are
/// free; ties go to the earlier behavior, then the earlier target. A `false`
/// from `start` drops a winner for the next best. `read` answers each
/// consideration's input for a target and `available` whether a behavior may
/// start now.
///
/// A character fights one target at a time: once a winner names a target, the
/// others may only win against that target or none. Momentum goes to a running
/// behavior only against the target it runs against, so a near tie does not
/// flip targets either.
pub fn assign_behaviors_to_slots<T: Copy + PartialEq>(
    definition: &BrainDefinition,
    pools: &TargetPools<T>,
    read: &impl Fn(ConsiderationInput, Option<T>) -> Option<f32>,
    available: &impl Fn(&Behavior) -> bool,
    running: &[Pick<T>],
    mut start: impl FnMut(usize) -> bool,
) -> Vec<Pick<T>> {
    // One score per behavior and target, in `TargetPools::candidates` order.
    let mut scores = [[0.0_f32; MAX_TARGETS]; MAX_BEHAVIORS];
    for (index, behavior) in definition.behaviors.iter().enumerate() {
        if find_skip_reason(definition, behavior, available, running).is_some() {
            continue;
        }
        for (column, target) in pools.candidates(behavior).enumerate() {
            let is_current = running.contains(&Pick {
                behavior: index,
                target,
            });
            let read_target = |input: ConsiderationInput| read(input, target);
            if let Some(score) = score_behavior(behavior, &read_target, is_current) {
                scores[index][column] = score * momentum_multiplier(is_current);
            }
        }
    }

    let mut winners: Vec<Pick<T>> = Vec::with_capacity(2);
    // A committed behavior still running keeps its target, so the rest agree with it.
    let mut chosen_target = running
        .iter()
        .filter(|running| definition.behaviors[running.behavior].commit.is_committed())
        .find_map(|running| running.target);
    loop {
        let mut best: Option<(Pick<T>, f32)> = None;
        for (index, behavior) in definition.behaviors.iter().enumerate() {
            let taken = winners
                .iter()
                .any(|winner| definition.behaviors[winner.behavior].shares_a_slot_with(behavior));
            if taken {
                continue;
            }
            for (column, target) in pools.candidates(behavior).enumerate() {
                let score = scores[index][column];
                let agrees = target.is_none() || chosen_target.is_none() || target == chosen_target;
                if agrees && score > best.map_or(0.0, |(_, leader)| leader) {
                    best = Some((
                        Pick {
                            behavior: index,
                            target,
                        },
                        score,
                    ));
                }
            }
        }
        let Some((pick, _)) = best else {
            return winners;
        };
        if !start(pick.behavior) {
            scores[pick.behavior] = [0.0; MAX_TARGETS];
            continue;
        }
        chosen_target = chosen_target.or(pick.target);
        winners.push(pick);
    }
}

pub(super) fn find_skip_reason<T>(
    definition: &BrainDefinition,
    behavior: &Behavior,
    available: &impl Fn(&Behavior) -> bool,
    running: &[Pick<T>],
) -> Option<SkipReason> {
    let tier = behavior.commit.interrupt_tier();
    let slot_committed = running.iter().any(|running| {
        let running_behavior = &definition.behaviors[running.behavior];
        let running_tier = running_behavior.commit.interrupt_tier();
        running_tier > 0 && tier <= running_tier && running_behavior.shares_a_slot_with(behavior)
    });
    if slot_committed {
        return Some(SkipReason::SlotCommitted);
    }
    (!available(behavior)).then_some(SkipReason::Unavailable)
}

pub(super) fn momentum_multiplier(is_current: bool) -> f32 {
    if is_current { MOMENTUM } else { 1.0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::character::brain::considerations::{HAS_TARGET, distance_threshold};
    use crate::character::brain::{Ability, Action, CommitPolicy, Consideration, Motion, Targets};

    const NEAR: Consideration = distance_threshold("near", 100.0, 120.0);

    static BRAIN: BrainDefinition = BrainDefinition {
        behaviors: &[
            Behavior::new(
                Some(Motion::Wander),
                None,
                CommitPolicy::ContinuouslyScored,
                1.0,
                &[],
            ),
            Behavior::new(
                Some(Motion::Chase),
                None,
                CommitPolicy::ContinuouslyScored,
                2.0,
                &[NEAR],
            ),
            Behavior::new(
                Some(Motion::Dash),
                None,
                CommitPolicy::UntilFinished { interrupt_tier: 1 },
                3.0,
                &[],
            ),
            Behavior::new(
                Some(Motion::Swoop),
                None,
                CommitPolicy::UntilFinished { interrupt_tier: 2 },
                0.5,
                &[],
            ),
        ],
    };

    /// No targets at all, as for a brain with no focus.
    const NO_TARGETS: TargetPools<u8> = TargetPools {
        focus: None,
        hostiles: &[],
    };

    /// Only wander and chase may start.
    fn available(behavior: &Behavior) -> bool {
        matches!(behavior.motion, Some(Motion::Wander | Motion::Chase))
    }

    fn untargeted(behaviors: &[usize]) -> Vec<Pick<u8>> {
        behaviors
            .iter()
            .map(|&behavior| Pick {
                behavior,
                target: None,
            })
            .collect()
    }

    fn read_distance(distance: f32) -> impl Fn(ConsiderationInput, Option<u8>) -> Option<f32> {
        move |input, _| match input {
            ConsiderationInput::TargetDistance => Some(distance),
            _ => Some(1.0),
        }
    }

    #[test]
    fn exit_threshold_keeps_a_running_chase() {
        let read = read_distance(110.0);
        assert_eq!(
            assign_behaviors_to_slots(&BRAIN, &NO_TARGETS, &read, &available, &[], |_| true),
            untargeted(&[0]),
            "110 is past the 100 enter threshold, so wander wins"
        );
        assert_eq!(
            assign_behaviors_to_slots(
                &BRAIN,
                &NO_TARGETS,
                &read,
                &available,
                &untargeted(&[1]),
                |_| true
            ),
            untargeted(&[1]),
            "the 120 exit threshold holds a running chase"
        );
    }

    #[test]
    fn rejected_start_hands_the_slot_to_the_next_best() {
        let read = read_distance(50.0);
        assert_eq!(
            assign_behaviors_to_slots(&BRAIN, &NO_TARGETS, &read, &available, &[], |index| {
                index != 1
            }),
            untargeted(&[0]),
            "chase wins on score but fails its start, so wander takes the slot"
        );
    }

    #[test]
    fn running_committed_behavior_yields_only_to_a_higher_tier() {
        let read = read_distance(50.0);
        let running = untargeted(&[2]);
        assert_eq!(
            assign_behaviors_to_slots(&BRAIN, &NO_TARGETS, &read, &all, &running, |index| {
                index != 3
            }),
            untargeted(&[]),
            "a running tier-1 dash keeps the motion slot from tiers 0 and 1"
        );
        assert_eq!(
            assign_behaviors_to_slots(&BRAIN, &NO_TARGETS, &read, &all, &running, |_| true),
            untargeted(&[3]),
            "a tier-2 swoop takes the slot despite its lower score"
        );
    }

    /// Chases a near hostile; bites the focus wherever it is; idles otherwise.
    static PER_TARGET: BrainDefinition = BrainDefinition {
        behaviors: &[
            Behavior::new(
                Some(Motion::Chase),
                None,
                CommitPolicy::ContinuouslyScored,
                2.0,
                &[NEAR],
            )
            .with_targets(Targets::Hostiles),
            Behavior::new(
                None,
                Some(Action::Ability(Ability::Bite)),
                CommitPolicy::ContinuouslyScored,
                1.0,
                &[HAS_TARGET],
            ),
            Behavior::new(
                None,
                Some(Action::Idle),
                CommitPolicy::ContinuouslyScored,
                0.1,
                &[],
            ),
        ],
    };

    /// Target 1 is 150 away, every other target 50.
    fn read_per_target(input: ConsiderationInput, target: Option<u8>) -> Option<f32> {
        match (input, target) {
            (ConsiderationInput::TargetDistance, Some(1)) => Some(150.0),
            (ConsiderationInput::TargetDistance, Some(_)) => Some(50.0),
            (ConsiderationInput::TargetDistance, None) => None,
            (ConsiderationInput::HasTarget, target) => Some(f32::from(target.is_some())),
            _ => Some(1.0),
        }
    }

    fn all(_: &Behavior) -> bool {
        true
    }

    #[test]
    fn hostiles_behavior_picks_the_target_that_scores_best() {
        let pools = TargetPools {
            focus: Some(1),
            hostiles: &[1, 2],
        };
        let winners =
            assign_behaviors_to_slots(&PER_TARGET, &pools, &read_per_target, &all, &[], |_| true);
        assert_eq!(
            winners,
            vec![
                Pick {
                    behavior: 0,
                    target: Some(2),
                },
                Pick {
                    behavior: 2,
                    target: None,
                },
            ],
            "the chase takes the near hostile 2; the bite at focus 1 disagrees with it, so the \
             action slot idles"
        );
    }

    #[test]
    fn momentum_holds_the_running_target() {
        let pools = TargetPools {
            focus: Some(2),
            hostiles: &[2, 3],
        };
        let running = [Pick {
            behavior: 0,
            target: Some(3),
        }];
        let winners = assign_behaviors_to_slots(
            &PER_TARGET,
            &pools,
            &read_per_target,
            &all,
            &running,
            |_| true,
        );
        assert_eq!(
            winners.first(),
            Some(&Pick {
                behavior: 0,
                target: Some(3),
            }),
            "an equal score elsewhere does not pull the chase off its running target"
        );
    }

    #[test]
    fn empty_hostile_pool_scores_once_without_a_target() {
        static ALONE: BrainDefinition = BrainDefinition {
            behaviors: &[Behavior::new(
                Some(Motion::Wander),
                None,
                CommitPolicy::ContinuouslyScored,
                1.0,
                &[HAS_TARGET.inverted("no target")],
            )
            .with_targets(Targets::Hostiles)],
        };
        assert_eq!(
            assign_behaviors_to_slots(&ALONE, &NO_TARGETS, &read_per_target, &all, &[], |_| true),
            untargeted(&[0]),
            "with no hostiles a no-target consideration passes"
        );
    }

    #[test]
    fn failed_start_drops_every_target_of_the_behavior() {
        let pools = TargetPools {
            focus: Some(2),
            hostiles: &[2, 3],
        };
        let winners =
            assign_behaviors_to_slots(&PER_TARGET, &pools, &read_per_target, &all, &[], |index| {
                index != 0
            });
        assert!(
            winners.iter().all(|winner| winner.behavior != 0),
            "a chase that fails its start wins against no other target either"
        );
    }

    #[test]
    fn focus_winner_restricts_later_hostile_behaviors() {
        static FOCUSED: BrainDefinition = BrainDefinition {
            behaviors: &[
                Behavior::new(
                    Some(Motion::Chase),
                    None,
                    CommitPolicy::ContinuouslyScored,
                    3.0,
                    &[HAS_TARGET],
                ),
                Behavior::new(
                    None,
                    Some(Action::Ability(Ability::Bite)),
                    CommitPolicy::ContinuouslyScored,
                    2.0,
                    &[NEAR],
                )
                .with_targets(Targets::Hostiles),
                Behavior::new(
                    None,
                    Some(Action::Idle),
                    CommitPolicy::ContinuouslyScored,
                    0.1,
                    &[],
                ),
            ],
        };
        let pools = TargetPools {
            focus: Some(1),
            hostiles: &[1, 2],
        };
        assert_eq!(
            assign_behaviors_to_slots(&FOCUSED, &pools, &read_per_target, &all, &[], |_| true),
            vec![
                Pick {
                    behavior: 0,
                    target: Some(1),
                },
                Pick {
                    behavior: 2,
                    target: None,
                },
            ],
            "the chase fixes target 1, so the bite may not take the nearer target 2"
        );
    }

    #[test]
    fn running_committed_target_holds_the_decision() {
        static SEEDED: BrainDefinition = BrainDefinition {
            behaviors: &[
                Behavior::new(
                    Some(Motion::Dash),
                    None,
                    CommitPolicy::UntilFinished { interrupt_tier: 1 },
                    3.0,
                    &[HAS_TARGET],
                ),
                Behavior::new(
                    None,
                    Some(Action::Ability(Ability::Bite)),
                    CommitPolicy::ContinuouslyScored,
                    2.0,
                    &[NEAR],
                )
                .with_targets(Targets::Hostiles),
            ],
        };
        let pools = TargetPools {
            focus: Some(2),
            hostiles: &[2, 3],
        };
        let running = [Pick {
            behavior: 0,
            target: Some(3),
        }];
        assert_eq!(
            assign_behaviors_to_slots(&SEEDED, &pools, &read_per_target, &all, &running, |_| true),
            vec![Pick {
                behavior: 1,
                target: Some(3),
            }],
            "the running dash against 3 holds the bite to 3, though the focus 2 comes first"
        );
    }
}

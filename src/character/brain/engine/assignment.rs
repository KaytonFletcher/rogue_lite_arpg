//! Fills the motion and action slots: skips what may not run, scores the rest, and
//! takes winners from the highest score down.

use super::behavior::{Behavior, BrainDefinition, MAX_BEHAVIORS};
use super::scoring::score_behavior;
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

/// Scores every behavior not skipped (see [`SkipReason`]), then takes them
/// from the highest score down while their slots are free; ties go to the
/// earlier behavior. A `false` from `start` drops a winner for the next best.
/// `read` answers each consideration's input and `available` whether a behavior
/// may start now. Behavior ids, taken and returned, are indices into
/// `definition.behaviors`.
pub fn assign_behaviors_to_slots(
    definition: &BrainDefinition,
    read: &impl Fn(ConsiderationInput) -> Option<f32>,
    available: &impl Fn(&Behavior) -> bool,
    running_behavior_ids: &[usize],
    mut start: impl FnMut(usize) -> bool,
) -> Vec<usize> {
    let mut scores = [0.0_f32; MAX_BEHAVIORS];
    for (index, behavior) in definition.behaviors.iter().enumerate() {
        if find_skip_reason(definition, behavior, available, running_behavior_ids).is_some() {
            continue;
        }
        let is_current = running_behavior_ids.contains(&index);
        let Some(score) = score_behavior(behavior, read, is_current) else {
            continue;
        };
        scores[index] = score * momentum_multiplier(is_current);
    }

    let mut winning_behavior_ids: Vec<usize> = Vec::with_capacity(2);
    loop {
        let mut best: Option<usize> = None;
        for (index, behavior) in definition.behaviors.iter().enumerate() {
            let taken = winning_behavior_ids.iter().any(|&winning_behavior_id| {
                definition.behaviors[winning_behavior_id].shares_a_slot_with(behavior)
            });
            if scores[index] <= 0.0 || taken {
                continue;
            }
            if best.is_none_or(|leader| scores[index] > scores[leader]) {
                best = Some(index);
            }
        }
        let Some(index) = best else {
            return winning_behavior_ids;
        };
        if !start(index) {
            scores[index] = 0.0;
            continue;
        }
        winning_behavior_ids.push(index);
    }
}

pub(super) fn find_skip_reason(
    definition: &BrainDefinition,
    behavior: &Behavior,
    available: &impl Fn(&Behavior) -> bool,
    running_behavior_ids: &[usize],
) -> Option<SkipReason> {
    let tier = behavior.commit.interrupt_tier();
    let slot_committed = running_behavior_ids.iter().any(|&running_behavior_id| {
        let running_behavior = &definition.behaviors[running_behavior_id];
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
    use crate::character::brain::considerations::distance_threshold;
    use crate::character::brain::{CommitPolicy, Motion};

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
                &[distance_threshold("near", 100.0, 120.0)],
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

    /// Only wander and chase may start.
    fn available(behavior: &Behavior) -> bool {
        matches!(behavior.motion, Some(Motion::Wander | Motion::Chase))
    }

    fn read_distance(distance: f32) -> impl Fn(ConsiderationInput) -> Option<f32> {
        move |input| match input {
            ConsiderationInput::TargetDistance => Some(distance),
            _ => Some(1.0),
        }
    }

    #[test]
    fn exit_threshold_keeps_a_running_chase() {
        let read = read_distance(110.0);
        assert_eq!(
            assign_behaviors_to_slots(&BRAIN, &read, &available, &[], |_| true),
            vec![0],
            "110 is past the 100 enter threshold, so wander wins"
        );
        assert_eq!(
            assign_behaviors_to_slots(&BRAIN, &read, &available, &[1], |_| true),
            vec![1],
            "the 120 exit threshold holds a running chase"
        );
    }

    #[test]
    fn rejected_start_hands_the_slot_to_the_next_best() {
        let read = read_distance(50.0);
        assert_eq!(
            assign_behaviors_to_slots(&BRAIN, &read, &available, &[], |index| index != 1),
            vec![0],
            "chase wins on score but fails its start, so wander takes the slot"
        );
    }

    #[test]
    fn running_committed_behavior_yields_only_to_a_higher_tier() {
        let read = read_distance(50.0);
        let all = |_: &Behavior| true;
        assert_eq!(
            assign_behaviors_to_slots(&BRAIN, &read, &all, &[2], |index| index != 3),
            Vec::<usize>::new(),
            "a running tier-1 dash keeps the motion slot from tiers 0 and 1"
        );
        assert_eq!(
            assign_behaviors_to_slots(&BRAIN, &read, &all, &[2], |_| true),
            vec![3],
            "a tier-2 swoop takes the slot despite its lower score"
        );
    }
}

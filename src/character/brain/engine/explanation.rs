//! Debug explanations of how a behavior scored, computed from the same inputs as
//! slot assignment.

use super::assignment::{SkipReason, find_skip_reason, momentum_multiplier};
use super::behavior::{Behavior, BrainDefinition};
use super::scoring::{combine, score_consideration};
use crate::character::brain::ConsiderationInput;

/// One consideration's part in a score. `raw` is `None` when the input is
/// absent; `score` is `None` when the score mapping rejects the raw value.
#[derive(Clone, Copy, Debug)]
pub struct ConsiderationScoreExplanation {
    pub name: &'static str,
    pub input: ConsiderationInput,
    pub raw: Option<f32>,
    pub score: Option<f32>,
}

/// How one behavior scored, for the debug trace: why it sat out, or its final
/// score with `momentum` applied (`None` when invalid).
#[derive(Clone, Debug)]
pub struct BehaviorScoreExplanation {
    pub skipped: Option<SkipReason>,
    pub score: Option<f32>,
    pub momentum: f32,
    pub considerations: Vec<ConsiderationScoreExplanation>,
}

/// Explains behavior `index` exactly as
/// [`assign_behaviors_to_slots`](super::assignment::assign_behaviors_to_slots) scores it from the
/// same inputs, reading each consideration's input once.
pub fn explain_behavior(
    definition: &BrainDefinition,
    index: usize,
    read: &impl Fn(ConsiderationInput) -> Option<f32>,
    available: &impl Fn(&Behavior) -> bool,
    running_behavior_ids: &[usize],
) -> BehaviorScoreExplanation {
    let behavior = &definition.behaviors[index];
    let is_current = running_behavior_ids.contains(&index);
    let momentum = momentum_multiplier(is_current);
    let skipped =
        find_skip_reason(definition, behavior, available, running_behavior_ids).map(|reason| {
            // The committed holder sits out too, but it is the one running.
            match reason {
                SkipReason::SlotCommitted if is_current => SkipReason::RunningCommitted,
                other => other,
            }
        });
    if skipped.is_some() {
        return BehaviorScoreExplanation {
            skipped,
            score: None,
            momentum,
            considerations: Vec::new(),
        };
    }
    let considerations: Vec<_> = behavior
        .considerations
        .iter()
        .map(|consideration| {
            let raw = read(consideration.input);
            ConsiderationScoreExplanation {
                name: consideration.name,
                input: consideration.input,
                raw,
                score: score_consideration(consideration, raw, is_current),
            }
        })
        .collect();
    BehaviorScoreExplanation {
        skipped,
        score: combine(
            behavior.weight,
            considerations.iter().map(|explanation| explanation.score),
        )
        .map(|score| score * momentum),
        momentum,
        considerations,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::character::brain::{CommitPolicy, Motion};

    static BRAIN: BrainDefinition = BrainDefinition {
        behaviors: &[
            Behavior::new(
                Some(Motion::Chase),
                None,
                CommitPolicy::ContinuouslyScored,
                2.0,
                &[],
            ),
            Behavior::new(
                Some(Motion::Dash),
                None,
                CommitPolicy::UntilFinished { interrupt_tier: 1 },
                3.0,
                &[],
            ),
        ],
    };

    /// Everything but the dash may start.
    fn available(behavior: &Behavior) -> bool {
        behavior.motion != Some(Motion::Dash)
    }

    #[test]
    fn explanation_matches_the_assigned_score() {
        let read = |_: ConsiderationInput| Some(1.0);
        let chase = explain_behavior(&BRAIN, 0, &read, &available, &[0]);
        let score = chase.score.expect("a running chase scores");
        let momentum = momentum_multiplier(true);
        assert!(
            (score - 2.0 * momentum).abs() < 1e-5,
            "weight 2 with momentum, got {score}"
        );
        assert!(
            (chase.momentum - momentum).abs() < f32::EPSILON,
            "a running behavior reports its momentum"
        );
        let dash = explain_behavior(&BRAIN, 1, &read, &available, &[0]);
        assert_eq!(
            dash.skipped,
            Some(SkipReason::Unavailable),
            "the dash may not start"
        );
        assert_eq!(dash.score, None, "a skipped behavior has no score");
    }
}

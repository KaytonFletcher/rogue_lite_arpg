//! Debug explanations of how a behavior scored, computed from the same inputs as
//! slot assignment.

use super::assignment::{SkipReason, find_skip_reason, momentum_multiplier};
use super::behavior::{Behavior, BrainDefinition};
use super::scoring::{combine, score_consideration};
use super::targets::{Pick, TargetPools};
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

/// How one behavior scored against its best target, for the debug trace: why it
/// sat out, or its final score with `momentum` applied (`None` when invalid).
#[derive(Clone, Debug)]
pub struct BehaviorScoreExplanation<T> {
    pub skipped: Option<SkipReason>,
    pub target: Option<T>,
    pub score: Option<f32>,
    pub momentum: f32,
    pub considerations: Vec<ConsiderationScoreExplanation>,
}

/// Explains behavior `index` exactly as
/// [`assign_behaviors_to_slots`](super::assignment::assign_behaviors_to_slots) scores it from the
/// same inputs, against the target it scores best on, reading each input once.
pub fn explain_behavior<T: Copy + PartialEq>(
    definition: &BrainDefinition,
    index: usize,
    pools: &TargetPools<T>,
    read: &impl Fn(ConsiderationInput, Option<T>) -> Option<f32>,
    available: &impl Fn(&Behavior) -> bool,
    running: &[Pick<T>],
) -> BehaviorScoreExplanation<T> {
    let behavior = &definition.behaviors[index];
    let running_here = running.iter().any(|running| running.behavior == index);
    let skipped = find_skip_reason(definition, behavior, available, running).map(|reason| {
        // The committed holder sits out too, but it is the one running.
        match reason {
            SkipReason::SlotCommitted if running_here => SkipReason::RunningCommitted,
            other => other,
        }
    });
    if skipped.is_some() {
        return BehaviorScoreExplanation {
            skipped,
            target: None,
            score: None,
            momentum: momentum_multiplier(running_here),
            considerations: Vec::new(),
        };
    }
    let mut best: Option<BehaviorScoreExplanation<T>> = None;
    for target in pools.candidates(behavior) {
        let is_current = running.contains(&Pick {
            behavior: index,
            target,
        });
        let momentum = momentum_multiplier(is_current);
        let considerations: Vec<_> = behavior
            .considerations
            .iter()
            .map(|consideration| {
                let raw = read(consideration.input, target);
                ConsiderationScoreExplanation {
                    name: consideration.name,
                    input: consideration.input,
                    raw,
                    score: score_consideration(consideration, raw, is_current),
                }
            })
            .collect();
        let score = combine(
            behavior.weight,
            considerations.iter().map(|explanation| explanation.score),
        )
        .map(|score| score * momentum);
        if best.as_ref().is_none_or(|leader| score > leader.score) {
            best = Some(BehaviorScoreExplanation {
                skipped: None,
                target,
                score,
                momentum,
                considerations,
            });
        }
    }
    best.expect("every pool yields at least one target")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::character::brain::considerations::consideration;
    use crate::character::brain::{CommitPolicy, Curve, Interval, Motion, Targets};

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

    const NO_TARGETS: TargetPools<u8> = TargetPools {
        focus: None,
        hostiles: &[],
    };

    #[test]
    fn explanation_matches_the_assigned_score() {
        let read = |_: ConsiderationInput, _: Option<u8>| Some(1.0);
        let running = [Pick {
            behavior: 0,
            target: None,
        }];
        let chase = explain_behavior(&BRAIN, 0, &NO_TARGETS, &read, &available, &running);
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
        let dash = explain_behavior(&BRAIN, 1, &NO_TARGETS, &read, &available, &running);
        assert_eq!(
            dash.skipped,
            Some(SkipReason::Unavailable),
            "the dash may not start"
        );
        assert_eq!(dash.score, None, "a skipped behavior has no score");
    }

    static HOSTILE_CHASE: BrainDefinition = BrainDefinition {
        behaviors: &[Behavior::new(
            Some(Motion::Chase),
            None,
            CommitPolicy::ContinuouslyScored,
            2.0,
            &[consideration(
                "closeness",
                ConsiderationInput::TargetDistance,
                Interval::new(0.0, 1.0),
                Curve::Linear,
            )],
        )
        .with_targets(Targets::Hostiles)],
    };

    #[test]
    fn explanation_reports_the_best_target() {
        let pools = TargetPools {
            focus: Some(1_u8),
            hostiles: &[1, 2],
        };
        let read = |_: ConsiderationInput, target: Option<u8>| match target {
            Some(1) => Some(0.2),
            _ => Some(0.8),
        };
        let chase = explain_behavior(&HOSTILE_CHASE, 0, &pools, &read, &|_: &Behavior| true, &[]);
        assert_eq!(chase.target, Some(2), "target 2 scores higher");
        let score = chase.score.expect("both targets score");
        assert!((score - 1.6).abs() < 1e-5, "2 x 0.8, got {score}");
    }
}

//! How a consideration turns one raw input into a `0.0..=1.0` score.

use super::behavior::Behavior;
use crate::character::brain::ConsiderationInput;

/// An ordered `min < max` interval: a distance range, a consideration's input
/// range, or a span of seconds to roll from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Interval {
    min: f32,
    max: f32,
}

impl Interval {
    /// Builds an interval, rejecting bad bounds at compile time.
    pub const fn new(min: f32, max: f32) -> Self {
        assert!(
            min.is_finite() && max.is_finite() && min < max,
            "interval needs finite min < max"
        );
        Self { min, max }
    }

    pub const fn min(self) -> f32 {
        self.min
    }

    pub const fn max(self) -> f32 {
        self.max
    }

    /// Signed distance outside the interval; zero inside it.
    pub fn error(self, value: f32) -> f32 {
        if value < self.min {
            value - self.min
        } else if value > self.max {
            value - self.max
        } else {
            0.0
        }
    }

    /// Offset from the center as a fraction of the half-width, clamped to `-1.0..=1.0`.
    pub fn center_error(self, value: f32) -> f32 {
        let half_width = (self.max - self.min) * 0.5;
        ((value - self.min.midpoint(self.max)) / half_width).clamp(-1.0, 1.0)
    }

    /// Maps `raw` onto `0.0..=1.0` across the interval, clamping outside it.
    pub fn normalize_clamped(self, raw: f32) -> Option<f32> {
        if !raw.is_finite() {
            return None;
        }
        let normalized = ((raw - self.min) / (self.max - self.min)).clamp(0.0, 1.0);
        normalized.is_finite().then_some(normalized)
    }
}

/// One normalized input to a behavior's score.
#[derive(Clone, Copy, Debug)]
pub struct Consideration {
    pub name: &'static str,
    pub input: ConsiderationInput,
    pub score_mapping: ScoreMapping,
    pub inverted: bool,
}

impl Consideration {
    /// The opposite fact under a new name. A missing input still vetoes.
    pub const fn inverted(self, name: &'static str) -> Self {
        Self {
            name,
            inverted: !self.inverted,
            ..self
        }
    }
}

/// How a consideration turns its raw input into a `0.0..=1.0` score.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ScoreMapping {
    /// Normalize over `range`, then shape with `curve`.
    Curve { range: Interval, curve: Curve },
    /// Full once the raw input crosses `enter`, zero before it. While its
    /// behavior holds a slot the threshold is `exit` instead, so the gap keeps
    /// the behavior. `exit < enter` passes raw inputs at or above the threshold;
    /// otherwise, including no gap at all, it passes raw inputs at or below it.
    Threshold { enter: f32, exit: f32 },
}

impl ScoreMapping {
    /// Score for `raw`, or `None` for a non-finite raw input.
    pub fn score_raw_input(self, raw: f32, is_current: bool) -> Option<f32> {
        match self {
            Self::Curve { range, curve } => {
                range.normalize_clamped(raw).map(|value| curve.apply(value))
            }
            Self::Threshold { enter, exit } => {
                if !raw.is_finite() {
                    return None;
                }
                let threshold = if is_current { exit } else { enter };
                let passes = if exit < enter {
                    raw >= threshold
                } else {
                    raw <= threshold
                };
                Some(if passes { 1.0 } else { 0.0 })
            }
        }
    }
}

/// Curve applied after normalizing a consideration's raw value.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Curve {
    Linear,
    /// Steepens a partial input (`x^3`).
    Cubic,
}

impl Curve {
    pub(crate) fn apply(self, normalized: f32) -> f32 {
        let value = match self {
            Self::Linear => normalized,
            Self::Cubic => normalized * normalized * normalized,
        };

        value.clamp(0.0, 1.0)
    }
}

/// Final bounded score for one behavior, reading each consideration's input
/// through `read`. `is_current` switches threshold considerations to their exit side.
pub fn score_behavior(
    behavior: &Behavior,
    read: &impl Fn(ConsiderationInput) -> Option<f32>,
    is_current: bool,
) -> Option<f32> {
    combine(
        behavior.weight,
        behavior.considerations.iter().map(|consideration| {
            score_consideration(consideration, read(consideration.input), is_current)
        }),
    )
}

/// Weight times the geometric mean of `scores`; `None` when any score is
/// `None`. A zero anywhere vetoes, but each passing flag raises graded inputs
/// to a higher root, so adding a flag shifts a behavior's crossovers. Brains
/// are tuned against that root.
pub(super) fn combine(weight: f32, scores: impl IntoIterator<Item = Option<f32>>) -> Option<f32> {
    let mut product = 1.0;
    let mut count = 0;
    for score in scores {
        product *= score?;
        count += 1;
    }
    let mut score = weight;
    if count > 0 {
        score *= product.powf(1.0 / (count as f32));
    }

    (score.is_finite() && score >= 0.0).then_some(score)
}

/// Scores one consideration from its `raw` input; `None` when the score
/// mapping rejects a non-finite raw value.
pub(super) fn score_consideration(
    consideration: &Consideration,
    raw: Option<f32>,
    is_current: bool,
) -> Option<f32> {
    // A missing input (no target, no home area) vetoes, inverted or not.
    let Some(raw) = raw else {
        return Some(0.0);
    };
    let score = consideration
        .score_mapping
        .score_raw_input(raw, is_current)?;
    Some(if consideration.inverted {
        1.0 - score
    } else {
        score
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::character::brain::considerations::{consideration, flag};
    use crate::character::brain::{CommitPolicy, Motion};

    const FLAG: Consideration = flag("flag", ConsiderationInput::HasTarget);
    const GRADED: Consideration = consideration(
        "graded",
        ConsiderationInput::TargetDistance,
        Interval::new(0.0, 1.0),
        Curve::Linear,
    );

    fn behavior(considerations: &'static [Consideration]) -> Behavior {
        Behavior::new(
            Some(Motion::Chase),
            None,
            CommitPolicy::ContinuouslyScored,
            8.0,
            considerations,
        )
    }

    #[test]
    fn score_is_weight_times_geometric_mean() {
        let read = |input: ConsiderationInput| match input {
            ConsiderationInput::TargetDistance => Some(0.125),
            _ => Some(1.0),
        };
        let score = score_behavior(&behavior(&[FLAG, FLAG, GRADED]), &read, false)
            .expect("finite inputs score");
        assert!(
            (score - 4.0).abs() < 1e-4,
            "8 x 0.125^(1/3) is 4, got {score}"
        );
    }

    #[test]
    fn missing_input_vetoes_even_when_inverted() {
        let read = |_: ConsiderationInput| None;
        let inverted = FLAG.inverted("no flag");
        assert_eq!(
            score_consideration(&inverted, read(inverted.input), false),
            Some(0.0),
            "a missing input scores zero, inverted or not"
        );
        assert_eq!(
            score_consideration(&inverted, Some(0.0), false),
            Some(1.0),
            "a present input inverts"
        );
        let score = score_behavior(&behavior(&[FLAG, GRADED]), &read, false)
            .expect("a veto is a zero score, not an invalid one");
        assert!(score.abs() < f32::EPSILON, "one zero vetoes the behavior");
    }

    #[test]
    fn non_finite_input_invalidates_the_score() {
        let read = |_: ConsiderationInput| Some(f32::NAN);
        assert_eq!(
            score_behavior(&behavior(&[GRADED]), &read, false),
            None,
            "a non-finite raw input makes the whole score invalid"
        );
    }

    #[test]
    fn rising_threshold_holds_while_current() {
        let mapping = ScoreMapping::Threshold {
            enter: 100.0,
            exit: 120.0,
        };
        assert_eq!(
            mapping.score_raw_input(110.0, false),
            Some(0.0),
            "past enter"
        );
        assert_eq!(
            mapping.score_raw_input(110.0, true),
            Some(1.0),
            "inside exit"
        );
    }

    #[test]
    fn falling_threshold_passes_at_or_above() {
        // `exit < enter` passes raw inputs at or above the threshold.
        let mapping = ScoreMapping::Threshold {
            enter: 1.0,
            exit: 0.94,
        };
        assert_eq!(
            mapping.score_raw_input(0.97, false),
            Some(0.0),
            "below enter"
        );
        assert_eq!(mapping.score_raw_input(1.0, false), Some(1.0), "at enter");
        assert_eq!(mapping.score_raw_input(0.97, true), Some(1.0), "above exit");
    }
}

//! Publishes brain and decision counts as `game/ai/…` diagnostics.

use std::sync::atomic::{AtomicU32, Ordering};

use bevy::diagnostic::{Diagnostic, DiagnosticPath, Diagnostics, RegisterDiagnostic};
use bevy::prelude::*;

use super::{Brain, Targeting};
use crate::prelude::*;

const BRAINS: DiagnosticPath = DiagnosticPath::const_new("game/ai/brains");
/// Brains with a target. More of them means a heavier fight.
const ENGAGED: DiagnosticPath = DiagnosticPath::const_new("game/ai/engaged");
const DECISIONS: DiagnosticPath = DiagnosticPath::const_new("game/ai/decisions");
const REDECISIONS: DiagnosticPath = DiagnosticPath::const_new("game/ai/redecisions");

/// Decisions made since the last frame was published. Atomic, so parallel
/// decisions can count through a shared reference.
#[derive(Resource, Debug, Default)]
pub(super) struct DecisionStats {
    paced_decisions: AtomicU32,
    redecisions: AtomicU32,
}

impl DecisionStats {
    /// Counts a decision made on a brain's own schedule.
    pub(super) fn record_paced_decision(&self) {
        self.paced_decisions.fetch_add(1, Ordering::Relaxed);
    }

    /// Counts a decision an observer made immediately, outside the schedule.
    pub(super) fn record_redecision(&self) {
        self.redecisions.fetch_add(1, Ordering::Relaxed);
    }
}

pub(super) fn plugin(app: &mut App) {
    app.init_resource::<DecisionStats>();
    for path in [BRAINS, ENGAGED, DECISIONS, REDECISIONS] {
        app.register_diagnostic(Diagnostic::new(path).with_smoothing_factor(0.0));
    }
    app.add_systems(
        Update,
        (measure_brains, publish_decision_stats).in_set(InGameSystems::Measure),
    );
}

fn measure_brains(
    mut diagnostics: Diagnostics,
    brains: Query<(), With<Brain>>,
    engaged_brains: Query<(), (With<Brain>, With<Targeting>)>,
) {
    diagnostics.add_measurement(&BRAINS, || brains.count() as f64);
    diagnostics.add_measurement(&ENGAGED, || engaged_brains.count() as f64);
}

/// Publishes the decisions made since the previous frame, then resets the counts.
fn publish_decision_stats(mut diagnostics: Diagnostics, decision_stats: Res<DecisionStats>) {
    let decisions = f64::from(decision_stats.paced_decisions.swap(0, Ordering::Relaxed));
    let redecisions = f64::from(decision_stats.redecisions.swap(0, Ordering::Relaxed));
    diagnostics.add_measurement(&DECISIONS, || decisions);
    diagnostics.add_measurement(&REDECISIONS, || redecisions);
}

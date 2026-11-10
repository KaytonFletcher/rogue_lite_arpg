//! What an entity can do: the per-motion-and-action table, the [`Capability`]
//! components that carry motions and actions out, and the [`Capabilities`]
//! cache of their profiles that decisions read.
//!
//! To add a capability: implement [`Capability`] for its component and register
//! it in [`plugin`]. Nothing in the decision code changes.

use bevy::prelude::*;

use super::action::{Ability, Action, Bite, Breath, Heal};
use super::motion::{Dash, Motion, Orbit, Swoop, Wander};

/// A type with its own entry in a [`MotionActionTable`].
pub trait MotionActionKey: Copy {
    fn table_index(self) -> usize;
}

/// Motions come first in a [`MotionActionTable`].
impl MotionActionKey for Motion {
    fn table_index(self) -> usize {
        self.index()
    }
}

/// Actions follow the motions in a [`MotionActionTable`].
impl MotionActionKey for Action {
    fn table_index(self) -> usize {
        Motion::COUNT + self.index()
    }
}

impl MotionActionKey for Ability {
    fn table_index(self) -> usize {
        Action::Ability(self).table_index()
    }
}

/// One value per motion and per action, indexed by any [`MotionActionKey`].
#[derive(Clone, Copy, Debug)]
pub struct MotionActionTable<T>([T; Motion::COUNT + Action::COUNT]);

impl<T: Default> Default for MotionActionTable<T> {
    fn default() -> Self {
        Self(std::array::from_fn(|_| T::default()))
    }
}

impl<T, K: MotionActionKey> std::ops::Index<K> for MotionActionTable<T> {
    type Output = T;

    fn index(&self, key: K) -> &T {
        &self.0[key.table_index()]
    }
}

impl<T, K: MotionActionKey> std::ops::IndexMut<K> for MotionActionTable<T> {
    fn index_mut(&mut self, key: K) -> &mut T {
        &mut self.0[key.table_index()]
    }
}

/// What the decision needs to know about one capability, cached in
/// [`Capabilities`] from the component that carries it out.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CapabilityProfile {
    pub reach: Option<f32>,
    /// Probability that a winning finite motion actually starts.
    pub chance: f32,
    /// Seconds to wait after a finish or a missed start roll.
    pub cooldown: f32,
}

impl Default for CapabilityProfile {
    fn default() -> Self {
        Self {
            reach: None,
            chance: 1.0,
            cooldown: 0.0,
        }
    }
}

/// The motion or action a capability component carries out.
#[derive(Clone, Copy, Debug)]
pub enum CapabilityKey {
    Motion(Motion),
    Action(Action),
}

impl MotionActionKey for CapabilityKey {
    fn table_index(self) -> usize {
        match self {
            Self::Motion(motion) => motion.table_index(),
            Self::Action(action) => action.table_index(),
        }
    }
}

/// A component that lets its entity run one motion or action.
///
/// Its [`profile`](Self::profile) is cached in [`Capabilities`] when the
/// component is inserted and cleared when it is removed, so it must not change
/// in place: insert a new component to change it. A component with no running
/// state is `#[component(immutable)]` to enforce that; one with running state
/// keeps its profile private.
pub trait Capability: Component {
    /// The table entry this component fills. A payload in a `Motion` key only
    /// stands in, since the table indexes by variant.
    const KEY: CapabilityKey;

    /// What a decision reads of this capability.
    fn profile(&self) -> CapabilityProfile {
        CapabilityProfile::default()
    }
}

/// The profile of each motion and action this entity can run, `None` for the
/// ones it can't. Motions steered without a component and `Action::Idle` are
/// always owned; the rest are filled from [`Capability`] components. Only their
/// observers write it, so it is never removed on its own: a re-added table
/// would forget the components already on the entity.
#[derive(Component, Clone, Copy, Debug)]
pub struct Capabilities(MotionActionTable<Option<CapabilityProfile>>);

impl Default for Capabilities {
    fn default() -> Self {
        let mut table = MotionActionTable::<Option<CapabilityProfile>>::default();
        for &motion in Motion::ALL {
            let always_owned = match motion {
                Motion::Chase
                | Motion::ReturnToHome
                | Motion::Flee
                | Motion::KeepDistance(_)
                | Motion::Idle => true,
                Motion::Wander | Motion::Orbit(_) | Motion::Dash | Motion::Swoop => false,
            };
            if always_owned {
                table[motion] = Some(CapabilityProfile::default());
            }
        }
        table[Action::Idle] = Some(CapabilityProfile::default());
        Self(table)
    }
}

impl Capabilities {
    /// The profile of the capability that carries out `key`; `None` when the
    /// entity lacks it.
    pub fn get(&self, key: impl MotionActionKey) -> Option<CapabilityProfile> {
        self.0[key]
    }

    fn set(&mut self, key: impl MotionActionKey, profile: Option<CapabilityProfile>) {
        self.0[key] = profile;
    }
}

pub(super) fn plugin(app: &mut App) {
    register::<Wander>(app);
    register::<Orbit>(app);
    register::<Dash>(app);
    register::<Swoop>(app);
    register::<Heal>(app);
    register::<Breath>(app);
    register::<Bite>(app);
}

/// Keeps `C`'s entry in [`Capabilities`] in step with the component, and makes
/// `C` bring a [`Capabilities`] so the entry has somewhere to go.
fn register<C: Capability>(app: &mut App) {
    app.register_required_components::<C, Capabilities>();
    app.add_observer(cache_profile::<C>)
        .add_observer(clear_profile::<C>);
}

fn cache_profile<C: Capability>(
    inserted: On<Insert<C>>,
    mut owners: Query<(&C, &mut Capabilities)>,
) {
    if let Ok((capability, mut capabilities)) = owners.get_mut(inserted.entity) {
        capabilities.set(C::KEY, Some(capability.profile()));
    }
}

fn clear_profile<C: Capability>(removed: On<Remove<C>>, mut owners: Query<&mut Capabilities>) {
    if let Ok(mut capabilities) = owners.get_mut(removed.entity) {
        capabilities.set(C::KEY, None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> App {
        let mut app = App::new();
        plugin(&mut app);
        app.world_mut().flush();
        app
    }

    #[test]
    fn steered_motions_and_idle_are_always_owned() {
        let capabilities = Capabilities::default();
        assert!(
            capabilities.get(Motion::Chase).is_some(),
            "chase needs no component"
        );
        assert!(
            capabilities.get(Action::Idle).is_some(),
            "idle needs no capability"
        );
        assert!(
            capabilities.get(Motion::Dash).is_none(),
            "dash needs its component"
        );
        assert!(
            capabilities.get(Ability::Bite).is_none(),
            "bite needs its component"
        );
    }

    #[test]
    fn inserting_a_capability_caches_its_profile() {
        let mut app = app();
        let bite = Bite::new(30.0, 5.0, 2.0);
        let entity = app.world_mut().spawn(bite).id();
        let capabilities = app
            .world()
            .get::<Capabilities>(entity)
            .expect("a capability brings its table");
        assert_eq!(
            capabilities.get(Ability::Bite),
            Some(bite.profile()),
            "the observer caches the inserted profile"
        );
    }

    #[test]
    fn reinserting_replaces_and_removing_clears() {
        let mut app = app();
        let entity = app.world_mut().spawn(Bite::new(30.0, 5.0, 2.0)).id();
        let longer = Bite::new(60.0, 5.0, 2.0);
        app.world_mut().entity_mut(entity).insert(longer);
        let reach = |app: &App| {
            app.world()
                .get::<Capabilities>(entity)
                .and_then(|capabilities| capabilities.get(Ability::Bite))
                .and_then(|profile| profile.reach)
        };
        assert_eq!(reach(&app), Some(60.0), "a re-insert replaces the profile");
        app.world_mut().entity_mut(entity).remove::<Bite>();
        assert_eq!(reach(&app), None, "a removal clears it");
    }
}

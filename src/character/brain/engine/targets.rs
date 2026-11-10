//! Which targets a behavior scores against. The core treats a target as an opaque
//! handle `T`; the game passes entities.

/// Most targets one behavior scores, so scoring needs no allocation.
pub const MAX_TARGETS: usize = 8;

/// The pool of targets a behavior scores, keeping the best.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Targets {
    /// The brain's focus target, if it has one.
    Focus,
    /// Every hostile the brain is aware of while it has a focus, the focus first.
    Hostiles,
}

/// One decision's targets.
#[derive(Clone, Copy, Debug)]
pub struct TargetPools<'a, T> {
    pub focus: Option<T>,
    pub hostiles: &'a [T],
}

impl<T: Copy> TargetPools<'_, T> {
    /// Each target a behavior over `targets` scores against, at most
    /// [`MAX_TARGETS`]. An empty pool scores once with no target, so a target
    /// input reads as absent and `NO_TARGET`-style considerations still pass.
    pub fn candidates(&self, targets: Targets) -> impl Iterator<Item = Option<T>> {
        let pool = match targets {
            Targets::Focus => self.focus.as_slice(),
            Targets::Hostiles => self.hostiles,
        };
        let pool = &pool[..pool.len().min(MAX_TARGETS)];
        pool.is_empty()
            .then_some(None)
            .into_iter()
            .chain(pool.iter().copied().map(Some))
    }
}

/// A behavior and the target it won or runs against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pick<T> {
    /// Index into the brain definition's behaviors.
    pub behavior: usize,
    pub target: Option<T>,
}

use super::Application;
use crate::scenario::definition::Builder;

/// Accessor trait for wrapper builders that delegate generic scenario behavior
/// to the core `Builder`.
#[doc(hidden)]
pub trait CoreBuilderAccess: Sized {
    type Env: Application;
    type Caps;

    fn map_core_builder(
        self,
        f: impl FnOnce(Builder<Self::Env, Self::Caps>) -> Builder<Self::Env, Self::Caps>,
    ) -> Self;

    fn core_builder_ref(&self) -> &Builder<Self::Env, Self::Caps>;

    fn core_builder_mut(&mut self) -> &mut Builder<Self::Env, Self::Caps>;
}

impl<E: Application, Caps> CoreBuilderAccess for Builder<E, Caps> {
    type Env = E;
    type Caps = Caps;

    fn map_core_builder(self, f: impl FnOnce(Builder<E, Caps>) -> Builder<E, Caps>) -> Self {
        f(self)
    }

    fn core_builder_ref(&self) -> &Builder<E, Caps> {
        self
    }

    fn core_builder_mut(&mut self) -> &mut Builder<E, Caps> {
        self
    }
}

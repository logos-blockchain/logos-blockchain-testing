use super::Application;
use crate::scenario::definition::Builder;

/// Accessor trait for wrapper builders that delegate generic scenario behavior
/// to the core `Builder`.
#[doc(hidden)]
pub trait CoreBuilderAccess: Sized {
    type Env: Application;

    fn map_core_builder(self, f: impl FnOnce(Builder<Self::Env>) -> Builder<Self::Env>) -> Self;

    fn core_builder_ref(&self) -> &Builder<Self::Env>;

    fn core_builder_mut(&mut self) -> &mut Builder<Self::Env>;
}

impl<E: Application> CoreBuilderAccess for Builder<E> {
    type Env = E;

    fn map_core_builder(self, f: impl FnOnce(Builder<E>) -> Builder<E>) -> Self {
        f(self)
    }

    fn core_builder_ref(&self) -> &Builder<E> {
        self
    }

    fn core_builder_mut(&mut self) -> &mut Builder<E> {
        self
    }
}

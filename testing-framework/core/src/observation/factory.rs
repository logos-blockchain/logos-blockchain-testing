use std::{marker::PhantomData, sync::Arc};

use async_trait::async_trait;

use super::{
    ObservationConfig, ObservationHandle, ObservationRuntime, ObservedSource, Observer,
    SourceProvider,
};
use crate::scenario::{
    Application, DynError, NodeClients, PreparedRuntimeExtension, RuntimeExtensionFactory,
};

/// Boxed source provider used by observation factories.
pub type BoxedSourceProvider<S> = Box<dyn SourceProvider<S>>;

/// Builds an observation source provider once node clients are available.
pub trait SourceProviderFactory<E: Application, S>: Send + Sync + 'static {
    /// Builds the source provider for one scenario run.
    fn build_source_provider(
        &self,
        deployment: &E::Deployment,
        node_clients: NodeClients<E>,
    ) -> Result<BoxedSourceProvider<S>, DynError>;
}

impl<E, S, F> SourceProviderFactory<E, S> for F
where
    E: Application,
    S: Clone + Send + Sync + 'static,
    F: Fn(&E::Deployment, NodeClients<E>) -> Result<BoxedSourceProvider<S>, DynError>
        + Send
        + Sync
        + 'static,
{
    fn build_source_provider(
        &self,
        deployment: &E::Deployment,
        node_clients: NodeClients<E>,
    ) -> Result<BoxedSourceProvider<S>, DynError> {
        self(deployment, node_clients)
    }
}

/// Fixed source provider for scenario runs with a stable source set.
#[derive(Clone, Debug)]
pub struct StaticSourceProvider<S> {
    sources: Vec<ObservedSource<S>>,
}

impl<S> StaticSourceProvider<S> {
    /// Builds a provider from a fixed source list.
    #[must_use]
    pub fn new(sources: Vec<ObservedSource<S>>) -> Self {
        Self { sources }
    }
}

#[async_trait]
impl<S> SourceProvider<S> for StaticSourceProvider<S>
where
    S: Clone + Send + Sync + 'static,
{
    async fn sources(&self) -> Result<Vec<ObservedSource<S>>, DynError> {
        Ok(self.sources.clone())
    }
}

/// Runtime extension factory that starts one observer and stores its handle in
/// `RunContext`.
pub struct ObservationExtensionFactory<E: Application, O: Observer> {
    observer_builder: Arc<dyn Fn() -> O + Send + Sync>,
    source_provider_factory: Arc<dyn SourceProviderFactory<E, O::Source>>,
    config: ObservationConfig,
    env_marker: PhantomData<E>,
}

impl<E: Application, O: Observer> ObservationExtensionFactory<E, O> {
    /// Builds an observation extension factory from builders.
    #[must_use]
    pub fn from_parts(
        observer_builder: impl Fn() -> O + Send + Sync + 'static,
        source_provider_factory: impl SourceProviderFactory<E, O::Source>,
        config: ObservationConfig,
    ) -> Self {
        Self {
            observer_builder: Arc::new(observer_builder),
            source_provider_factory: Arc::new(source_provider_factory),
            config,
            env_marker: PhantomData,
        }
    }
}

impl<E, O> ObservationExtensionFactory<E, O>
where
    E: Application,
    O: Observer + Clone,
{
    /// Builds an observation extension factory from one clonable observer.
    #[must_use]
    pub fn new(
        observer: O,
        source_provider_factory: impl SourceProviderFactory<E, O::Source>,
        config: ObservationConfig,
    ) -> Self {
        Self::from_parts(move || observer.clone(), source_provider_factory, config)
    }
}

#[async_trait]
impl<E, O> RuntimeExtensionFactory<E> for ObservationExtensionFactory<E, O>
where
    E: Application,
    O: Observer,
{
    async fn prepare(
        &self,
        deployment: &E::Deployment,
        node_clients: NodeClients<E>,
    ) -> Result<PreparedRuntimeExtension, DynError> {
        let source_provider = self
            .source_provider_factory
            .build_source_provider(deployment, node_clients)?;

        let observer = (self.observer_builder)();
        let runtime =
            ObservationRuntime::start(source_provider, observer, self.config.clone()).await?;

        let (handle, task) = runtime.into_parts();

        Ok(PreparedRuntimeExtension::from_task(handle, task))
    }
}

#[async_trait]
impl<S, P> SourceProvider<S> for Box<P>
where
    S: Clone + Send + Sync + 'static,
    P: SourceProvider<S> + ?Sized,
{
    async fn sources(&self) -> Result<Vec<ObservedSource<S>>, DynError> {
        (**self).sources().await
    }
}

#[async_trait]
impl<S, P> SourceProvider<S> for Arc<P>
where
    S: Clone + Send + Sync + 'static,
    P: SourceProvider<S> + ?Sized,
{
    async fn sources(&self) -> Result<Vec<ObservedSource<S>>, DynError> {
        (**self).sources().await
    }
}

impl<O: Observer> From<ObservationHandle<O>> for PreparedRuntimeExtension {
    fn from(handle: ObservationHandle<O>) -> Self {
        PreparedRuntimeExtension::new(handle)
    }
}

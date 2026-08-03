use std::{
    future::Future,
    path::PathBuf,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use async_trait::async_trait;
use testing_framework_core::scenario::{Application, DynError, internal::CleanupGuard};
use testing_framework_runner_local::{LaunchSpec, NodeEndpoints, ProcessNode};
use tokio::sync::Mutex;

use crate::{AppDeployment, DeployContext};

type ReadinessFuture = Pin<Box<dyn Future<Output = Result<(), DynError>> + Send>>;
type Readiness<C> = Arc<dyn Fn(NodeEndpoints, C) -> ReadinessFuture + Send + Sync>;

/// One local binary process that can be composed with other app deployments.
///
/// This is the small path for heterogeneous stacks. It does not require the
/// process to pretend to be a uniform TF node cluster. The app repository owns
/// its launch files, client type, and readiness check; TF owns process
/// lifetime, working directories, and teardown.
pub struct LocalProcessApp<C> {
    label: String,
    launch: LaunchSpec,
    endpoints: NodeEndpoints,
    client: C,
    readiness: Option<Readiness<C>>,
    keep_tempdir: bool,
    persist_dir: Option<PathBuf>,
    snapshot_dir: Option<PathBuf>,
}

impl<C> LocalProcessApp<C>
where
    C: Clone + Send + Sync + 'static,
{
    /// Describes a process and the client returned through its runtime handle.
    ///
    /// `endpoints` describes the addresses assigned by the application-specific
    /// launch configuration; the generic process layer does not allocate them.
    #[must_use]
    pub fn new(
        label: impl Into<String>,
        launch: LaunchSpec,
        endpoints: NodeEndpoints,
        client: C,
    ) -> Self {
        Self {
            label: label.into(),
            launch,
            endpoints,
            client,
            readiness: None,
            keep_tempdir: false,
            persist_dir: None,
            snapshot_dir: None,
        }
    }

    /// Adds an asynchronous readiness check that runs after the process starts.
    ///
    /// A readiness failure stops the process and fails deployment.
    #[must_use]
    pub fn with_readiness<F, Fut>(mut self, readiness: F) -> Self
    where
        F: Fn(NodeEndpoints, C) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Result<(), DynError>> + Send + 'static,
    {
        self.readiness = Some(Arc::new(move |endpoints, client| {
            Box::pin(readiness(endpoints, client))
        }));
        self
    }

    /// Controls whether the generated process working directory survives
    /// teardown.
    #[must_use]
    pub fn keep_tempdir(mut self, keep: bool) -> Self {
        self.keep_tempdir = keep;
        self
    }

    /// Copies persisted state from `path` into the process working directory.
    #[must_use]
    pub fn with_persist_dir(mut self, path: impl Into<PathBuf>) -> Self {
        self.persist_dir = Some(path.into());
        self
    }

    /// Copies snapshot state from `path` into the process working directory.
    #[must_use]
    pub fn with_snapshot_dir(mut self, path: impl Into<PathBuf>) -> Self {
        self.snapshot_dir = Some(path.into());
        self
    }
}

#[async_trait]
impl<E, C> AppDeployment<E> for LocalProcessApp<C>
where
    E: Application,
    C: Clone + Send + Sync + 'static,
{
    type Handle = LocalProcessHandle<C>;

    async fn deploy(self, ctx: &mut DeployContext<E>) -> Result<Self::Handle, DynError> {
        let Self {
            label,
            launch,
            endpoints,
            client,
            readiness,
            keep_tempdir,
            persist_dir,
            snapshot_dir,
        } = self;

        let mut process = ProcessNode::spawn(
            &label,
            (),
            move |(), _working_dir, _label| Box::pin(async move { Ok(launch) }),
            move |()| Ok(endpoints),
            keep_tempdir,
            persist_dir.as_deref(),
            snapshot_dir.as_deref(),
            move |_endpoints| Ok(client),
        )
        .await?;

        let endpoints = process.endpoints().clone();
        let client = process.client();

        if let Some(readiness) = &readiness {
            if let Err(error) = readiness(endpoints.clone(), client.clone()).await {
                process.stop().await;
                return Err(error);
            }
        }

        let state = Arc::new(LocalProcessState {
            process: Mutex::new(process),
            endpoints: endpoints.clone(),
            client: client.clone(),
            readiness,
            closed: AtomicBool::new(false),
        });
        let handle = LocalProcessHandle {
            state,
            endpoints,
            client,
        };
        ctx.register_cleanup(handle.cleanup_guard());

        Ok(handle)
    }
}

struct LocalProcessState<C>
where
    C: Clone + Send + Sync + 'static,
{
    process: Mutex<ProcessNode<(), C>>,
    endpoints: NodeEndpoints,
    client: C,
    readiness: Option<Readiness<C>>,
    closed: AtomicBool,
}

impl<C> LocalProcessState<C>
where
    C: Clone + Send + Sync + 'static,
{
    fn ensure_open(&self) -> Result<(), DynError> {
        if self.closed.load(Ordering::Acquire) {
            Err("local process is no longer owned by an active run".into())
        } else {
            Ok(())
        }
    }

    fn close(&self) {
        if self.closed.swap(true, Ordering::AcqRel) {
            return;
        }

        if let Ok(mut process) = self.process.try_lock() {
            process.stop_blocking();
            return;
        }

        std::thread::scope(|scope| {
            let worker = scope.spawn(|| self.process.blocking_lock().stop_blocking());
            let _ = worker.join();
        });
    }

    async fn wait_ready(&self) -> Result<(), DynError> {
        if let Some(readiness) = &self.readiness {
            readiness(self.endpoints.clone(), self.client.clone()).await?;
        }
        Ok(())
    }
}

struct LocalProcessCleanup<C>
where
    C: Clone + Send + Sync + 'static,
{
    state: Arc<LocalProcessState<C>>,
}

impl<C> CleanupGuard for LocalProcessCleanup<C>
where
    C: Clone + Send + Sync + 'static,
{
    fn cleanup(self: Box<Self>) {
        self.state.close();
    }
}

/// Shared ownership and control handle for one local application process.
///
/// Clones refer to the same process. The scenario owns its lifetime, so cleanup
/// stops it even if a handle clone remains elsewhere.
pub struct LocalProcessHandle<C>
where
    C: Clone + Send + Sync + 'static,
{
    state: Arc<LocalProcessState<C>>,
    endpoints: NodeEndpoints,
    client: C,
}

impl<C> Clone for LocalProcessHandle<C>
where
    C: Clone + Send + Sync + 'static,
{
    fn clone(&self) -> Self {
        Self {
            state: Arc::clone(&self.state),
            endpoints: self.endpoints.clone(),
            client: self.client.clone(),
        }
    }
}

impl<C> LocalProcessHandle<C>
where
    C: Clone + Send + Sync + 'static,
{
    /// Returns a clone of the application-specific client.
    #[must_use]
    pub fn client(&self) -> C {
        self.client.clone()
    }

    /// Returns the process endpoints supplied at deployment time.
    #[must_use]
    pub fn endpoints(&self) -> &NodeEndpoints {
        &self.endpoints
    }

    /// Returns the operating-system process ID.
    pub async fn pid(&self) -> u32 {
        self.state.process.lock().await.pid()
    }

    /// Returns whether the child process is still running.
    pub async fn is_running(&self) -> bool {
        if self.state.closed.load(Ordering::Acquire) {
            return false;
        }
        self.state.process.lock().await.is_running()
    }

    /// Returns the generated working directory used by the process.
    pub async fn working_dir(&self) -> PathBuf {
        self.state.process.lock().await.working_dir().to_owned()
    }

    /// Starts a process that was previously stopped.
    pub async fn start(&self) -> Result<(), DynError> {
        self.state.ensure_open()?;
        let mut process = self.state.process.lock().await;
        if !process.is_running() {
            process.restart().await?;
        }
        drop(process);
        self.state.wait_ready().await?;
        Ok(())
    }

    /// Restarts the process with its original launch specification.
    pub async fn restart(&self) -> Result<(), DynError> {
        self.state.ensure_open()?;
        self.state.process.lock().await.restart().await?;
        self.state.wait_ready().await?;
        Ok(())
    }

    /// Stops the process without waiting for the handle to be dropped.
    pub async fn stop(&self) -> Result<(), DynError> {
        self.state.ensure_open()?;
        self.state.process.lock().await.stop().await;
        Ok(())
    }

    /// Prevents deletion of the temporary working directory on teardown.
    pub async fn keep_tempdir(&self) -> std::io::Result<()> {
        self.state.process.lock().await.keep_tempdir()
    }

    pub(crate) fn cleanup_guard(&self) -> Box<dyn CleanupGuard> {
        Box::new(LocalProcessCleanup {
            state: Arc::clone(&self.state),
        })
    }
}

#[cfg(all(test, unix))]
mod tests {
    use async_trait::async_trait;
    use testing_framework_core::{
        scenario::{Application, NodeClients},
        topology::DeploymentDescriptor,
    };
    use testing_framework_runner_local::{LaunchSpec, NodeEndpoints};

    use super::LocalProcessApp;
    use crate::DeployContext;

    #[derive(Clone)]
    struct TestDeployment;

    impl DeploymentDescriptor for TestDeployment {
        fn node_count(&self) -> usize {
            0
        }
    }

    struct TestEnv;

    #[async_trait]
    impl Application for TestEnv {
        type Deployment = TestDeployment;
        type NodeClient = ();
        type NodeConfig = ();
    }

    #[tokio::test]
    async fn process_lifetime_follows_scenario_ownership() {
        let launch = LaunchSpec {
            binary: "/bin/sh".into(),
            args: vec!["-c".into(), "sleep 30".into()],
            ..LaunchSpec::default()
        };
        let app = LocalProcessApp::new(
            "test-process",
            launch,
            NodeEndpoints::default(),
            "client".to_owned(),
        )
        .with_readiness(|_endpoints, client| async move {
            assert_eq!(client, "client");
            Ok(())
        });
        let mut ctx = DeployContext::<TestEnv>::new(TestDeployment, NodeClients::default());

        let handle = ctx.deploy(app).await.expect("deploy process");
        assert!(handle.is_running().await);
        assert_eq!(handle.client(), "client");

        handle.stop().await.unwrap();
        assert!(!handle.is_running().await);
        handle.start().await.unwrap();
        assert!(handle.is_running().await);

        drop(ctx);
        assert!(!handle.is_running().await);
        assert!(handle.restart().await.is_err());
    }
}

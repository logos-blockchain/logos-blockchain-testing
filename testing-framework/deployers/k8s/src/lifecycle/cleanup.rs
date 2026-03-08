use std::{io, process::Output, thread};

use k8s_openapi::api::core::v1::Namespace;
use kube::{Api, Client, api::DeleteParams};
use testing_framework_core::scenario::internal::CleanupGuard;
use tokio::{
    process::Command,
    runtime::{Handle, Runtime},
    time::{Duration, error::Elapsed, sleep, timeout},
};
use tracing::{info, warn};

use crate::infrastructure::helm::uninstall_release;

const CLEANUP_TIMEOUT: Duration = Duration::from_secs(120);
const NAMESPACE_DELETE_TIMEOUT: Duration = Duration::from_secs(10);

/// Tears down Helm release and namespace after a run unless preservation is
/// set.
pub struct RunnerCleanup {
    client: Client,
    namespace: String,
    release: String,
    preserve: bool,
}

impl RunnerCleanup {
    /// Build a cleanup guard; `preserve` skips deletion when true.
    pub fn new(client: Client, namespace: String, release: String, preserve: bool) -> Self {
        debug_assert!(
            !namespace.is_empty() && !release.is_empty(),
            "k8s cleanup requires namespace and release"
        );
        Self {
            client,
            namespace,
            release,
            preserve,
        }
    }

    async fn cleanup_async(&self) {
        if self.preserve {
            info!(
                release = %self.release,
                namespace = %self.namespace,
                "preserving k8s release and namespace"
            );
            return;
        }

        uninstall_release_and_namespace(&self.client, &self.release, &self.namespace).await;
    }

    fn blocking_cleanup_success(&self) -> bool {
        run_cleanup_with_runtime(self).unwrap_or_else(|error| {
            warn!(error = ?error, "unable to complete blocking cleanup; falling back to background thread");
            false
        })
    }

    fn spawn_cleanup_thread(self: Box<Self>) {
        match thread::Builder::new()
            .name("k8s-runner-cleanup".into())
            .spawn(move || run_background_cleanup(self))
        {
            Ok(handle) => {
                if let Err(err) = handle.join() {
                    warn!(error = ?err, "cleanup thread panicked");
                }
            }

            Err(err) => warn!(error = ?err, "failed to spawn cleanup thread"),
        }
    }
}

#[derive(Debug, thiserror::Error)]
enum BlockingCleanupError {
    #[error("failed to create tokio runtime: {source}")]
    RuntimeInit { source: io::Error },
}

fn run_cleanup_with_runtime(cleanup: &RunnerCleanup) -> Result<bool, BlockingCleanupError> {
    let runtime = Runtime::new().map_err(|source| BlockingCleanupError::RuntimeInit { source })?;
    let result =
        runtime.block_on(async { timeout(CLEANUP_TIMEOUT, cleanup.cleanup_async()).await });
    Ok(cleanup_completed(result))
}

async fn uninstall_release_and_namespace(client: &Client, release: &str, namespace: &str) {
    if let Err(err) = uninstall_release(release, namespace).await {
        warn!(release, namespace, error = ?err, "helm uninstall failed during cleanup");
    }

    info!(namespace, "deleting namespace via k8s API");
    delete_namespace(client, namespace).await;
    info!(namespace, "namespace delete request finished");
}

fn run_background_cleanup(cleanup: Box<RunnerCleanup>) {
    match Runtime::new() {
        Ok(rt) => {
            if let Err(err) =
                rt.block_on(async { timeout(CLEANUP_TIMEOUT, cleanup.cleanup_async()).await })
            {
                warn!(error = ?err, "background cleanup timed out");
            }
        }

        Err(err) => warn!(error = ?err, "unable to create cleanup runtime"),
    }
}

async fn delete_namespace(client: &Client, namespace: &str) {
    let namespaces: Api<Namespace> = Api::all(client.clone());
    let deleted = try_delete_namespace(&namespaces, namespace).await;

    if deleted {
        wait_for_namespace_termination(&namespaces, namespace).await;
    } else {
        warn!(
            namespace,
            "unable to delete namespace using kubectl fallback"
        );
    }
}

async fn try_delete_namespace(namespaces: &Api<Namespace>, namespace: &str) -> bool {
    if delete_namespace_via_api(namespaces, namespace).await {
        return true;
    }

    delete_namespace_via_cli(namespace).await
}

async fn delete_namespace_via_api(namespaces: &Api<Namespace>, namespace: &str) -> bool {
    info!(namespace, "invoking kubernetes API to delete namespace");
    match timeout(
        NAMESPACE_DELETE_TIMEOUT,
        namespaces.delete(namespace, &DeleteParams::default()),
    )
    .await
    {
        Ok(Ok(_)) => {
            info!(
                namespace,
                "delete request accepted; waiting for termination"
            );
            true
        }

        Ok(Err(err)) => {
            warn!(namespace, error = ?err, "failed to delete namespace via API");
            false
        }

        Err(_) => {
            warn!(
                namespace,
                "kubernetes API timed out deleting namespace; falling back to kubectl"
            );
            false
        }
    }
}

async fn delete_namespace_via_cli(namespace: &str) -> bool {
    info!(namespace, "invoking kubectl delete namespace fallback");
    let output = run_kubectl_delete_namespace(namespace).await;

    match output {
        Ok(result) if result.status.success() => {
            info!(namespace, "kubectl delete namespace completed successfully");
            true
        }

        Ok(result) => {
            log_kubectl_delete_failure(namespace, &result.stdout, &result.stderr);
            false
        }

        Err(err) => {
            warn!(namespace, error = ?err, "failed to spawn kubectl delete namespace");
            false
        }
    }
}

async fn run_kubectl_delete_namespace(namespace: &str) -> Result<Output, io::Error> {
    Command::new("kubectl")
        .arg("delete")
        .arg("namespace")
        .arg(namespace)
        .arg("--wait=true")
        .output()
        .await
}

fn log_kubectl_delete_failure(namespace: &str, stdout: &[u8], stderr: &[u8]) {
    warn!(
        namespace,
        stderr = %String::from_utf8_lossy(stderr),
        stdout = %String::from_utf8_lossy(stdout),
        "kubectl delete namespace failed"
    );
}

async fn wait_for_namespace_termination(namespaces: &Api<Namespace>, namespace: &str) {
    const NAMESPACE_TERMINATION_POLL_ATTEMPTS: u32 = 60;
    const NAMESPACE_TERMINATION_POLL_INTERVAL: Duration = Duration::from_secs(1);

    for attempt in 0..NAMESPACE_TERMINATION_POLL_ATTEMPTS {
        if namespace_deleted(namespaces, namespace, attempt).await {
            return;
        }
        sleep(NAMESPACE_TERMINATION_POLL_INTERVAL).await;
    }

    warn_namespace_still_present(namespace);
}

async fn namespace_deleted(namespaces: &Api<Namespace>, namespace: &str, attempt: u32) -> bool {
    match namespaces.get_opt(namespace).await {
        Ok(Some(ns)) => {
            if attempt == 0 {
                let phase = ns
                    .status
                    .as_ref()
                    .and_then(|status| status.phase.clone())
                    .unwrap_or_else(|| "Unknown".into());
                info!(namespace, ?phase, "waiting for namespace to terminate");
            }
            false
        }

        Ok(None) => {
            info!(namespace, "namespace deleted");
            true
        }

        Err(err) => {
            warn!(namespace, error = ?err, "namespace poll failed");
            true
        }
    }
}

fn warn_namespace_still_present(namespace: &str) {
    warn!(
        namespace,
        "namespace still present after waiting for deletion"
    );
}

fn cleanup_completed(result: Result<(), Elapsed>) -> bool {
    match result {
        Ok(()) => true,
        Err(error) => {
            warn!(
                error = ?error,
                timeout_secs = CLEANUP_TIMEOUT.as_secs(),
                "cleanup timed out; falling back to background thread"
            );
            false
        }
    }
}

impl CleanupGuard for RunnerCleanup {
    fn cleanup(self: Box<Self>) {
        if Handle::try_current().is_err() && self.blocking_cleanup_success() {
            return;
        }
        self.spawn_cleanup_thread();
    }
}

use std::thread;

use k8s_openapi::api::core::v1::Namespace;
use kube::{Api, Client, api::DeleteParams};
use testing_framework_core::scenario::CleanupGuard;
use tokio::{
    process::Command,
    time::{Duration, sleep},
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
        match tokio::runtime::Runtime::new() {
            Ok(rt) => match rt.block_on(async {
                tokio::time::timeout(CLEANUP_TIMEOUT, self.cleanup_async()).await
            }) {
                Ok(()) => true,
                Err(err) => {
                    warn!(
                        error = ?err,
                        "cleanup timed out after {}s; falling back to background thread",
                        CLEANUP_TIMEOUT.as_secs()
                    );
                    false
                }
            },
            Err(err) => {
                warn!(error = ?err, "unable to create cleanup runtime; falling back to background thread");
                false
            }
        }
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

async fn uninstall_release_and_namespace(client: &Client, release: &str, namespace: &str) {
    if let Err(err) = uninstall_release(release, namespace).await {
        warn!(release, namespace, error = ?err, "helm uninstall failed during cleanup");
    }

    info!(namespace, "deleting namespace via k8s API");
    delete_namespace(client, namespace).await;
    info!(namespace, "namespace delete request finished");
}

fn run_background_cleanup(cleanup: Box<RunnerCleanup>) {
    match tokio::runtime::Runtime::new() {
        Ok(rt) => {
            if let Err(err) = rt.block_on(async {
                tokio::time::timeout(CLEANUP_TIMEOUT, cleanup.cleanup_async()).await
            }) {
                warn!("[k8s-runner] background cleanup timed out: {err}");
            }
        }
        Err(err) => warn!("[k8s-runner] unable to create cleanup runtime: {err}"),
    }
}

async fn delete_namespace(client: &Client, namespace: &str) {
    let namespaces: Api<Namespace> = Api::all(client.clone());

    if delete_namespace_via_api(&namespaces, namespace).await {
        wait_for_namespace_termination(&namespaces, namespace).await;
        return;
    }

    if delete_namespace_via_cli(namespace).await {
        wait_for_namespace_termination(&namespaces, namespace).await;
    } else {
        warn!(
            namespace,
            "unable to delete namespace using kubectl fallback"
        );
    }
}

async fn delete_namespace_via_api(namespaces: &Api<Namespace>, namespace: &str) -> bool {
    info!(namespace, "invoking kubernetes API to delete namespace");
    match tokio::time::timeout(
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
    let output = Command::new("kubectl")
        .arg("delete")
        .arg("namespace")
        .arg(namespace)
        .arg("--wait=true")
        .output()
        .await;

    match output {
        Ok(result) if result.status.success() => {
            info!(namespace, "kubectl delete namespace completed successfully");
            true
        }
        Ok(result) => {
            warn!(
                namespace,
                stderr = %String::from_utf8_lossy(&result.stderr),
                stdout = %String::from_utf8_lossy(&result.stdout),
                "kubectl delete namespace failed"
            );
            false
        }
        Err(err) => {
            warn!(namespace, error = ?err, "failed to spawn kubectl delete namespace");
            false
        }
    }
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

    warn!(
        "[k8s-runner] namespace `{}` still present after waiting for deletion",
        namespace
    );
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

impl CleanupGuard for RunnerCleanup {
    fn cleanup(self: Box<Self>) {
        if tokio::runtime::Handle::try_current().is_err() && self.blocking_cleanup_success() {
            return;
        }
        self.spawn_cleanup_thread();
    }
}

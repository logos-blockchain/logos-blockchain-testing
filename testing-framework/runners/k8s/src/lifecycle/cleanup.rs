use std::thread;

use k8s_openapi::api::core::v1::Namespace;
use kube::{Api, Client, api::DeleteParams};
use testing_framework_core::scenario::CleanupGuard;
use tokio::{
    process::Command,
    time::{Duration, sleep},
};
use tracing::warn;

use crate::infrastructure::helm::uninstall_release;

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
            print_preserve_notice(&self.release, &self.namespace);
            return;
        }

        uninstall_release_and_namespace(&self.client, &self.release, &self.namespace).await;
    }

    fn blocking_cleanup_success(&self) -> bool {
        match tokio::runtime::Runtime::new() {
            Ok(rt) => match rt.block_on(async {
                tokio::time::timeout(Duration::from_secs(120), self.cleanup_async()).await
            }) {
                Ok(()) => true,
                Err(err) => {
                    warn!(
                        "[k8s-runner] cleanup timed out after 120s: {err}; falling back to background thread"
                    );
                    false
                }
            },
            Err(err) => {
                warn!(
                    "[k8s-runner] unable to create cleanup runtime: {err}; falling back to background thread"
                );
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
                    warn!("[k8s-runner] cleanup thread panicked: {err:?}");
                }
            }
            Err(err) => warn!("[k8s-runner] failed to spawn cleanup thread: {err}"),
        }
    }
}

fn print_preserve_notice(release: &str, namespace: &str) {
    println!("[k8s-runner] preserving Helm release `{release}` in namespace `{namespace}`");
}

async fn uninstall_release_and_namespace(client: &Client, release: &str, namespace: &str) {
    if let Err(err) = uninstall_release(release, namespace).await {
        println!("[k8s-runner] helm uninstall {release} failed: {err}");
    }

    println!("[k8s-runner] deleting namespace `{namespace}` via k8s API",);
    delete_namespace(client, namespace).await;
    println!("[k8s-runner] delete request for namespace `{namespace}` finished",);
}

fn run_background_cleanup(cleanup: Box<RunnerCleanup>) {
    match tokio::runtime::Runtime::new() {
        Ok(rt) => {
            if let Err(err) = rt.block_on(async {
                tokio::time::timeout(Duration::from_secs(120), cleanup.cleanup_async()).await
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
        warn!("[k8s-runner] unable to delete namespace `{namespace}` using kubectl fallback");
    }
}

async fn delete_namespace_via_api(namespaces: &Api<Namespace>, namespace: &str) -> bool {
    println!("[k8s-runner] invoking kubernetes API to delete namespace `{namespace}`");
    match tokio::time::timeout(
        Duration::from_secs(10),
        namespaces.delete(namespace, &DeleteParams::default()),
    )
    .await
    {
        Ok(Ok(_)) => {
            println!(
                "[k8s-runner] delete request accepted for namespace `{namespace}`; waiting for termination"
            );
            true
        }
        Ok(Err(err)) => {
            println!("[k8s-runner] failed to delete namespace `{namespace}` via API: {err}");
            warn!("[k8s-runner] api delete failed for namespace {namespace}: {err}");
            false
        }
        Err(_) => {
            println!(
                "[k8s-runner] kubernetes API timed out deleting namespace `{namespace}`; falling back to kubectl"
            );
            false
        }
    }
}

async fn delete_namespace_via_cli(namespace: &str) -> bool {
    println!("[k8s-runner] invoking `kubectl delete namespace {namespace}` fallback");
    let output = Command::new("kubectl")
        .arg("delete")
        .arg("namespace")
        .arg(namespace)
        .arg("--wait=true")
        .output()
        .await;

    match output {
        Ok(result) if result.status.success() => {
            println!("[k8s-runner] `kubectl delete namespace {namespace}` completed successfully");
            true
        }
        Ok(result) => {
            println!(
                "[k8s-runner] `kubectl delete namespace {namespace}` failed: {}\n{}",
                String::from_utf8_lossy(&result.stderr),
                String::from_utf8_lossy(&result.stdout)
            );
            false
        }
        Err(err) => {
            println!("[k8s-runner] failed to spawn kubectl for namespace `{namespace}`: {err}");
            false
        }
    }
}

async fn wait_for_namespace_termination(namespaces: &Api<Namespace>, namespace: &str) {
    for attempt in 0..60 {
        if namespace_deleted(namespaces, namespace, attempt).await {
            return;
        }
        sleep(Duration::from_secs(1)).await;
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
                println!(
                    "[k8s-runner] waiting for namespace `{}` to terminate (phase={phase:?})",
                    namespace
                );
            }
            false
        }
        Ok(None) => {
            println!("[k8s-runner] namespace `{namespace}` deleted");
            true
        }
        Err(err) => {
            warn!("[k8s-runner] namespace `{namespace}` poll failed: {err}");
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

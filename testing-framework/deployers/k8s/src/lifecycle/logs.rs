use k8s_openapi::api::core::v1::Pod;
use kube::{
    Api, Client,
    api::{ListParams, LogParams},
};
use tracing::{info, warn};

const LOG_TAIL_LINES: i64 = 500;

/// Dumps recent logs for every pod in the namespace so failures keep their
/// diagnostics before cleanup tears the release and namespace down. Failures
/// while collecting logs only warn and never mask the original error.
pub async fn dump_namespace_logs(client: &Client, namespace: &str) {
    let pod_names = match list_pod_names(client, namespace).await {
        Ok(names) => names,
        Err(err) => {
            warn!(%namespace, error = ?err, "failed to list pods for log dump");
            return;
        }
    };

    for pod_name in pod_names {
        stream_pod_logs(client, namespace, &pod_name).await;
    }
}

async fn list_pod_names(client: &Client, namespace: &str) -> Result<Vec<String>, kube::Error> {
    let list = Api::<Pod>::namespaced(client.clone(), namespace)
        .list(&ListParams::default())
        .await?;
    Ok(list
        .into_iter()
        .filter_map(|pod| pod.metadata.name)
        .collect())
}

async fn stream_pod_logs(client: &Client, namespace: &str, pod_name: &str) {
    let pods: Api<Pod> = Api::namespaced(client.clone(), namespace);
    let params = LogParams {
        follow: false,
        tail_lines: Some(LOG_TAIL_LINES),
        ..Default::default()
    };

    match pods.logs(pod_name, &params).await {
        Ok(log) => info!(pod = pod_name, "pod logs:\n{log}"),
        Err(err) => warn!(pod = pod_name, error = ?err, "failed to fetch pod logs"),
    }
}

use k8s_openapi::api::apps::v1::Deployment;
use kube::{Api, Client};
use tokio::time::sleep;

use super::{ClusterWaitError, deployment_timeout};

const DEPLOYMENT_POLL_INTERVAL_SECS: u64 = 2;

pub async fn wait_for_deployment_ready(
    client: &Client,
    namespace: &str,
    name: &str,
) -> Result<(), ClusterWaitError> {
    let mut elapsed = std::time::Duration::ZERO;
    let interval = std::time::Duration::from_secs(DEPLOYMENT_POLL_INTERVAL_SECS);

    let timeout = deployment_timeout();

    while elapsed <= timeout {
        match Api::<Deployment>::namespaced(client.clone(), namespace)
            .get(name)
            .await
        {
            Ok(deployment) => {
                let desired = deployment
                    .spec
                    .as_ref()
                    .and_then(|spec| spec.replicas)
                    .unwrap_or(1);
                let ready = deployment
                    .status
                    .as_ref()
                    .and_then(|status| status.ready_replicas)
                    .unwrap_or(0);

                if ready >= desired {
                    return Ok(());
                }
            }
            Err(err) => {
                return Err(ClusterWaitError::DeploymentFetch {
                    name: name.to_owned(),
                    source: err,
                });
            }
        }

        sleep(interval).await;
        elapsed += interval;
    }

    Err(ClusterWaitError::DeploymentTimeout {
        name: name.to_owned(),
        namespace: namespace.to_owned(),
        timeout,
    })
}

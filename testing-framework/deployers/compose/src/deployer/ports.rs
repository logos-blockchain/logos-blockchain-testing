use testing_framework_core::topology::generation::GeneratedTopology;
use tracing::{debug, info};

use crate::{
    errors::ComposeRunnerError,
    infrastructure::{
        environment::StackEnvironment,
        ports::{HostPortMapping, discover_host_ports},
    },
};

pub struct PortManager;

impl PortManager {
    pub async fn prepare(
        environment: &mut StackEnvironment,
        descriptors: &GeneratedTopology,
    ) -> Result<HostPortMapping, ComposeRunnerError> {
        debug!(
            nodes = descriptors.nodes().len(),
            "resolving host ports for compose services"
        );
        match discover_host_ports(environment, descriptors).await {
            Ok(mapping) => {
                info!(
                    node_ports = ?mapping.node_api_ports(),
                    "resolved container host ports"
                );
                Ok(mapping)
            }
            Err(err) => {
                environment
                    .fail("failed to determine container host ports")
                    .await;

                tracing::warn!(%err, "failed to resolve host ports");
                Err(err)
            }
        }
    }
}

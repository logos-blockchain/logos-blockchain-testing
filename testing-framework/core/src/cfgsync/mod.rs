#[doc(hidden)]
pub use cfgsync_adapter::static_deployment::{
    DeploymentAdapter as CfgsyncEnv, build_materialized_artifacts as build_cfgsync_node_catalog,
};
pub use cfgsync_adapter::*;

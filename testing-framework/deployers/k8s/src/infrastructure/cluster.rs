use crate::wait::NodeConfigPorts;

#[derive(Default)]
/// Port specification for a k8s deployment before port-forwarding starts.
pub struct PortSpecs {
    /// Per-node API and auxiliary ports that must be exposed.
    pub nodes: Vec<NodeConfigPorts>,
}

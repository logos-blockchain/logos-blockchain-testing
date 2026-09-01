use std::{
    fmt, io,
    io::Read,
    net::{Ipv4Addr, TcpListener, TcpStream},
    process::{Child, Command as StdCommand, ExitStatus, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

use anyhow::anyhow;

use super::{ClusterWaitError, NodeConfigPorts, NodePortAllocation};

const PORT_FORWARD_READY_ATTEMPTS: u32 = 240;
const PORT_FORWARD_READY_POLL_INTERVAL: Duration = Duration::from_millis(250);

pub struct PortForwardHandle {
    child: Child,
}

impl fmt::Debug for PortForwardHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PortForwardHandle").finish_non_exhaustive()
    }
}

impl PortForwardHandle {
    pub fn shutdown(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for PortForwardHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

pub struct PortForwardSpawn {
    pub local_port: u16,
    pub handle: PortForwardHandle,
}

/// Everything needed to (re)spawn one service port-forward.
#[derive(Clone, Debug)]
pub struct ForwardSpec {
    /// Namespace holding the forwarded service.
    pub namespace: String,
    /// Service name to forward to.
    pub service: String,
    /// Local port the forward listens on.
    pub local_port: u16,
    /// Service port being forwarded.
    pub remote_port: u16,
}

/// One node's live forwards together with the specs to recreate them.
struct NodeForwards {
    api: ForwardEntry,
    auxiliary: ForwardEntry,
}

struct ForwardEntry {
    spec: ForwardSpec,
    handle: PortForwardHandle,
}

/// Shared registry of per-node port-forwards.
///
/// A restarted node's pod kills the `kubectl port-forward` processes bound to
/// it; this registry lets node lifecycle operations respawn a node's forwards
/// on the same local ports so existing clients and probes keep working. Empty
/// when the cluster is reached through NodePorts directly.
#[derive(Clone, Default)]
pub struct PortForwardRegistry {
    nodes: Arc<Mutex<Vec<Option<Arc<Mutex<NodeForwards>>>>>>,
}

impl fmt::Debug for PortForwardRegistry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PortForwardRegistry")
            .field("nodes", &self.registered_node_count())
            .finish()
    }
}

impl PortForwardRegistry {
    /// Returns whether any forwards are registered (NodePort mode has none).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.registered_node_count() == 0
    }

    /// Registers one node's api/auxiliary forwards in node-index order.
    pub(crate) fn register_node(
        &self,
        api_spec: ForwardSpec,
        api_handle: PortForwardHandle,
        auxiliary_spec: ForwardSpec,
        auxiliary_handle: PortForwardHandle,
    ) {
        self.lock().push(Some(Arc::new(Mutex::new(NodeForwards {
            api: ForwardEntry {
                spec: api_spec,
                handle: api_handle,
            },
            auxiliary: ForwardEntry {
                spec: auxiliary_spec,
                handle: auxiliary_handle,
            },
        }))));
    }

    /// Starts or recreates one node's forwards and returns its local ports.
    ///
    /// Manual clusters can start nodes in any order, so their registry is
    /// sparse until each node has been started at least once.
    pub(crate) fn forward_node(
        &self,
        index: usize,
        namespace: &str,
        service: &str,
        ports: NodeConfigPorts,
    ) -> Result<NodePortAllocation, ClusterWaitError> {
        if let Some(allocation) = self.local_ports(index) {
            self.respawn_node(index)?;
            return Ok(allocation);
        }

        let api_forward = port_forward_service(namespace, service, ports.api)?;
        let auxiliary_forward = port_forward_service(namespace, service, ports.auxiliary)?;
        let allocation = NodePortAllocation {
            api: api_forward.local_port,
            auxiliary: auxiliary_forward.local_port,
        };
        self.register_node_at(
            index,
            ForwardSpec {
                namespace: namespace.to_owned(),
                service: service.to_owned(),
                local_port: api_forward.local_port,
                remote_port: ports.api,
            },
            api_forward.handle,
            ForwardSpec {
                namespace: namespace.to_owned(),
                service: service.to_owned(),
                local_port: auxiliary_forward.local_port,
                remote_port: ports.auxiliary,
            },
            auxiliary_forward.handle,
        );
        Ok(allocation)
    }

    /// Shuts down every registered forward and clears the registry.
    pub fn shutdown_all(&self) {
        let nodes = {
            let mut nodes = self.lock();
            std::mem::take(&mut *nodes)
        };
        for node in nodes {
            let Some(node) = node else {
                continue;
            };
            let mut node = lock_node(&node);
            node.api.handle.shutdown();
            node.auxiliary.handle.shutdown();
        }
    }

    /// Async-friendly wrapper that keeps process waits off runtime workers.
    pub(crate) async fn shutdown_all_async(&self) {
        let forwards = self.clone();
        let _ = tokio::task::spawn_blocking(move || forwards.shutdown_all()).await;
    }

    /// Respawns the given node's forwards on their original local ports.
    ///
    /// No-op when the registry is empty (NodePort mode). Blocking: spawns
    /// `kubectl` processes and polls the local ports for readiness.
    pub(crate) fn respawn_node(&self, index: usize) -> Result<(), ClusterWaitError> {
        let node = {
            let nodes = self.lock();
            nodes.get(index).and_then(Option::as_ref).cloned().ok_or(
                ClusterWaitError::MissingPortForwardNode {
                    index,
                    nodes: nodes.iter().flatten().count(),
                },
            )?
        };
        let mut node = lock_node(&node);

        respawn_entry(&mut node.api)?;
        respawn_entry(&mut node.auxiliary)
    }

    fn register_node_at(
        &self,
        index: usize,
        api_spec: ForwardSpec,
        api_handle: PortForwardHandle,
        auxiliary_spec: ForwardSpec,
        auxiliary_handle: PortForwardHandle,
    ) {
        let node = Arc::new(Mutex::new(NodeForwards {
            api: ForwardEntry {
                spec: api_spec,
                handle: api_handle,
            },
            auxiliary: ForwardEntry {
                spec: auxiliary_spec,
                handle: auxiliary_handle,
            },
        }));
        let previous = {
            let mut nodes = self.lock();
            if nodes.len() <= index {
                nodes.resize_with(index + 1, || None);
            }
            nodes[index].replace(node)
        };
        if let Some(previous) = previous {
            let mut previous = lock_node(&previous);
            previous.api.handle.shutdown();
            previous.auxiliary.handle.shutdown();
        }
    }

    fn local_ports(&self, index: usize) -> Option<NodePortAllocation> {
        let node = self.lock().get(index).and_then(Option::as_ref).cloned()?;
        let node = lock_node(&node);
        Some(NodePortAllocation {
            api: node.api.spec.local_port,
            auxiliary: node.auxiliary.spec.local_port,
        })
    }

    fn registered_node_count(&self) -> usize {
        self.lock().iter().flatten().count()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Option<Arc<Mutex<NodeForwards>>>>> {
        self.nodes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn lock_node(node: &Mutex<NodeForwards>) -> std::sync::MutexGuard<'_, NodeForwards> {
    node.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Kills an entry's forward and spawns a replacement on the same local port.
fn respawn_entry(entry: &mut ForwardEntry) -> Result<(), ClusterWaitError> {
    entry.handle.shutdown();

    let spec = &entry.spec;
    let mut child = spawn_kubectl_port_forward(
        &spec.namespace,
        &spec.service,
        spec.local_port,
        spec.remote_port,
    )
    .map_err(|source| port_forward_error(&spec.service, spec.remote_port, source.into()))?;
    wait_until_port_forward_ready(&mut child, spec.local_port, &spec.service, spec.remote_port)?;

    entry.handle = PortForwardHandle { child };
    Ok(())
}

pub fn port_forward_service(
    namespace: &str,
    service: &str,
    remote_port: u16,
) -> Result<PortForwardSpawn, ClusterWaitError> {
    let local_port =
        allocate_local_port().map_err(|source| port_forward_error(service, remote_port, source))?;
    let mut child = spawn_kubectl_port_forward(namespace, service, local_port, remote_port)
        .map_err(|source| port_forward_error(service, remote_port, source.into()))?;

    wait_until_port_forward_ready(&mut child, local_port, service, remote_port)?;

    Ok(PortForwardSpawn {
        local_port,
        handle: PortForwardHandle { child },
    })
}

fn allocate_local_port() -> anyhow::Result<u16> {
    let listener = TcpListener::bind(localhost_addr(0))?;
    let port = listener.local_addr()?.port();
    drop(listener);
    Ok(port)
}

fn spawn_kubectl_port_forward(
    namespace: &str,
    service: &str,
    local_port: u16,
    remote_port: u16,
) -> io::Result<Child> {
    StdCommand::new("kubectl")
        .arg("port-forward")
        .arg("-n")
        .arg(namespace)
        .arg(format!("svc/{service}"))
        .arg(format!("{local_port}:{remote_port}"))
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
}

fn wait_until_port_forward_ready(
    child: &mut Child,
    local_port: u16,
    service: &str,
    remote_port: u16,
) -> Result<(), ClusterWaitError> {
    for _ in 0..PORT_FORWARD_READY_ATTEMPTS {
        ensure_port_forward_running(child, service, remote_port)?;

        if local_port_reachable(local_port) {
            return Ok(());
        }

        thread::sleep(PORT_FORWARD_READY_POLL_INTERVAL);
    }

    let _ = child.kill();
    let _ = child.wait();
    let details = read_port_forward_stderr(child);
    Err(port_forward_ready_timeout_error(
        service,
        remote_port,
        details.as_deref(),
    ))
}

fn ensure_port_forward_running(
    child: &mut Child,
    service: &str,
    remote_port: u16,
) -> Result<(), ClusterWaitError> {
    let Some(status) = exited_status(child) else {
        return Ok(());
    };

    Err(port_forward_error(
        service,
        remote_port,
        port_forward_process_error(status, read_port_forward_stderr(child)),
    ))
}

fn port_forward_error(service: &str, remote_port: u16, source: anyhow::Error) -> ClusterWaitError {
    ClusterWaitError::PortForward {
        service: service.to_owned(),
        port: remote_port,
        source,
    }
}

fn port_forward_ready_timeout_error(
    service: &str,
    remote_port: u16,
    details: Option<&str>,
) -> ClusterWaitError {
    port_forward_error(
        service,
        remote_port,
        anyhow!(
            "port-forward did not become ready{}",
            format_port_forward_details(details)
        ),
    )
}

fn exited_status(child: &mut Child) -> Option<ExitStatus> {
    child.try_wait().ok().flatten()
}

fn read_port_forward_stderr(child: &mut Child) -> Option<String> {
    let mut stderr = child.stderr.take()?;
    let mut output = String::new();
    if stderr.read_to_string(&mut output).is_err() {
        return None;
    }
    let trimmed = output.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

fn port_forward_process_error(status: ExitStatus, details: Option<String>) -> anyhow::Error {
    anyhow!(
        "kubectl exited with {status}{}",
        format_port_forward_details(details.as_deref())
    )
}

fn format_port_forward_details(details: Option<&str>) -> String {
    match details {
        Some(details) => format!(": {details}"),
        None => String::new(),
    }
}

fn local_port_reachable(local_port: u16) -> bool {
    TcpStream::connect(localhost_addr(local_port)).is_ok()
}

const fn localhost_addr(port: u16) -> (Ipv4Addr, u16) {
    (Ipv4Addr::LOCALHOST, port)
}

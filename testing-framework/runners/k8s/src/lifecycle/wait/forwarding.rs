use std::{
    net::{Ipv4Addr, TcpListener, TcpStream},
    process::{Child, Command as StdCommand, Stdio},
    thread,
    time::Duration,
};

use anyhow::{Result as AnyhowResult, anyhow};

use super::{ClusterWaitError, NodeConfigPorts, NodePortAllocation};

const PORT_FORWARD_READY_ATTEMPTS: u32 = 20;
const PORT_FORWARD_READY_POLL_INTERVAL: Duration = Duration::from_millis(250);

pub struct PortForwardHandle {
    child: Child,
}

impl std::fmt::Debug for PortForwardHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
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

pub fn port_forward_group(
    namespace: &str,
    release: &str,
    kind: &str,
    ports: &[NodeConfigPorts],
    allocations: &mut Vec<NodePortAllocation>,
) -> Result<Vec<PortForwardHandle>, ClusterWaitError> {
    let mut forwards = Vec::new();
    for (index, ports) in ports.iter().enumerate() {
        let service = format!("{release}-{kind}-{index}");
        let PortForwardSpawn {
            local_port: api_port,
            handle: api_forward,
        } = match port_forward_service(namespace, &service, ports.api) {
            Ok(forward) => forward,
            Err(err) => {
                kill_port_forwards(&mut forwards);
                return Err(err);
            }
        };
        let PortForwardSpawn {
            local_port: testing_port,
            handle: testing_forward,
        } = match port_forward_service(namespace, &service, ports.testing) {
            Ok(forward) => forward,
            Err(err) => {
                kill_port_forwards(&mut forwards);
                return Err(err);
            }
        };
        allocations.push(NodePortAllocation {
            api: api_port,
            testing: testing_port,
        });
        forwards.push(api_forward);
        forwards.push(testing_forward);
    }
    Ok(forwards)
}

pub fn port_forward_service(
    namespace: &str,
    service: &str,
    remote_port: u16,
) -> Result<PortForwardSpawn, ClusterWaitError> {
    let local_port = allocate_local_port().map_err(|source| ClusterWaitError::PortForward {
        service: service.to_owned(),
        port: remote_port,
        source,
    })?;

    let mut child = StdCommand::new("kubectl")
        .arg("port-forward")
        .arg("-n")
        .arg(namespace)
        .arg(format!("svc/{service}"))
        .arg(format!("{local_port}:{remote_port}"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|source| ClusterWaitError::PortForward {
            service: service.to_owned(),
            port: remote_port,
            source: source.into(),
        })?;

    for _ in 0..PORT_FORWARD_READY_ATTEMPTS {
        if let Ok(Some(status)) = child.try_wait() {
            return Err(ClusterWaitError::PortForward {
                service: service.to_owned(),
                port: remote_port,
                source: anyhow!("kubectl exited with {status}"),
            });
        }
        if TcpStream::connect((Ipv4Addr::LOCALHOST, local_port)).is_ok() {
            return Ok(PortForwardSpawn {
                local_port,
                handle: PortForwardHandle { child },
            });
        }
        thread::sleep(PORT_FORWARD_READY_POLL_INTERVAL);
    }

    let _ = child.kill();
    Err(ClusterWaitError::PortForward {
        service: service.to_owned(),
        port: remote_port,
        source: anyhow!("port-forward did not become ready"),
    })
}

pub fn kill_port_forwards(handles: &mut Vec<PortForwardHandle>) {
    for handle in handles.iter_mut() {
        handle.shutdown();
    }
    handles.clear();
}

fn allocate_local_port() -> AnyhowResult<u16> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    let port = listener.local_addr()?.port();
    drop(listener);
    Ok(port)
}

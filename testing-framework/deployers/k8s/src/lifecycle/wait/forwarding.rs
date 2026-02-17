use std::{
    fmt, io,
    net::{Ipv4Addr, TcpListener, TcpStream},
    process::{Child, Command as StdCommand, ExitStatus, Stdio},
    thread,
    time::Duration,
};

use anyhow::anyhow;

use super::ClusterWaitError;

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

pub fn kill_port_forwards(handles: &mut Vec<PortForwardHandle>) {
    for handle in handles.iter_mut() {
        handle.shutdown();
    }
    handles.clear();
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
        .stderr(Stdio::null())
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
    Err(port_forward_ready_timeout_error(service, remote_port))
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
        anyhow!("kubectl exited with {status}"),
    ))
}

fn port_forward_error(service: &str, remote_port: u16, source: anyhow::Error) -> ClusterWaitError {
    ClusterWaitError::PortForward {
        service: service.to_owned(),
        port: remote_port,
        source,
    }
}

fn port_forward_ready_timeout_error(service: &str, remote_port: u16) -> ClusterWaitError {
    port_forward_error(
        service,
        remote_port,
        anyhow!("port-forward did not become ready"),
    )
}

fn exited_status(child: &mut Child) -> Option<ExitStatus> {
    child.try_wait().ok().flatten()
}

fn local_port_reachable(local_port: u16) -> bool {
    TcpStream::connect(localhost_addr(local_port)).is_ok()
}

const fn localhost_addr(port: u16) -> (Ipv4Addr, u16) {
    (Ipv4Addr::LOCALHOST, port)
}

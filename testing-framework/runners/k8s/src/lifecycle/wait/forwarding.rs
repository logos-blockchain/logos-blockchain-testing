use std::{
    net::{Ipv4Addr, TcpListener, TcpStream},
    process::{Child, Command as StdCommand, Stdio},
    thread,
    time::Duration,
};

use super::{ClusterWaitError, NodeConfigPorts, NodePortAllocation};

pub fn port_forward_group(
    namespace: &str,
    release: &str,
    kind: &str,
    ports: &[NodeConfigPorts],
    allocations: &mut Vec<NodePortAllocation>,
) -> Result<Vec<Child>, ClusterWaitError> {
    let mut forwards = Vec::new();
    for (index, ports) in ports.iter().enumerate() {
        let service = format!("{release}-{kind}-{index}");
        let (api_port, api_forward) = match port_forward_service(namespace, &service, ports.api) {
            Ok(forward) => forward,
            Err(err) => {
                kill_port_forwards(&mut forwards);
                return Err(err);
            }
        };
        let (testing_port, testing_forward) =
            match port_forward_service(namespace, &service, ports.testing) {
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
) -> Result<(u16, Child), ClusterWaitError> {
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

    for _ in 0..20 {
        if let Ok(Some(status)) = child.try_wait() {
            return Err(ClusterWaitError::PortForward {
                service: service.to_owned(),
                port: remote_port,
                source: anyhow::anyhow!("kubectl exited with {status}"),
            });
        }
        if TcpStream::connect((Ipv4Addr::LOCALHOST, local_port)).is_ok() {
            return Ok((local_port, child));
        }
        thread::sleep(Duration::from_millis(250));
    }

    let _ = child.kill();
    Err(ClusterWaitError::PortForward {
        service: service.to_owned(),
        port: remote_port,
        source: anyhow::anyhow!("port-forward did not become ready"),
    })
}

pub fn kill_port_forwards(handles: &mut Vec<Child>) {
    for handle in handles.iter_mut() {
        let _ = handle.kill();
        let _ = handle.wait();
    }
    handles.clear();
}

fn allocate_local_port() -> anyhow::Result<u16> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    let port = listener.local_addr()?.port();
    drop(listener);
    Ok(port)
}

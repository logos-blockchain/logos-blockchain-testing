use std::process::Stdio;

use serde_json::Value;
use testing_framework_core::scenario::DynError;
use tokio::process::Command;

pub const ATTACHABLE_NODE_LABEL_KEY: &str = "testing-framework.node";
pub const ATTACHABLE_NODE_LABEL_VALUE: &str = "true";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MappedTcpPort {
    pub container_port: u16,
    pub host_port: u16,
}

pub async fn discover_service_container_id(
    project: &str,
    service: &str,
) -> Result<String, DynError> {
    let stdout = run_docker_capture([
        "ps",
        "--filter",
        &format!("label=com.docker.compose.project={project}"),
        "--filter",
        &format!("label=com.docker.compose.service={service}"),
        "--format",
        "{{.ID}}",
    ])
    .await?;

    let ids: Vec<String> = stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect();

    match ids.as_slice() {
        [id] => Ok(id.clone()),
        [] => Err(format!(
            "no running container found for compose project '{project}' service '{service}'"
        )
        .into()),
        _ => Err(format!(
            "multiple running containers found for compose project '{project}' service '{service}'"
        )
        .into()),
    }
}

pub async fn discover_attachable_services(project: &str) -> Result<Vec<String>, DynError> {
    let attachable_filter =
        format!("label={ATTACHABLE_NODE_LABEL_KEY}={ATTACHABLE_NODE_LABEL_VALUE}");
    let attachable = discover_services_with_filters(project, Some(&attachable_filter)).await?;

    if attachable.is_empty() {
        return Err(format!(
            "no running compose services with label '{ATTACHABLE_NODE_LABEL_KEY}={ATTACHABLE_NODE_LABEL_VALUE}' found for project '{project}'"
        )
        .into());
    }

    Ok(attachable)
}

pub async fn inspect_mapped_tcp_ports(container_id: &str) -> Result<Vec<MappedTcpPort>, DynError> {
    let stdout = run_docker_capture([
        "inspect",
        "--format",
        "{{json .NetworkSettings.Ports}}",
        container_id,
    ])
    .await?;

    parse_mapped_tcp_ports(&stdout)
}

pub fn parse_mapped_tcp_ports(raw: &str) -> Result<Vec<MappedTcpPort>, DynError> {
    let ports_value: Value = serde_json::from_str(raw.trim())?;
    let ports_object = ports_value
        .as_object()
        .ok_or_else(|| "docker inspect ports payload is not an object".to_owned())?;

    let mut mapped = Vec::new();
    for (container_port_key, bindings) in ports_object {
        let Some(container_port) = parse_container_port(container_port_key) else {
            continue;
        };

        let Some(bindings_array) = bindings.as_array() else {
            continue;
        };

        let Some(host_port) = bindings_array.iter().find_map(parse_host_port_binding) else {
            continue;
        };

        mapped.push(MappedTcpPort {
            container_port,
            host_port,
        });
    }

    mapped.sort_by_key(|port| port.container_port);

    Ok(mapped)
}

pub async fn run_docker_capture<const N: usize>(args: [&str; N]) -> Result<String, DynError> {
    let output = Command::new("docker")
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .stdout(Stdio::piped())
        .output()
        .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

        return Err(format!(
            "docker {} failed with status {}: {stderr}",
            args.join(" "),
            output.status
        )
        .into());
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

async fn discover_services_with_filters(
    project: &str,
    extra_filter: Option<&str>,
) -> Result<Vec<String>, DynError> {
    let mut args = vec![
        "ps".to_owned(),
        "--filter".to_owned(),
        format!("label=com.docker.compose.project={project}"),
    ];

    if let Some(filter) = extra_filter {
        args.push("--filter".to_owned());
        args.push(filter.to_owned());
    }

    args.push("--format".to_owned());
    args.push("{{.Label \"com.docker.compose.service\"}}".to_owned());

    let output = Command::new("docker")
        .args(&args)
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .stdout(Stdio::piped())
        .output()
        .await?;

    if !output.status.success() {
        return Err(format!(
            "docker {} failed with status {}: {}",
            args.join(" "),
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        )
        .into());
    }

    let mut services: Vec<String> = output
        .stdout
        .split(|byte| *byte == b'\n')
        .filter_map(|line| {
            let parsed = String::from_utf8_lossy(line).trim().to_owned();
            (!parsed.is_empty()).then_some(parsed)
        })
        .collect();
    services.sort();
    services.dedup();
    Ok(services)
}

fn parse_container_port(port_key: &str) -> Option<u16> {
    let (port, proto) = port_key.split_once('/')?;
    if proto != "tcp" {
        return None;
    }

    port.parse::<u16>().ok()
}

fn parse_host_port_binding(binding: &Value) -> Option<u16> {
    binding
        .get("HostPort")
        .and_then(Value::as_str)?
        .parse::<u16>()
        .ok()
}

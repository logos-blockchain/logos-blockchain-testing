use std::{path::Path, process::Command as StdCommand};

use testing_framework_core::{
    scenario::cfgsync::{apply_topology_overrides, load_cfgsync_template, write_cfgsync_template},
    topology::GeneratedTopology,
};

#[derive(Debug)]
pub enum CfgsyncServerHandle {
    Container { name: String },
}

impl CfgsyncServerHandle {
    pub fn shutdown(&mut self) {
        match self {
            Self::Container { name } => {
                let container_name = name.clone();
                let status = StdCommand::new("docker")
                    .arg("rm")
                    .arg("-f")
                    .arg(&container_name)
                    .status();
                if let Ok(status) = status {
                    if !status.success() {
                        eprintln!(
                            "[compose-runner] failed to remove cfgsync container {container_name}: {status}"
                        );
                    }
                } else {
                    eprintln!(
                        "[compose-runner] failed to spawn docker rm for cfgsync container {container_name}"
                    );
                }
            }
        }
    }
}

pub fn update_cfgsync_config(
    path: &Path,
    topology: &GeneratedTopology,
    use_kzg_mount: bool,
    port: u16,
) -> anyhow::Result<()> {
    let mut cfg = load_cfgsync_template(path)?;
    cfg.port = port;
    apply_topology_overrides(&mut cfg, topology, use_kzg_mount);
    write_cfgsync_template(path, &cfg)?;
    Ok(())
}

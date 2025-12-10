use std::{path::Path, process::Command as StdCommand};

use testing_framework_core::{
    scenario::cfgsync::{apply_topology_overrides, load_cfgsync_template, write_cfgsync_template},
    topology::generation::GeneratedTopology,
};

/// Handle that tracks a cfgsync server started for compose runs.
#[derive(Debug)]
pub enum CfgsyncServerHandle {
    Container { name: String, stopped: bool },
}

impl CfgsyncServerHandle {
    /// Stop the backing container if still running.
    pub fn shutdown(&mut self) {
        match self {
            Self::Container { name, stopped } if !*stopped => {
                remove_container(name);
                *stopped = true;
            }
            _ => {}
        }
    }
}

fn remove_container(name: &str) {
    match StdCommand::new("docker")
        .arg("rm")
        .arg("-f")
        .arg(name)
        .status()
    {
        Ok(status) if status.success() => {}
        Ok(status) => {
            eprintln!("[compose-runner] failed to remove cfgsync container {name}: {status}");
        }
        Err(_) => {
            eprintln!("[compose-runner] failed to spawn docker rm for cfgsync container {name}");
        }
    }
}

impl Drop for CfgsyncServerHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Updates the cfgsync template on disk with topology-driven overrides.
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

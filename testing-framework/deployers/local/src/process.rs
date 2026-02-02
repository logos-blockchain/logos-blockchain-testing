use std::{
    collections::HashMap,
    fs, io, mem,
    net::{Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    process::Stdio,
    thread,
    time::Duration,
};

use tempfile::TempDir;
use testing_framework_core::{env::Application, process::RuntimeNode, scenario::DynError};
use tokio::{
    process::{Child, Command},
    time::timeout,
};

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub enum NodeEndpointPort {
    TestingApi,
    Network,
    Custom(String),
}

#[derive(Clone)]
pub struct NodeEndpoints {
    pub api: SocketAddr,
    pub extra_ports: HashMap<NodeEndpointPort, u16>,
}

impl Default for NodeEndpoints {
    fn default() -> Self {
        Self {
            api: default_api_socket(),
            extra_ports: HashMap::new(),
        }
    }
}

impl NodeEndpoints {
    pub fn insert_port(&mut self, key: NodeEndpointPort, port: u16) {
        self.extra_ports.insert(key, port);
    }

    pub fn port(&self, key: &NodeEndpointPort) -> Option<u16> {
        self.extra_ports.get(key).copied()
    }
}

/// File materialized in the node working directory before spawn.
#[derive(Clone)]
pub struct LaunchFile {
    /// Path relative to the node working directory.
    pub relative_path: PathBuf,
    /// Raw file contents to write.
    pub contents: Vec<u8>,
}

/// Environment variable passed to the spawned process.
#[derive(Clone)]
pub struct LaunchEnvVar {
    /// Environment variable name.
    pub key: String,
    /// Environment variable value.
    pub value: String,
}

impl LaunchEnvVar {
    #[must_use]
    pub fn new(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: value.into(),
        }
    }
}

/// Local process launch plan.
#[derive(Clone, Default)]
pub struct LaunchSpec {
    /// Executable path.
    pub binary: PathBuf,
    /// Files to write before spawn.
    pub files: Vec<LaunchFile>,
    /// Command-line arguments.
    pub args: Vec<String>,
    /// Process environment variables.
    pub env: Vec<LaunchEnvVar>,
}

#[derive(Debug, thiserror::Error)]
pub enum ProcessSpawnError {
    #[error("failed to create tempdir: {source}")]
    TempDir {
        #[source]
        source: io::Error,
    },
    #[error("failed to write config: {source}")]
    Config {
        #[source]
        source: DynError,
    },
    #[error("failed to spawn process: {source}")]
    Spawn {
        #[source]
        source: io::Error,
    },
    #[error("failed to materialize launch files: {source}")]
    Materialize {
        #[source]
        source: io::Error,
    },
    #[error("process wait failed: {source}")]
    Wait {
        #[source]
        source: io::Error,
    },
    #[error("process readiness failed: {source}")]
    Readiness {
        #[source]
        source: tokio::time::error::Elapsed,
    },
}

pub struct ProcessNode<Config: Clone + Send + Sync + 'static, Client: Clone + Send + Sync + 'static>
{
    child: Child,
    tempdir: TempDir,
    keep_tempdir: bool,
    launch: LaunchSpec,
    config: Config,
    endpoints: NodeEndpoints,
    client: Client,
}

impl<Config: Clone + Send + Sync + 'static, Client: Clone + Send + Sync + 'static>
    ProcessNode<Config, Client>
{
    pub const fn config(&self) -> &Config {
        &self.config
    }

    pub fn client(&self) -> Client {
        self.client.clone()
    }

    pub fn client_ref(&self) -> &Client {
        &self.client
    }

    pub fn endpoints(&self) -> &NodeEndpoints {
        &self.endpoints
    }

    pub fn pid(&self) -> u32 {
        self.child.id().unwrap_or_default()
    }

    pub fn is_running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    pub async fn wait_for_exit(&mut self, wait_timeout: Duration) -> bool {
        timeout(wait_timeout, async {
            loop {
                if !self.is_running() {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .is_ok()
    }

    pub fn start_kill(&mut self) {
        let _ = self.child.start_kill();
    }

    pub fn keep_tempdir(&mut self) -> io::Result<()> {
        let dir = mem::replace(&mut self.tempdir, tempfile::tempdir()?);
        let _ = dir.keep();
        Ok(())
    }

    pub async fn spawn(
        label: &str,
        config: Config,
        build_launch_spec: impl FnOnce(&Config, &Path, &str) -> Result<LaunchSpec, DynError>,
        endpoints_from_config: impl FnOnce(&Config) -> NodeEndpoints,
        keep_tempdir: bool,
        persist_dir: Option<&Path>,
        client_from_endpoints: impl FnOnce(&NodeEndpoints) -> Client,
    ) -> Result<Self, ProcessSpawnError> {
        let tempdir = match persist_dir {
            Some(path) => {
                std::fs::create_dir_all(path).map_err(|source| ProcessSpawnError::TempDir {
                    source: io::Error::new(
                        source.kind(),
                        format!("failed to create persist dir {}: {source}", path.display()),
                    ),
                })?;
                TempDir::new_in(path).map_err(|source| ProcessSpawnError::TempDir { source })?
            }
            None => TempDir::new().map_err(|source| ProcessSpawnError::TempDir { source })?,
        };
        let launch = build_launch_spec(&config, tempdir.path(), label)
            .map_err(|source| ProcessSpawnError::Config { source })?;
        let endpoints = endpoints_from_config(&config);
        let client = client_from_endpoints(&endpoints);

        let child = spawn_child_for_launch(tempdir.path(), &launch).await?;

        Ok(Self {
            child,
            tempdir,
            keep_tempdir,
            launch,
            config,
            endpoints,
            client,
        })
    }

    async fn spawn_child(&self) -> Result<Child, ProcessSpawnError> {
        spawn_child_for_launch(self.tempdir.path(), &self.launch).await
    }

    async fn stop_child(&mut self) -> Result<(), ProcessSpawnError> {
        let _ = self.child.kill().await;
        let _ = self
            .child
            .wait()
            .await
            .map_err(|source| ProcessSpawnError::Wait { source })?;
        Ok(())
    }

    pub async fn restart(&mut self) -> Result<(), ProcessSpawnError> {
        self.stop_child().await?;
        self.child = self.spawn_child().await?;
        Ok(())
    }

    pub async fn stop(&mut self) {
        let _ = self.stop_child().await;
    }
}

async fn spawn_child_for_launch(
    tempdir: &Path,
    launch: &LaunchSpec,
) -> Result<Child, ProcessSpawnError> {
    materialize_launch_files(tempdir, launch)
        .map_err(|source| ProcessSpawnError::Materialize { source })?;

    build_process_command(tempdir, launch)
        .spawn()
        .map_err(|source| ProcessSpawnError::Spawn { source })
}

fn build_process_command(tempdir: &Path, launch: &LaunchSpec) -> Command {
    let mut command = Command::new(&launch.binary);
    command
        .args(&launch.args)
        .envs(launch_env_pairs(&launch.env))
        .current_dir(tempdir)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    command
}

fn launch_env_pairs(env: &[LaunchEnvVar]) -> impl Iterator<Item = (&str, &str)> {
    env.iter()
        .map(|entry| (entry.key.as_str(), entry.value.as_str()))
}

impl<Config: Clone + Send + Sync + 'static, Client: Clone + Send + Sync + 'static> Drop
    for ProcessNode<Config, Client>
{
    fn drop(&mut self) {
        if should_preserve_tempdir(self.keep_tempdir) {
            let _ = self.keep_tempdir();
        }
        self.start_kill();
    }
}

fn should_preserve_tempdir(keep_tempdir: bool) -> bool {
    thread::panicking() || keep_tempdir
}

#[async_trait::async_trait]
impl<E, Config> RuntimeNode<E> for ProcessNode<Config, E::NodeClient>
where
    E: Application,
    Config: Clone + Send + Sync + 'static,
{
    type SpawnError = ProcessSpawnError;

    fn client(&self) -> E::NodeClient {
        self.client()
    }

    fn is_running(&mut self) -> bool {
        self.is_running()
    }

    fn pid(&self) -> u32 {
        self.pid()
    }

    async fn stop(&mut self) {
        self.stop().await;
    }

    async fn restart(&mut self) -> Result<(), Self::SpawnError> {
        self.restart().await
    }
}

fn materialize_launch_files(base: &Path, launch: &LaunchSpec) -> io::Result<()> {
    for file in &launch.files {
        write_launch_file(base, file)?;
    }

    Ok(())
}

fn write_launch_file(base: &Path, file: &LaunchFile) -> io::Result<()> {
    let path = base.join(&file.relative_path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, &file.contents)
}

fn default_api_socket() -> SocketAddr {
    SocketAddr::from((Ipv4Addr::LOCALHOST, 0))
}

#[cfg(test)]
mod tests {
    use super::{NodeEndpointPort, NodeEndpoints};

    #[test]
    fn typed_ports_roundtrip() {
        let mut endpoints = NodeEndpoints::default();
        endpoints.insert_port(NodeEndpointPort::TestingApi, 18081);
        endpoints.insert_port(NodeEndpointPort::Network, 3000);
        endpoints.insert_port(NodeEndpointPort::Custom("metrics".to_string()), 9000);

        assert_eq!(endpoints.port(&NodeEndpointPort::TestingApi), Some(18081));
        assert_eq!(endpoints.port(&NodeEndpointPort::Network), Some(3000));
        assert_eq!(
            endpoints.port(&NodeEndpointPort::Custom("metrics".to_string())),
            Some(9000)
        );
    }
}

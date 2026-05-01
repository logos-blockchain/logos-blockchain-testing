use std::{
    collections::HashMap,
    env, fs,
    io::{self, Error, ErrorKind},
    mem,
    net::{Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
    process::Stdio,
    thread,
    time::{Duration, Instant},
};

use fs_extra::dir::{CopyOptions, copy as copy_dir};
use tempfile::TempDir;
use testing_framework_core::{env::Application, process::RuntimeNode, scenario::DynError};
use tokio::{
    process::{Child, Command},
    time::timeout,
};

const PROCESS_CLEANUP_WAIT_TIMEOUT: Duration = Duration::from_secs(5);
const PROCESS_CLEANUP_POLL_INTERVAL: Duration = Duration::from_millis(20);

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
    #[must_use]
    pub fn from_api_port(port: u16) -> Self {
        Self {
            api: SocketAddr::from((Ipv4Addr::LOCALHOST, port)),
            extra_ports: HashMap::new(),
        }
    }

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
    #[error("failed to copy snapshot directory: {source}")]
    Snapshot {
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

    pub fn working_dir(&self) -> &Path {
        self.tempdir.path()
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

    pub fn stop_blocking(&mut self) {
        let _ = self.child.start_kill();
        let _ = self.wait_for_exit_blocking(PROCESS_CLEANUP_WAIT_TIMEOUT);
    }

    fn wait_for_exit_blocking(&mut self, wait_timeout: Duration) -> bool {
        let deadline = Instant::now() + wait_timeout;

        loop {
            match self.child.try_wait() {
                Ok(Some(_)) | Err(_) => return true,
                Ok(None) => {}
            }

            if Instant::now() >= deadline {
                return false;
            }

            thread::sleep(PROCESS_CLEANUP_POLL_INTERVAL);
        }
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
        endpoints_from_config: impl FnOnce(&Config) -> Result<NodeEndpoints, DynError>,
        keep_tempdir: bool,
        persist_dir: Option<&Path>,
        snapshot_dir: Option<&Path>,
        client_from_endpoints: impl FnOnce(&NodeEndpoints) -> Result<Client, DynError>,
    ) -> Result<Self, ProcessSpawnError> {
        let tempdir = create_tempdir(persist_dir)?;
        if let Some(snapshot_dir) = snapshot_dir {
            copy_snapshot_dir(snapshot_dir, tempdir.path())
                .map_err(|source| ProcessSpawnError::Snapshot { source })?;
        }

        let launch = build_launch_spec(&config, tempdir.path(), label)
            .map_err(|source| ProcessSpawnError::Config { source })?;
        let endpoints = endpoints_from_config(&config)
            .map_err(|source| ProcessSpawnError::Config { source })?;
        let client = client_from_endpoints(&endpoints)
            .map_err(|source| ProcessSpawnError::Config { source })?;

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

    pub async fn restart_with_launch(
        &mut self,
        launch: LaunchSpec,
    ) -> Result<(), ProcessSpawnError> {
        self.stop_child().await?;
        self.launch = launch;
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
        .stderr(Stdio::inherit())
        .kill_on_drop(true);
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
        self.stop_blocking();
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

fn copy_snapshot_dir(from: &Path, to: &Path) -> io::Result<()> {
    let mut options = CopyOptions::new();
    options.copy_inside = true;
    options.overwrite = true;

    copy_dir(from, to, &options)
        .map(|_| ())
        .map_err(io::Error::other)
}

fn default_api_socket() -> SocketAddr {
    SocketAddr::from((Ipv4Addr::LOCALHOST, 0))
}

pub fn allocate_available_port() -> Result<u16, io::Error> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    drop(listener);
    Ok(port)
}

fn create_tempdir(persist_dir: Option<&Path>) -> Result<TempDir, ProcessSpawnError> {
    match persist_dir {
        Some(dir) => {
            let final_dir_name = dir
                .components()
                .last()
                .ok_or_else(|| ProcessSpawnError::TempDir {
                    source: Error::new(ErrorKind::Other, "invalid final directory"),
                })?
                .as_os_str()
                .display()
                .to_string()
                + "_";
            let parent_dir = dir.parent().ok_or_else(|| ProcessSpawnError::TempDir {
                source: Error::new(ErrorKind::Other, "invalid parent directory"),
            })?;

            fs::create_dir_all(parent_dir).map_err(|source| ProcessSpawnError::TempDir {
                source: Error::new(
                    source.kind(),
                    format!(
                        "failed to create parent dir for persist path {}: {source}",
                        dir.display()
                    ),
                ),
            })?;

            TempDir::with_prefix_in(final_dir_name, parent_dir)
                .map_err(|source| ProcessSpawnError::TempDir { source })
        }
        None => {
            let cwd = env::current_dir().map_err(|source| ProcessSpawnError::TempDir { source })?;
            TempDir::new_in(cwd).map_err(|source| ProcessSpawnError::TempDir { source })
        }
    }
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::{
        process::{Command as StdCommand, Stdio},
        time::{Duration, Instant},
    };

    #[cfg(unix)]
    use super::{LaunchSpec, ProcessNode};
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

    #[cfg(unix)]
    #[tokio::test]
    async fn drop_stops_child_process() {
        let node = ProcessNode::<(), ()>::spawn(
            "node",
            (),
            |_, _, _| {
                Ok(LaunchSpec {
                    binary: "/bin/sleep".into(),
                    args: vec!["60".into()],
                    ..LaunchSpec::default()
                })
            },
            |_| Ok(NodeEndpoints::default()),
            false,
            None,
            None,
            |_| Ok(()),
        )
        .await
        .expect("process should spawn");

        let pid = node.pid();
        drop(node);

        assert!(
            wait_until_process_exits(pid, Duration::from_secs(1)),
            "process {pid} should exit on drop"
        );
    }

    #[cfg(unix)]
    fn wait_until_process_exits(pid: u32, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;

        while Instant::now() < deadline {
            if !process_exists(pid) {
                return true;
            }

            std::thread::sleep(Duration::from_millis(20));
        }

        !process_exists(pid)
    }

    #[cfg(unix)]
    fn process_exists(pid: u32) -> bool {
        StdCommand::new("kill")
            .arg("-0")
            .arg(pid.to_string())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }
}

use std::{collections::HashMap, sync::Arc, time::Duration};

use nomos_tracing_service::TracingSettings;
use testing_framework_config::topology::configs::{
    GeneralConfig, consensus::ConsensusParams, da::DaParams, wallet::WalletConfig,
};
use tokio::{
    sync::{Mutex, oneshot::Sender},
    time::timeout,
};
use tracing::{error, info, warn};

use crate::{config::builder::try_create_node_configs, host::Host, server::CfgSyncConfig};

const HOST_POLLING_INTERVAL: Duration = Duration::from_secs(1);

pub enum RepoResponse {
    Config(Box<GeneralConfig>),
    Timeout,
    Error(String),
}

pub struct ConfigRepo {
    waiting_hosts: Mutex<HashMap<Host, Sender<RepoResponse>>>,
    n_hosts: usize,
    consensus_params: ConsensusParams,
    da_params: DaParams,
    tracing_settings: TracingSettings,
    wallet_config: WalletConfig,
    timeout_duration: Duration,
    ids: Option<Vec<[u8; 32]>>,
    da_ports: Option<Vec<u16>>,
    blend_ports: Option<Vec<u16>>,
}

impl From<CfgSyncConfig> for Arc<ConfigRepo> {
    fn from(config: CfgSyncConfig) -> Self {
        let consensus_params = config.to_consensus_params();
        let da_params = config.to_da_params();
        let tracing_settings = config.to_tracing_settings();
        let wallet_config = config.wallet_config();
        let ids = config.ids;
        let da_ports = config.da_ports;
        let blend_ports = config.blend_ports;

        ConfigRepo::new(
            config.n_hosts,
            consensus_params,
            da_params,
            tracing_settings,
            wallet_config,
            ids,
            da_ports,
            blend_ports,
            Duration::from_secs(config.timeout),
        )
    }
}

impl ConfigRepo {
    #[must_use]
    pub fn new(
        n_hosts: usize,
        consensus_params: ConsensusParams,
        da_params: DaParams,
        tracing_settings: TracingSettings,
        wallet_config: WalletConfig,
        ids: Option<Vec<[u8; 32]>>,
        da_ports: Option<Vec<u16>>,
        blend_ports: Option<Vec<u16>>,
        timeout_duration: Duration,
    ) -> Arc<Self> {
        let repo = Arc::new(Self {
            waiting_hosts: Mutex::new(HashMap::new()),
            n_hosts,
            consensus_params,
            da_params,
            tracing_settings,
            wallet_config,
            ids,
            da_ports,
            blend_ports,
            timeout_duration,
        });

        let repo_clone = Arc::clone(&repo);
        tokio::spawn(async move {
            repo_clone.run().await;
        });

        repo
    }

    pub async fn register(&self, host: Host, reply_tx: Sender<RepoResponse>) {
        let mut waiting_hosts = self.waiting_hosts.lock().await;
        waiting_hosts.insert(host, reply_tx);
    }

    async fn run(&self) {
        let timeout_duration = self.timeout_duration;

        if wait_for_hosts_with_timeout(self, timeout_duration).await {
            info!("all hosts have announced their IPs");

            let mut waiting_hosts = take_waiting_hosts(self).await;
            let hosts = waiting_hosts.keys().cloned().collect();

            let configs = match generate_node_configs(self, hosts) {
                Ok(configs) => configs,
                Err(message) => {
                    send_error_to_all(&mut waiting_hosts, &message);
                    return;
                }
            };

            send_configs_to_all_hosts(&mut waiting_hosts, &configs);
            return;
        }

        warn!("timeout: not all hosts announced within the time limit");
        let mut waiting_hosts = take_waiting_hosts(self).await;
        send_timeout_to_all(&mut waiting_hosts);
    }

    async fn wait_for_hosts(&self) {
        loop {
            let len = { self.waiting_hosts.lock().await.len() };
            if len >= self.n_hosts {
                break;
            }
            tokio::time::sleep(HOST_POLLING_INTERVAL).await;
        }
    }
}

async fn wait_for_hosts_with_timeout(repo: &ConfigRepo, timeout_duration: Duration) -> bool {
    timeout(timeout_duration, repo.wait_for_hosts())
        .await
        .is_ok()
}

async fn take_waiting_hosts(repo: &ConfigRepo) -> HashMap<Host, Sender<RepoResponse>> {
    let mut guard = repo.waiting_hosts.lock().await;
    std::mem::take(&mut *guard)
}

fn generate_node_configs(
    repo: &ConfigRepo,
    hosts: Vec<Host>,
) -> Result<HashMap<Host, GeneralConfig>, String> {
    try_create_node_configs(
        &repo.consensus_params,
        &repo.da_params,
        &repo.tracing_settings,
        &repo.wallet_config,
        repo.ids.clone(),
        repo.da_ports.clone(),
        repo.blend_ports.clone(),
        hosts,
    )
    .map_err(|err| {
        error!(error = %err, "failed to generate node configs");
        err.to_string()
    })
}

fn send_error_to_all(waiting_hosts: &mut HashMap<Host, Sender<RepoResponse>>, message: &str) {
    for (_, sender) in waiting_hosts.drain() {
        let _ = sender.send(RepoResponse::Error(message.to_string()));
    }
}

fn send_timeout_to_all(waiting_hosts: &mut HashMap<Host, Sender<RepoResponse>>) {
    for (_, sender) in waiting_hosts.drain() {
        let _ = sender.send(RepoResponse::Timeout);
    }
}

fn send_configs_to_all_hosts(
    waiting_hosts: &mut HashMap<Host, Sender<RepoResponse>>,
    configs: &HashMap<Host, GeneralConfig>,
) {
    for (host, sender) in waiting_hosts.drain() {
        match configs.get(&host) {
            Some(config) => {
                let _ = sender.send(RepoResponse::Config(Box::new(config.to_owned())));
            }
            None => {
                warn!(identifier = %host.identifier, "missing config for host");
                let _ = sender.send(RepoResponse::Error("missing config for host".to_string()));
            }
        }
    }
}

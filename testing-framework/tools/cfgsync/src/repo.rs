use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use nomos_tracing_service::TracingSettings;
use testing_framework_config::topology::configs::{
    GeneralConfig, consensus::ConsensusParams, da::DaParams, wallet::WalletConfig,
};
use tokio::{sync::oneshot::Sender, time::timeout};

use crate::{config::builder::create_node_configs, host::Host, server::CfgSyncConfig};

const HOST_POLLING_INTERVAL: Duration = Duration::from_secs(1);

pub enum RepoResponse {
    Config(Box<GeneralConfig>),
    Timeout,
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

    pub fn register(&self, host: Host, reply_tx: Sender<RepoResponse>) {
        let mut waiting_hosts = self.waiting_hosts.lock().unwrap();
        waiting_hosts.insert(host, reply_tx);
    }

    async fn run(&self) {
        let timeout_duration = self.timeout_duration;

        if timeout(timeout_duration, self.wait_for_hosts())
            .await
            .is_ok()
        {
            println!("All hosts have announced their IPs");

            let mut waiting_hosts = self.waiting_hosts.lock().unwrap();
            let hosts = waiting_hosts.keys().cloned().collect();

            let configs = create_node_configs(
                &self.consensus_params,
                &self.da_params,
                &self.tracing_settings,
                &self.wallet_config,
                self.ids.clone(),
                self.da_ports.clone(),
                self.blend_ports.clone(),
                hosts,
            );

            for (host, sender) in waiting_hosts.drain() {
                let config = configs.get(&host).expect("host should have a config");
                let _ = sender.send(RepoResponse::Config(Box::new(config.to_owned())));
            }
        } else {
            println!("Timeout: Not all hosts announced within the time limit");

            let mut waiting_hosts = self.waiting_hosts.lock().unwrap();
            for (_, sender) in waiting_hosts.drain() {
                let _ = sender.send(RepoResponse::Timeout);
            }
        }
    }

    async fn wait_for_hosts(&self) {
        loop {
            if self.waiting_hosts.lock().unwrap().len() >= self.n_hosts {
                break;
            }
            tokio::time::sleep(HOST_POLLING_INTERVAL).await;
        }
    }
}

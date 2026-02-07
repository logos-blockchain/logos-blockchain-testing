use core::{num::NonZeroU64, time::Duration};

use lb_blend_service::{
    core::settings::{CoverTrafficSettings, MessageDelayerSettings, SchedulerSettings},
    settings::TimingSettings,
};
use lb_libp2p::protocol_name::StreamProtocol;
use lb_node::config::{
    blend::deployment::{
        CommonSettings as BlendCommonSettings, CoreSettings as BlendCoreSettings,
        Settings as BlendDeploymentSettings,
    },
    deployment::{CustomDeployment, Settings as DeploymentSettings},
    network::deployment::Settings as NetworkDeploymentSettings,
};
use lb_utils::math::NonNegativeF64;

const DEFAULT_ROUND_DURATION: Duration = Duration::from_secs(1);

#[must_use]
pub fn default_e2e_deployment_settings() -> DeploymentSettings {
    let normalization_constant = match NonNegativeF64::try_from(1.03f64) {
        Ok(value) => value,
        Err(_) => unsafe {
            // Safety: normalization constant is a finite non-negative constant.
            std::hint::unreachable_unchecked()
        },
    };
    let message_frequency_per_round = match NonNegativeF64::try_from(1f64) {
        Ok(value) => value,
        Err(_) => unsafe {
            // Safety: message frequency is a finite non-negative constant.
            std::hint::unreachable_unchecked()
        },
    };
    DeploymentSettings::Custom(CustomDeployment {
        blend: BlendDeploymentSettings {
            common: BlendCommonSettings {
                minimum_network_size: unsafe { NonZeroU64::new_unchecked(30) },
                num_blend_layers: unsafe { NonZeroU64::new_unchecked(3) },
                timing: TimingSettings {
                    round_duration: DEFAULT_ROUND_DURATION,
                    rounds_per_interval: unsafe { NonZeroU64::new_unchecked(30) },
                    // (21,600 blocks * 30s per block) / 1s per round = 648,000 rounds
                    rounds_per_session: unsafe { NonZeroU64::new_unchecked(648_000) },
                    rounds_per_observation_window: unsafe { NonZeroU64::new_unchecked(30) },
                    rounds_per_session_transition_period: unsafe { NonZeroU64::new_unchecked(30) },
                    epoch_transition_period_in_slots: unsafe { NonZeroU64::new_unchecked(2_600) },
                },
                protocol_name: StreamProtocol::new("/blend/integration-tests"),
            },
            core: BlendCoreSettings {
                minimum_messages_coefficient: unsafe { NonZeroU64::new_unchecked(1) },
                normalization_constant,
                scheduler: SchedulerSettings {
                    cover: CoverTrafficSettings {
                        intervals_for_safety_buffer: 100,
                        message_frequency_per_round,
                    },
                    delayer: MessageDelayerSettings {
                        maximum_release_delay_in_rounds: unsafe { NonZeroU64::new_unchecked(3) },
                    },
                },
            },
        },
        network: NetworkDeploymentSettings {
            identify_protocol_name: StreamProtocol::new("/integration/nomos/identify/1.0.0"),
            kademlia_protocol_name: StreamProtocol::new("/integration/nomos/kad/1.0.0"),
        },
    })
}

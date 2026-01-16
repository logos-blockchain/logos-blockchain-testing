use groth16::fr_to_bytes;
use key_management_system_service::{backend::preload::PreloadKMSBackendSettings, keys::Key};
use testing_framework_config::topology::configs::{blend::GeneralBlendConfig, da::GeneralDaConfig};

pub fn create_kms_configs(
    blend_configs: &[GeneralBlendConfig],
    da_configs: &[GeneralDaConfig],
) -> Vec<PreloadKMSBackendSettings> {
    da_configs
        .iter()
        .zip(blend_configs.iter())
        .map(|(da_conf, blend_conf)| PreloadKMSBackendSettings {
            keys: [
                (
                    hex::encode(blend_conf.signer.public_key().to_bytes()),
                    Key::Ed25519(blend_conf.signer.clone()),
                ),
                (
                    hex::encode(fr_to_bytes(
                        blend_conf.secret_zk_key.to_public_key().as_fr(),
                    )),
                    Key::Zk(blend_conf.secret_zk_key.clone()),
                ),
                (
                    hex::encode(da_conf.signer.public_key().to_bytes()),
                    Key::Ed25519(da_conf.signer.clone()),
                ),
                (
                    hex::encode(fr_to_bytes(da_conf.secret_zk_key.to_public_key().as_fr())),
                    Key::Zk(da_conf.secret_zk_key.clone()),
                ),
            ]
            .into(),
        })
        .collect()
}

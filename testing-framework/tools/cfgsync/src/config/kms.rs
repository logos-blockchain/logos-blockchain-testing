use groth16::fr_to_bytes;
use key_management_system_service::{
    backend::preload::PreloadKMSBackendSettings,
    keys::{Ed25519Key, Key, ZkKey},
};
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
                    hex::encode(blend_conf.signer.verifying_key().as_bytes()),
                    Key::Ed25519(Ed25519Key::new(blend_conf.signer.clone())),
                ),
                (
                    hex::encode(fr_to_bytes(
                        &blend_conf.secret_zk_key.to_public_key().into_inner(),
                    )),
                    Key::Zk(ZkKey::new(blend_conf.secret_zk_key.clone())),
                ),
                (
                    hex::encode(da_conf.signer.verifying_key().as_bytes()),
                    Key::Ed25519(Ed25519Key::new(da_conf.signer.clone())),
                ),
                (
                    hex::encode(fr_to_bytes(
                        &da_conf.secret_zk_key.to_public_key().into_inner(),
                    )),
                    Key::Zk(ZkKey::new(da_conf.secret_zk_key.clone())),
                ),
            ]
            .into(),
        })
        .collect()
}

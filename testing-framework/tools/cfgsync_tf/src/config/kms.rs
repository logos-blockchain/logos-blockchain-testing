use groth16::fr_to_bytes;
use key_management_system_service::{backend::preload::PreloadKMSBackendSettings, keys::Key};
use testing_framework_config::topology::configs::blend::GeneralBlendConfig;

pub fn create_kms_configs(blend_configs: &[GeneralBlendConfig]) -> Vec<PreloadKMSBackendSettings> {
    blend_configs
        .iter()
        .map(|blend_conf| PreloadKMSBackendSettings {
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
            ]
            .into(),
        })
        .collect()
}

use hex;
use key_management_system_service::keys::{Ed25519Key, Key};
use serde_yaml::{Mapping, Number as YamlNumber, Value};
use testing_framework_config::nodes::kms::key_id_for_preload_backend;

pub fn normalize_ed25519_sigs(_value: &mut Value) {}

/// Inject cryptarchia/IBD defaults into a YAML config in-place.
pub fn inject_ibd_into_cryptarchia(yaml_value: &mut Value) {
    let Some(cryptarchia) = cryptarchia_section(yaml_value) else {
        return;
    };
    ensure_network_adapter(cryptarchia);
    ensure_sync_defaults(cryptarchia);
    ensure_ibd_bootstrap(cryptarchia);
}

/// Inject blend non-ephemeral signing key id when missing.
pub fn inject_blend_non_ephemeral_signing_key_id(yaml_value: &mut Value) {
    let Some(blend) = blend_section(yaml_value) else {
        return;
    };

    let key_id_key = Value::String("non_ephemeral_signing_key_id".into());
    if blend.contains_key(&key_id_key) {
        return;
    }

    let Some(key_str) = blend
        .get(&Value::String("non_ephemeral_signing_key".into()))
        .and_then(Value::as_str)
    else {
        return;
    };

    let Ok(bytes) = hex::decode(key_str) else {
        return;
    };
    let Ok(raw) = <[u8; 32]>::try_from(bytes.as_slice()) else {
        return;
    };

    let key_id = key_id_for_preload_backend(&Key::Ed25519(Ed25519Key::from_bytes(&raw)));
    blend.insert(key_id_key, Value::String(key_id));
}

/// Inject deployment chain sync protocol name when missing.
pub fn inject_chain_sync_protocol_name(yaml_value: &mut Value) {
    let Some(network) = deployment_network_section(yaml_value) else {
        return;
    };

    let key = Value::String("chain_sync_protocol_name".into());
    if network.contains_key(&key) {
        return;
    }

    network.insert(
        key,
        Value::String("/integration/nomos/cryptarchia/sync/1.0.0".into()),
    );
}

fn cryptarchia_section(yaml_value: &mut Value) -> Option<&mut Mapping> {
    yaml_value
        .as_mapping_mut()
        .and_then(|root| root.get_mut(&Value::String("cryptarchia".into())))
        .and_then(Value::as_mapping_mut)
}

fn blend_section(yaml_value: &mut Value) -> Option<&mut Mapping> {
    yaml_value
        .as_mapping_mut()
        .and_then(|root| root.get_mut(&Value::String("blend".into())))
        .and_then(Value::as_mapping_mut)
}

fn deployment_network_section(yaml_value: &mut Value) -> Option<&mut Mapping> {
    yaml_value
        .as_mapping_mut()
        .and_then(|root| root.get_mut(&Value::String("deployment".into())))
        .and_then(Value::as_mapping_mut)
        .and_then(|deployment| deployment.get_mut(&Value::String("network".into())))
        .and_then(Value::as_mapping_mut)
}

fn ensure_network_adapter(cryptarchia: &mut Mapping) {
    if cryptarchia.contains_key(&Value::String("network_adapter_settings".into())) {
        return;
    }
    let mut network = Mapping::new();
    network.insert(
        Value::String("topic".into()),
        Value::String("/cryptarchia/proto".into()),
    );
    cryptarchia.insert(
        Value::String("network_adapter_settings".into()),
        Value::Mapping(network),
    );
}

fn ensure_sync_defaults(cryptarchia: &mut Mapping) {
    if cryptarchia.contains_key(&Value::String("sync".into())) {
        return;
    }
    let mut orphan = Mapping::new();
    orphan.insert(
        Value::String("max_orphan_cache_size".into()),
        Value::Number(YamlNumber::from(5)),
    );
    let mut sync = Mapping::new();
    sync.insert(Value::String("orphan".into()), Value::Mapping(orphan));
    cryptarchia.insert(Value::String("sync".into()), Value::Mapping(sync));
}

fn ensure_ibd_bootstrap(cryptarchia: &mut Mapping) {
    let Some(bootstrap) = cryptarchia
        .get_mut(&Value::String("bootstrap".into()))
        .and_then(Value::as_mapping_mut)
    else {
        return;
    };

    let ibd_key = Value::String("ibd".into());
    if bootstrap.contains_key(&ibd_key) {
        return;
    }

    let mut ibd = Mapping::new();
    ibd.insert(Value::String("peers".into()), Value::Sequence(vec![]));

    bootstrap.insert(ibd_key, Value::Mapping(ibd));
}

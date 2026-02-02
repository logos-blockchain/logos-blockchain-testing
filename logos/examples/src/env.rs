use std::{
    env,
    str::{self, FromStr},
};

use testing_framework_core::topology::DeploymentSeed;

const DEFAULT_TOPOLOGY_SEED: [u8; 32] = {
    let mut bytes = [0u8; 32];
    bytes[31] = 1;
    bytes
};

pub fn read_env_any<T>(keys: &[&str], default: T) -> T
where
    T: FromStr + Copy,
{
    keys.iter()
        .find_map(|key| env::var(key).ok().and_then(|raw| raw.parse::<T>().ok()))
        .unwrap_or(default)
}

pub fn read_topology_seed() -> Option<DeploymentSeed> {
    let raw = env::var("LOGOS_BLOCKCHAIN_TOPOLOGY_SEED").ok()?;
    let raw = raw.strip_prefix("0x").unwrap_or(&raw);
    if raw.len() != 64 {
        return None;
    }

    let mut bytes = [0u8; 32];
    for (idx, chunk) in raw.as_bytes().chunks(2).enumerate() {
        let chunk = str::from_utf8(chunk).ok()?;
        bytes[idx] = u8::from_str_radix(chunk, 16).ok()?;
    }

    Some(DeploymentSeed::new(bytes))
}

pub fn read_topology_seed_or_default() -> DeploymentSeed {
    read_topology_seed().unwrap_or_else(|| DeploymentSeed::new(DEFAULT_TOPOLOGY_SEED))
}

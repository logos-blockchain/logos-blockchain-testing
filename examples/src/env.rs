use std::{env, str::FromStr};

pub fn read_env_any<T>(keys: &[&str], default: T) -> T
where
    T: FromStr + Copy,
{
    keys.iter()
        .find_map(|key| env::var(key).ok().and_then(|raw| raw.parse::<T>().ok()))
        .unwrap_or(default)
}

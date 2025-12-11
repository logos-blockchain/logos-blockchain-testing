#![allow(dead_code)]

use std::{fs::File, io, path::Path};

use nomos_tracing::logging::local::FileConfig;
use serde::Serialize;
use serde_yaml::Value;
use tracing::debug;

/// Configure tracing logger to write into `NOMOS_LOG_DIR` if set, else into the
/// provided base dir.
pub fn configure_logging<F>(base_dir: &Path, prefix: &str, set_logger: F)
where
    F: FnOnce(FileConfig),
{
    debug!(prefix, base_dir = %base_dir.display(), "configuring node logging");
    if let Ok(env_dir) = std::env::var("NOMOS_LOG_DIR") {
        let log_dir = std::path::PathBuf::from(env_dir);
        let _ = std::fs::create_dir_all(&log_dir);
        set_logger(FileConfig {
            directory: log_dir,
            prefix: Some(prefix.into()),
        });
    } else {
        set_logger(FileConfig {
            directory: base_dir.to_owned(),
            prefix: Some(prefix.into()),
        });
    }
}

/// Write a YAML config file, allowing a caller-provided injection hook to
/// mutate the serialized value before it is written.
pub fn write_config_with_injection<T, F>(config: &T, path: &Path, inject: F) -> io::Result<()>
where
    T: Serialize,
    F: FnOnce(&mut Value),
{
    debug!(path = %path.display(), "writing node config with injection");
    let mut yaml_value =
        serde_yaml::to_value(config).map_err(|err| io::Error::new(io::ErrorKind::Other, err))?;
    inject(&mut yaml_value);
    let file = File::create(path)?;
    serde_yaml::to_writer(file, &yaml_value)
        .map_err(|err| io::Error::new(io::ErrorKind::Other, err))
}

use std::{fs, path::Path};

use anyhow::Result;

#[derive(Debug, Clone)]
pub struct RenderedCfgsync {
    pub config_yaml: String,
    pub bundle_yaml: String,
}

#[derive(Debug, Clone, Copy)]
pub struct CfgsyncOutputPaths<'a> {
    pub config_path: &'a Path,
    pub bundle_path: &'a Path,
}

pub fn ensure_bundle_path(bundle_path: &mut Option<String>, output_bundle_path: &Path) {
    if bundle_path.is_some() {
        return;
    }

    *bundle_path = Some(
        output_bundle_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("cfgsync.bundle.yaml")
            .to_string(),
    );
}

pub fn apply_timeout_floor(timeout: &mut u64, min_timeout_secs: Option<u64>) {
    if let Some(min_timeout_secs) = min_timeout_secs {
        *timeout = (*timeout).max(min_timeout_secs);
    }
}

pub fn write_rendered_cfgsync(
    rendered: &RenderedCfgsync,
    output: CfgsyncOutputPaths<'_>,
) -> Result<()> {
    fs::write(output.config_path, &rendered.config_yaml)?;
    fs::write(output.bundle_path, &rendered.bundle_yaml)?;
    Ok(())
}

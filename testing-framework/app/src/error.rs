use thiserror::Error;

#[derive(Debug, Error)]
/// Error returned when a typed application handle cannot be registered or
/// found.
pub enum AppDeployError {
    /// No handle exists for the requested type and optional name.
    #[error("app handle is not exposed: {type_name}{suffix}", suffix = format_name(.name))]
    HandleMissing {
        /// Rust type name of the requested handle.
        type_name: &'static str,
        /// Instance name, or `None` for the default unnamed handle.
        name: Option<String>,
    },
    /// A handle already exists for the same type and optional name.
    #[error("app handle is already exposed: {type_name}{suffix}", suffix = format_name(.name))]
    DuplicateHandle {
        /// Rust type name of the duplicate handle.
        type_name: &'static str,
        /// Instance name, or `None` for the default unnamed handle.
        name: Option<String>,
    },
}

fn format_name(name: &Option<String>) -> String {
    name.as_ref()
        .map(|name| format!(" named {name:?}"))
        .unwrap_or_default()
}

//! Config schema migration helpers.
//!
//! Applies sequential version upgrades to a [`PawanConfig`] loaded from disk,
//! creating a timestamped backup before modifying anything.

use super::{default_tool_idle_timeout, PawanConfig};
use chrono::Utc;
use std::path::PathBuf;
use tracing;

/// Latest config version understood by this build.
const LATEST_CONFIG_VERSION: u32 = 1;

/// Outcome of a migration attempt.
#[derive(Debug)]
pub struct MigrationResult {
    /// Whether any migration steps were applied.
    pub migrated: bool,
    /// Version the config was at on disk.
    pub from_version: u32,
    /// Version the config is at now.
    pub to_version: u32,
    /// Path to the pre-migration backup, if one was created.
    pub backup_path: Option<PathBuf>,
}

impl MigrationResult {
    pub fn new(from_version: u32, to_version: u32, backup_path: Option<PathBuf>) -> Self {
        Self {
            migrated: from_version != to_version,
            from_version,
            to_version,
            backup_path,
        }
    }

    pub fn no_migration(version: u32) -> Self {
        Self {
            migrated: false,
            from_version: version,
            to_version: version,
            backup_path: None,
        }
    }
}

/// Migrate `config` to [`LATEST_CONFIG_VERSION`] in place.
///
/// Creates a timestamped backup at `config_path` (when provided) before
/// applying any changes. Migration steps are applied sequentially; if any
/// step fails the function returns early with a partial result and logs the
/// error — the config is left in the partially-migrated state.
pub fn migrate_to_latest(
    config: &mut PawanConfig,
    config_path: Option<&PathBuf>,
) -> MigrationResult {
    let current_version = config.config_version;

    if current_version >= LATEST_CONFIG_VERSION {
        return MigrationResult::no_migration(current_version);
    }

    let backup_path = config_path.and_then(|path| create_backup(path).ok());

    let mut version = current_version;
    while version < LATEST_CONFIG_VERSION {
        version = match apply_migration(config, version + 1) {
            Ok(v) => v,
            Err(e) => {
                tracing::error!(
                    from_version = version,
                    to_version = LATEST_CONFIG_VERSION,
                    error = %e,
                    "Config migration failed"
                );
                return MigrationResult::new(current_version, version, backup_path);
            }
        };
    }

    config.config_version = LATEST_CONFIG_VERSION;
    MigrationResult::new(current_version, LATEST_CONFIG_VERSION, backup_path)
}

/// Save `config` to `path` as pretty-printed TOML.
pub fn save_config(config: &PawanConfig, path: &PathBuf) -> Result<(), String> {
    let toml_string = toml::to_string_pretty(config)
        .map_err(|e| format!("Failed to serialize config to TOML: {}", e))?;

    std::fs::write(path, toml_string)
        .map_err(|e| format!("Failed to write config to {}: {}", path.display(), e))?;

    tracing::info!(path = %path.display(), "Config saved");
    Ok(())
}

// ---------------------------------------------------------------------------
// Internal migration steps
// ---------------------------------------------------------------------------

fn apply_migration(config: &mut PawanConfig, target_version: u32) -> Result<u32, String> {
    match target_version {
        1 => migrate_to_v1(config),
        _ => Err(format!("Unknown target version: {}", target_version)),
    }
}

/// v0 → v1: adds `config_version`, `tool_call_idle_timeout_secs`, and
/// optional skill/local-inference fields (all handled by serde defaults).
pub(super) fn migrate_to_v1(config: &mut PawanConfig) -> Result<u32, String> {
    config.config_version = 1;
    if config.tool_call_idle_timeout_secs == 0 {
        config.tool_call_idle_timeout_secs = default_tool_idle_timeout();
    }
    tracing::info!("Config migrated to version 1");
    Ok(1)
}

fn create_backup(config_path: &PathBuf) -> Result<PathBuf, String> {
    let timestamp = Utc::now().format("%Y%m%d_%H%M%S");
    let backup_path = config_path.with_extension(format!("toml.backup.{}", timestamp));

    std::fs::copy(config_path, &backup_path).map_err(|e| {
        format!(
            "Failed to create backup at {}: {}",
            backup_path.display(),
            e
        )
    })?;

    tracing::info!(backup = %backup_path.display(), "Config backup created");
    Ok(backup_path)
}

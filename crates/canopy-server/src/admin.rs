//! Fail-closed administrative command model and process dispatch.

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use serde::Serialize;

use canopy_core::{CanopyError, CanopyResult, UserProfile};

use crate::media::service::ImportSummary;

const DEFAULT_MAX_AUDIO_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const DEFAULT_MAX_ARTWORK_BYTES: u64 = 20 * 1024 * 1024;

#[derive(Parser, Debug)]
#[command(name = "canopy-admin", about = "Owner-only Canopy administration")]
pub struct AdminCli {
    #[command(subcommand)]
    pub command: AdminCommand,
}

#[derive(Subcommand, Debug)]
pub enum AdminCommand {
    Owner {
        #[command(subcommand)]
        command: OwnerCommand,
    },
    Media {
        #[command(subcommand)]
        command: MediaCommand,
    },
}

#[derive(Subcommand, Debug)]
pub enum OwnerCommand {
    Set { external_user_id: String },
}

#[derive(Subcommand, Debug)]
pub enum MediaCommand {
    Import { path: PathBuf },
}

/// Returns whether Clap produced normal help or version display output.
pub fn clap_display_requested(error: &clap::Error) -> bool {
    matches!(
        error.kind(),
        clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
    )
}

#[derive(Clone)]
pub struct AdminConfig {
    pub database_url: String,
    pub media_root: PathBuf,
    pub max_audio_bytes: u64,
    pub max_artwork_bytes: u64,
}

impl AdminConfig {
    /// Loads required admin configuration from the process environment.
    pub fn from_env() -> CanopyResult<Self> {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    /// Loads configuration through an injected lookup for deterministic tests.
    pub fn from_lookup<F>(lookup: F) -> CanopyResult<Self>
    where
        F: Fn(&str) -> Option<String>,
    {
        let database_url = required_value(&lookup, "CANOPY_DATABASE_URL")?;
        let media_root = PathBuf::from(required_value(&lookup, "CANOPY_MEDIA_ROOT")?);
        let max_audio_bytes =
            optional_limit(&lookup, "CANOPY_MAX_AUDIO_BYTES", DEFAULT_MAX_AUDIO_BYTES)?;
        let max_artwork_bytes = optional_limit(
            &lookup,
            "CANOPY_MAX_ARTWORK_BYTES",
            DEFAULT_MAX_ARTWORK_BYTES,
        )?;
        Ok(Self {
            database_url,
            media_root,
            max_audio_bytes,
            max_artwork_bytes,
        })
    }
}

#[derive(Debug, Serialize)]
pub struct OwnerProfileOutput {
    pub id: String,
    pub external_user_id: String,
    pub display_name: Option<String>,
    pub history_enabled: bool,
}

impl From<UserProfile> for OwnerProfileOutput {
    fn from(profile: UserProfile) -> Self {
        Self {
            id: profile.id,
            external_user_id: profile.external_user_id,
            display_name: profile.display_name,
            history_enabled: profile.history_enabled,
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum AdminOutput {
    OwnerSet { profile: OwnerProfileOutput },
    MediaImport { summary: ImportSummary },
    Error { code: String, message: String },
}

impl AdminOutput {
    /// Returns whether the command completed useful owner or import work.
    pub fn is_success(&self) -> bool {
        match self {
            Self::OwnerSet { .. } => true,
            Self::MediaImport { summary } => summary.imported + summary.duplicate > 0,
            Self::Error { .. } => false,
        }
    }

    /// Converts a domain error into a stable, secret-safe CLI document.
    pub fn from_error(error: CanopyError) -> Self {
        let (code, message) = match error {
            CanopyError::NotFound { entity, id } => {
                ("not_found", format!("{entity} not found: {id}"))
            }
            CanopyError::InvalidArgument(message) => ("invalid_argument", message),
            CanopyError::FailedPrecondition(message) => ("failed_precondition", message),
            CanopyError::Aborted(message) => ("aborted", message),
            CanopyError::RateLimited(message) => ("rate_limited", message),
            CanopyError::Unauthenticated(_) => ("unauthenticated", "authentication failed".into()),
            CanopyError::Storage(_) => (
                "storage_unavailable",
                "required storage dependency is unavailable".into(),
            ),
            CanopyError::Internal(_) => ("internal", "internal operation failed".into()),
        };
        Self::Error {
            code: code.into(),
            message,
        }
    }
}

#[cfg(feature = "pg")]
pub async fn execute_admin(
    cli: AdminCli,
    config: AdminConfig,
    pool: sqlx::PgPool,
) -> CanopyResult<AdminOutput> {
    use std::sync::Arc;

    use crate::jade_store::{
        PgInstanceSettingsRepository, PgMediaImportRepository, PgProfileRepository,
    };
    use crate::media::inspector::Mp3Inspector;
    use crate::media::service::MediaImportService;
    use crate::media::storage::ManagedMediaStore;
    use crate::owner::OwnerService;

    match cli.command {
        AdminCommand::Owner {
            command: OwnerCommand::Set { external_user_id },
        } => {
            let owner = OwnerService::new(
                Arc::new(PgProfileRepository::new(pool.clone())),
                Arc::new(PgInstanceSettingsRepository::new(pool)),
            );
            let profile = owner.set_owner(&external_user_id).await?;
            Ok(AdminOutput::OwnerSet {
                profile: profile.into(),
            })
        }
        AdminCommand::Media {
            command: MediaCommand::Import { path },
        } => {
            let storage = Arc::new(ManagedMediaStore::new(config.media_root));
            storage.prepare().await?;
            let service = MediaImportService::new(
                Arc::new(PgInstanceSettingsRepository::new(pool.clone())),
                Arc::new(PgMediaImportRepository::new(pool)),
                Arc::new(Mp3Inspector::new(
                    config.max_audio_bytes,
                    config.max_artwork_bytes,
                )),
                storage,
            );
            let summary = service.import_path(&path).await?;
            Ok(AdminOutput::MediaImport { summary })
        }
    }
}

fn required_value<F>(lookup: &F, key: &str) -> CanopyResult<String>
where
    F: Fn(&str) -> Option<String>,
{
    lookup(key)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| CanopyError::InvalidArgument(format!("{key} is required")))
}

fn optional_limit<F>(lookup: &F, key: &str, default: u64) -> CanopyResult<u64>
where
    F: Fn(&str) -> Option<String>,
{
    let Some(value) = lookup(key) else {
        return Ok(default);
    };
    let parsed = value
        .parse::<u64>()
        .map_err(|_| CanopyError::InvalidArgument(format!("{key} must be a positive integer")))?;
    if parsed == 0 {
        return Err(CanopyError::InvalidArgument(format!(
            "{key} must be greater than zero"
        )));
    }
    Ok(parsed)
}
#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::Path;

    use clap::Parser;

    use super::{
        AdminCli, AdminCommand, AdminConfig, MediaCommand, OwnerCommand, clap_display_requested,
    };

    #[test]
    fn parser_accepts_owner_assignment() {
        let cli = AdminCli::try_parse_from(["canopy-admin", "owner", "set", "user-123"])
            .expect("owner command should parse");

        assert!(matches!(
            cli.command,
            AdminCommand::Owner {
                command: OwnerCommand::Set { external_user_id }
            } if external_user_id == "user-123"
        ));
    }

    #[test]
    fn parser_accepts_media_import_path() {
        let cli = AdminCli::try_parse_from(["canopy-admin", "media", "import", "/music/album"])
            .expect("media import command should parse");

        assert!(matches!(
            cli.command,
            AdminCommand::Media {
                command: MediaCommand::Import { path }
            } if path == Path::new("/music/album")
        ));
    }

    #[test]
    fn config_requires_database_url_and_media_root() {
        let missing_database = AdminConfig::from_lookup(|key| {
            (key == "CANOPY_MEDIA_ROOT").then(|| "/srv/canopy/media".into())
        });
        assert!(missing_database.is_err());

        let missing_root = AdminConfig::from_lookup(|key| {
            (key == "CANOPY_DATABASE_URL").then(|| "postgres://canopy".into())
        });
        assert!(missing_root.is_err());
    }

    #[test]
    fn config_uses_conservative_default_limits() {
        let values = HashMap::from([
            ("CANOPY_DATABASE_URL", "postgres://canopy"),
            ("CANOPY_MEDIA_ROOT", "/srv/canopy/media"),
        ]);

        let config = AdminConfig::from_lookup(|key| values.get(key).map(ToString::to_string))
            .expect("required config should load");

        assert_eq!(config.max_audio_bytes, 2 * 1024 * 1024 * 1024);
        assert_eq!(config.max_artwork_bytes, 20 * 1024 * 1024);
    }

    #[test]
    fn config_rejects_zero_and_malformed_limits() {
        for (key, value) in [
            ("CANOPY_MAX_AUDIO_BYTES", "0"),
            ("CANOPY_MAX_AUDIO_BYTES", "many"),
            ("CANOPY_MAX_ARTWORK_BYTES", "0"),
            ("CANOPY_MAX_ARTWORK_BYTES", "many"),
        ] {
            let config = AdminConfig::from_lookup(|lookup| match lookup {
                "CANOPY_DATABASE_URL" => Some("postgres://canopy".into()),
                "CANOPY_MEDIA_ROOT" => Some("/srv/canopy/media".into()),
                lookup if lookup == key => Some(value.into()),
                _ => None,
            });
            assert!(config.is_err(), "{key}={value} must fail");
        }
    }

    #[test]
    fn help_is_a_successful_clap_display_request() {
        let error = AdminCli::try_parse_from(["canopy-admin", "--help"])
            .expect_err("help should short-circuit command parsing");

        assert!(clap_display_requested(&error));
    }
}

//! Production entry point for owner-only Canopy administration.

use std::process::ExitCode;
use std::time::Duration;

use clap::Parser;

use canopy_core::CanopyError;
use canopy_server::admin::{
    AdminCli, AdminConfig, AdminOutput, clap_display_requested, execute_admin,
};
use canopy_server::observability;

#[tokio::main]
async fn main() -> ExitCode {
    if observability::init().is_err() {
        return emit(AdminOutput::from_error(CanopyError::Internal(
            "failed to initialize tracing".into(),
        )));
    }

    let cli = match AdminCli::try_parse() {
        Ok(cli) => cli,
        Err(error) if clap_display_requested(&error) => {
            print!("{error}");
            return ExitCode::SUCCESS;
        }
        Err(error) => {
            return emit(AdminOutput::Error {
                code: "invalid_command".into(),
                message: error.to_string(),
            });
        }
    };
    let config = match AdminConfig::from_env() {
        Ok(config) => config,
        Err(error) => return emit(AdminOutput::from_error(error)),
    };
    let pool = match sqlx::postgres::PgPoolOptions::new()
        .max_connections(3)
        .acquire_timeout(Duration::from_secs(10))
        .connect(&config.database_url)
        .await
    {
        Ok(pool) => pool,
        Err(_) => {
            return emit(AdminOutput::Error {
                code: "database_unavailable".into(),
                message: "required database dependency is unavailable".into(),
            });
        }
    };
    let output = match execute_admin(cli, config, pool).await {
        Ok(output) => output,
        Err(error) => AdminOutput::from_error(error),
    };
    emit(output)
}

fn emit(output: AdminOutput) -> ExitCode {
    let succeeded = output.is_success();
    match serde_json::to_string(&output) {
        Ok(document) => println!("{document}"),
        Err(_) => {
            println!(
                r#"{{"command":"error","code":"serialization_failed","message":"failed to serialize admin output"}}"#
            )
        }
    }
    if succeeded {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

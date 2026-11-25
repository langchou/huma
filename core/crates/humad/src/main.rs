mod audit;
mod cli;
mod manager;
mod server;
mod state;

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;

use clap::{Parser, Subcommand};
use tracing::error;
use tracing_subscriber::EnvFilter;

use crate::audit::AuditLog;
use crate::cli::{CtlCommand, run_ctl};
use crate::manager::SessionManager;
use crate::server::{resolve_socket_path, resolve_state_dir, run_unix_server};

#[derive(Debug, Parser)]
#[command(name = "humad", version, about = "Huma headless agent runtime daemon")]
struct Args {
    /// Override Unix Domain Socket path.
    #[arg(long, global = true)]
    socket: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run the background humad Unix Domain Socket server.
    Serve,
    /// Control a running humad instance (CLI smoke & automation client).
    Ctl {
        #[command(subcommand)]
        action: CtlCommand,
    },
}

fn init_tracing() {
    let filter = EnvFilter::try_from_env("HUMA_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_target(true)
        .compact()
        .try_init();
}

#[tokio::main]
async fn main() -> ExitCode {
    init_tracing();
    let args = Args::parse();
    let socket_path = args.socket.unwrap_or_else(resolve_socket_path);

    match args.command {
        Command::Serve => {
            let audit_path = resolve_state_dir().join("audit/audit.jsonl");
            let audit = Arc::new(AuditLog::new(audit_path));
            let manager = Arc::new(SessionManager::new(audit));
            if let Err(err) = run_unix_server(&socket_path, manager).await {
                error!(error = %err, "humad serve exited with error");
                return ExitCode::FAILURE;
            }
            ExitCode::SUCCESS
        }
        Command::Ctl { action } => match run_ctl(&socket_path, action).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                error!(error = %err, "ctl command failed");
                ExitCode::FAILURE
            }
        },
    }
}

use std::path::Path;

use clap::Subcommand;
use huma_protocol::rpc::ClientCommand;
use huma_protocol::{Attachment, ClientRequest, DriverKind, PromptInput, ServerEnvelope};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use uuid::Uuid;

#[derive(Debug, Subcommand)]
pub enum CtlCommand {
    /// Ping the running humad daemon.
    Ping,
    /// List all open projects and their current snapshots.
    List,
    /// Open a repository directory with an agent driver.
    Open {
        cwd: String,
        #[arg(long, default_value = "agy")]
        driver: String,
    },
    /// Send a prompt (and optional image paths) to an open project.
    Prompt {
        project_id: String,
        text: String,
        #[arg(long = "image")]
        images: Vec<String>,
        #[arg(long)]
        wait: bool,
    },
    /// Cancel an active turn on a project.
    Cancel { project_id: String },
    /// Resolve a pending permission request.
    Permit {
        project_id: String,
        request_id: String,
        #[arg(long)]
        option: Option<String>,
    },
    /// Close a project and terminate its agent process.
    Close { project_id: String },
    /// Stream live daemon events as NDJSON to stdout.
    Watch,
}

pub async fn run_ctl(socket_path: &Path, cmd: CtlCommand) -> Result<(), String> {
    let stream = UnixStream::connect(socket_path)
        .await
        .map_err(|err| format!("failed to connect to {}: {err}", socket_path.display()))?;
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half).lines();

    match cmd {
        CtlCommand::Ping => send_and_print(&mut write_half, &mut reader, ClientCommand::Ping).await,
        CtlCommand::List => {
            send_and_print(&mut write_half, &mut reader, ClientCommand::ListProjects).await
        }
        CtlCommand::Open { cwd, driver } => {
            let driver = parse_driver(&driver)?;
            send_and_print(
                &mut write_half,
                &mut reader,
                ClientCommand::OpenProject {
                    cwd,
                    driver,
                    custom_command: None,
                },
            )
            .await
        }
        CtlCommand::Prompt {
            project_id,
            text,
            images,
            wait,
        } => {
            if wait {
                let _ = send_rpc(&mut write_half, &mut reader, ClientCommand::Subscribe).await?;
            }
            let attachments = images
                .into_iter()
                .map(|path| Attachment::ImagePath {
                    path,
                    mime_type: None,
                })
                .collect();
            let resp = send_rpc(
                &mut write_half,
                &mut reader,
                ClientCommand::SendPrompt {
                    project_id: project_id.clone(),
                    input: PromptInput { text, attachments },
                },
            )
            .await?;
            println!("{resp}");
            if wait {
                while let Ok(Some(line)) = reader.next_line().await {
                    println!("{line}");
                    if let Ok(ServerEnvelope::Event(ev)) = serde_json::from_str(&line) {
                        match ev {
                            huma_protocol::ServerEvent::TurnCompleted {
                                project_id: pid, ..
                            } if pid == project_id => break,
                            huma_protocol::ServerEvent::ProjectStatusChanged {
                                project_id: pid,
                                status: huma_protocol::ProjectStatus::Error,
                                ..
                            } if pid == project_id => break,
                            _ => {}
                        }
                    }
                }
            }
            Ok(())
        }
        CtlCommand::Cancel { project_id } => {
            send_and_print(
                &mut write_half,
                &mut reader,
                ClientCommand::CancelTurn { project_id },
            )
            .await
        }
        CtlCommand::Permit {
            project_id,
            request_id,
            option,
        } => {
            send_and_print(
                &mut write_half,
                &mut reader,
                ClientCommand::RespondPermission {
                    project_id,
                    request_id,
                    option_id: option,
                },
            )
            .await
        }
        CtlCommand::Close { project_id } => {
            send_and_print(
                &mut write_half,
                &mut reader,
                ClientCommand::CloseProject { project_id },
            )
            .await
        }
        CtlCommand::Watch => {
            let _ = send_rpc(&mut write_half, &mut reader, ClientCommand::Subscribe).await?;
            while let Ok(Some(line)) = reader.next_line().await {
                println!("{line}");
            }
            Ok(())
        }
    }
}

fn parse_driver(raw: &str) -> Result<DriverKind, String> {
    match raw {
        "agy" => Ok(DriverKind::Agy),
        "opencode" => Ok(DriverKind::Opencode),
        "claude" | "claude_code" => Ok(DriverKind::ClaudeCode),
        other => Err(format!("unsupported driver `{other}`")),
    }
}

async fn send_and_print(
    write_half: &mut tokio::net::unix::OwnedWriteHalf,
    reader: &mut tokio::io::Lines<BufReader<tokio::net::unix::OwnedReadHalf>>,
    command: ClientCommand,
) -> Result<(), String> {
    let raw = send_rpc(write_half, reader, command).await?;
    println!("{raw}");
    Ok(())
}

async fn send_rpc(
    write_half: &mut tokio::net::unix::OwnedWriteHalf,
    reader: &mut tokio::io::Lines<BufReader<tokio::net::unix::OwnedReadHalf>>,
    command: ClientCommand,
) -> Result<String, String> {
    let req_id = Uuid::new_v4().to_string();
    let req = ClientRequest {
        id: req_id.clone(),
        command,
    };
    let line = serde_json::to_string(&req).map_err(|e| e.to_string())?;
    write_half
        .write_all(format!("{line}\n").as_bytes())
        .await
        .map_err(|e| e.to_string())?;
    write_half.flush().await.map_err(|e| e.to_string())?;

    while let Some(resp_line) = reader.next_line().await.map_err(|e| e.to_string())? {
        if let Ok(ServerEnvelope::Response(resp)) =
            serde_json::from_str::<ServerEnvelope>(&resp_line)
            && resp.id == req_id
        {
            if !resp.ok {
                return Err(resp.error.unwrap_or_else(|| "daemon error".into()));
            }
            return Ok(resp_line);
        }
    }
    Err("daemon closed connection before replying".into())
}

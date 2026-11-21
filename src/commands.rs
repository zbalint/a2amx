//! Command dispatch over the CLI.

mod daemon;
mod gateway;
mod grants;
mod home;
mod launch;
mod sessions;
mod team;

use self::{
    daemon::{run_daemon, run_daemon_background, run_daemon_status, run_stop},
    gateway::{
        run_gateway, run_gateway_status, run_gateway_stop, run_gateway_token, run_gateway_up,
    },
    grants::auto_grants_for,
    home::resolve_home,
    launch::run_new,
    sessions::{
        run_attach_command, run_cancel, run_confine_show, run_explain, run_kill, run_list,
        run_messages, run_reset, run_screen,
    },
    team::{run_team_down, run_team_reset, run_team_status, run_team_up},
};
use crate::attach::AttachOptions;
use crate::bridge;
use crate::cli::{
    Cli, Command, ConfigAction, ConfineAction, DaemonAction, GatewayAction, TeamAction,
};
use crate::hook;
use crate::mcp;
use crate::statusline;
use crate::terminal::write_stdout;
use anyhow::{Context, anyhow, bail};
use std::path::PathBuf;

pub async fn run(cli: Cli) -> anyhow::Result<std::process::ExitCode> {
    let home_arg = cli.home;
    match &cli.command {
        Command::Mcp { channel, optional } => {
            return mcp::run(*channel, *optional)
                .await
                .map(|_| std::process::ExitCode::SUCCESS);
        }
        Command::Hook => return hook::run().await.map(|_| std::process::ExitCode::SUCCESS),
        Command::Statusline { report, harness } => {
            return statusline::run(*report, *harness)
                .await
                .map(|_| std::process::ExitCode::SUCCESS);
        }
        Command::OmpBridge => return bridge::run().await.map(|_| std::process::ExitCode::SUCCESS),
        Command::Confine {
            action: ConfineAction::List,
        } => {
            write_stdout(crate::grant::render_list().into_bytes()).await?;
            return Ok(std::process::ExitCode::SUCCESS);
        }
        Command::Confine {
            action: ConfineAction::Suggest { dir },
        } => {
            let dir = match dir {
                Some(dir) => dir.clone(),
                None => std::env::current_dir()?,
            };
            let host_home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_default();
            let found = tokio::task::spawn_blocking(move || -> anyhow::Result<_> {
                let metadata = std::fs::metadata(&dir)
                    .with_context(|| format!("cannot read project directory {}", dir.display()))?;
                if !metadata.is_dir() {
                    bail!("cannot read project directory {}", dir.display());
                }
                Ok(auto_grants_for(&dir, &host_home))
            })
            .await??;
            let grants = found.iter().map(|(name, _)| *name).collect::<Vec<_>>();
            write_stdout(crate::suggest::render(&grants).into_bytes()).await?;
            return Ok(std::process::ExitCode::SUCCESS);
        }
        _ => {}
    }
    let explicit_home = home_arg.is_some();
    let home = resolve_home(home_arg)?;
    let client_overrides = crate::config::ClientOverrides {
        prefix: cli.prefix,
        viewer_after_secs: cli.viewer_after,
    };
    let show_client_overrides = client_overrides.clone();
    let client_layout = crate::layout::Layout::new(home.clone());
    let loaded_client = tokio::task::spawn_blocking(move || {
        crate::config::load_client(&client_layout, &client_overrides)
    })
    .await??;
    let attach_options = AttachOptions {
        prefix: loaded_client.settings.prefix,
        viewer_after: loaded_client.settings.viewer_after,
    };
    let result = match cli.command {
        Command::Gateway { action } => match action {
            GatewayAction::Token => run_gateway_token(&home, explicit_home).await,
            GatewayAction::Up { pair, foreground } => {
                run_gateway_up(home, pair, foreground, explicit_home).await
            }
            GatewayAction::Stop => return run_gateway_stop(&home).await,
            GatewayAction::Status => return run_gateway_status(&home).await,
            GatewayAction::Run { pair, pair_only } => run_gateway(home, pair, pair_only).await,
        },
        Command::Config {
            action: ConfigAction::Show,
        } => {
            let env_value = match std::env::var("A2AMX_OWNER_AWAY_SECS") {
                Ok(value) => Some(
                    value
                        .parse::<u64>()
                        .with_context(|| "A2AMX_OWNER_AWAY_SECS must be an integer")?,
                ),
                Err(std::env::VarError::NotPresent) => None,
                Err(error) => return Err(anyhow!("cannot read A2AMX_OWNER_AWAY_SECS: {error}")),
            };
            let layout = crate::layout::Layout::new(home.clone());
            let client_overrides = show_client_overrides.clone();
            let output = tokio::task::spawn_blocking(move || {
                let mut output = crate::config::show_daemon(&layout, env_value)?;
                output.push('\n');
                output.push_str(&crate::config::show_gateway(&layout)?);
                output.push('\n');
                output.push_str(&crate::config::show_client(&layout, &client_overrides)?);
                Ok::<_, anyhow::Error>(output)
            })
            .await??;
            write_stdout(output.into_bytes()).await
        }
        Command::Daemon { action } => match action {
            DaemonAction::Start {
                listen,
                host_name,
                foreground,
                owner_away_secs,
            } => {
                if foreground {
                    run_daemon(home, listen, host_name, owner_away_secs, explicit_home).await
                } else {
                    run_daemon_background(home, listen, host_name, owner_away_secs, explicit_home)
                        .await
                }
            }
            DaemonAction::Status => return run_daemon_status(home).await,
            DaemonAction::Stop { yes, now } => run_stop(home, yes, now).await,
        },
        options @ Command::New { .. } => run_new(home, attach_options, options).await,
        Command::List {
            details,
            team,
            viewer,
        } => run_list(home, details, team, viewer).await,
        Command::Team { action } => match action {
            TeamAction::Status { file } => return run_team_status(home, file).await,
            TeamAction::Up {
                file,
                detach,
                dry_run,
                yes,
                items,
            } => run_team_up(home, attach_options, file, detach, dry_run, yes, items).await,
            TeamAction::Down { file, names, now } => run_team_down(home, file, names, now).await,
            TeamAction::Reset {
                file,
                names,
                except,
                include_attached,
                yes,
            } => run_team_reset(home, file, names, except, include_attached, yes).await,
        },
        Command::Attach { session, force } => {
            run_attach_command(home, attach_options, session, force).await
        }
        Command::Kill {
            session,
            exited,
            yes,
            now,
        } => run_kill(home, session, exited, yes, now).await,
        Command::Reset { session } => run_reset(home, session).await,
        Command::Explain { session } => run_explain(home, session).await,
        Command::Screen { session, rows } => run_screen(home, session, rows).await,
        Command::Messages { session, state } => run_messages(home, session, state).await,
        Command::Cancel { message } => run_cancel(home, message).await,
        Command::Confine {
            action: ConfineAction::Show { session },
        } => run_confine_show(home, session).await,
        Command::Confine { .. }
        | Command::Mcp { .. }
        | Command::Hook
        | Command::Statusline { .. }
        | Command::OmpBridge => {
            unreachable!("early dispatch returned before resolving home")
        }
    };
    result.map(|_| std::process::ExitCode::SUCCESS)
}

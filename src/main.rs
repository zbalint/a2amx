use clap::Parser;
use tracing_subscriber::prelude::*;

use a2amx::cli::Cli;
use a2amx::commands;
use a2amx::terminal::{write_stderr, write_stdout};

fn log_target_allowed(target: &str) -> bool {
    ![
        "tungstenite",
        "tokio_tungstenite",
        "ureq",
        "ureq_proto",
        "rustls",
    ]
    .iter()
    .any(|prefix| {
        target == *prefix
            || target
                .strip_prefix(prefix)
                .is_some_and(|rest| rest.starts_with("::"))
    })
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(std::io::stderr)
                .with_filter(tracing_subscriber::EnvFilter::from_default_env())
                .with_filter(tracing_subscriber::filter::FilterFn::new(|metadata| {
                    log_target_allowed(metadata.target())
                })),
        )
        .init();
    let result = match Cli::try_parse() {
        Ok(cli) => match a2amx::privilege::ensure_not_root() {
            Ok(()) => commands::run(cli).await,
            Err(error) => Err(error),
        },
        Err(error) if !error.use_stderr() => write_stdout(error.to_string().into_bytes())
            .await
            .map(|_| std::process::ExitCode::SUCCESS),
        Err(error) => Err(error.into()),
    };
    match result {
        Ok(code) => code,
        Err(error) => {
            let _ = write_stderr(format!("a2amx: {error:#}\n").into_bytes()).await;
            std::process::ExitCode::FAILURE
        }
    }
}

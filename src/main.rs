use clap::Parser;

use a2amx::cli::Cli;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let cli = Cli::parse();
    tracing::debug!(?cli, "parsed command line");
    todo!("dispatch subcommands (spec 1)")
}

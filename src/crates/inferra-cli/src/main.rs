mod cli;
mod commands;
mod context;
mod ui;

use anyhow::Result;
use clap::Parser;

use crate::cli::Cli;

#[tokio::main]
async fn main() -> Result<()> {
    context::init_tracing();
    let cli = Cli::parse();
    commands::dispatch(cli).await
}

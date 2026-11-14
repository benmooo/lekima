//! lekima command line interface.

#![forbid(unsafe_code)]

mod cli;
mod commands;
mod daemon;
mod output;

use std::io::IsTerminal;

use clap::Parser;
use lekima_config::Paths;
use tracing_subscriber::EnvFilter;

use crate::{
    cli::Cli,
    output::{Failure, print_failure},
};

fn main() -> std::process::ExitCode {
    match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime.block_on(async_main()),
        Err(error) => {
            eprintln!("error: failed to start the async runtime: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

async fn async_main() -> std::process::ExitCode {
    let cli = Cli::parse();
    let json = cli.json;

    let paths = match Paths::resolve() {
        Ok(paths) => paths,
        Err(error) => return fail(&Failure::from(error), json),
    };
    let config = match paths.config() {
        Ok(config) => config,
        Err(error) => return fail(&Failure::from(error), json),
    };
    init_tracing(&config.daemon.log_level);

    match commands::execute(cli, paths, config).await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(failure) => fail(&failure, json),
    }
}

fn init_tracing(default_level: &str) {
    let filter = std::env::var("LEKIMA_LOG")
        .or_else(|_| std::env::var("RUST_LOG"))
        .unwrap_or_else(|_| default_level.to_owned());
    let filter = EnvFilter::try_new(filter).unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_ansi(std::io::stderr().is_terminal())
        .with_writer(std::io::stderr)
        .try_init();
}

fn fail(failure: &Failure, json: bool) -> std::process::ExitCode {
    if json {
        print_failure(failure, true);
    } else {
        print_failure(failure, false);
    }
    std::process::ExitCode::from(failure.exit_code())
}

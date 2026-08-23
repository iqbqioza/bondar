mod cli;
mod command;
mod compose;
mod config;
mod docker;
mod error;
mod features;
mod host;
mod lifecycle;

use clap::Parser;

use cli::{Cli, Commands};

fn main() {
    let cli = Cli::parse();
    let ws = cli.workspace_folder;
    let cfg = cli.config;

    let result = match cli.command {
        Commands::Build { no_cache } => command::build::run(ws, cfg, no_cache),
        Commands::Up {
            remove_existing_container,
            no_build,
            no_cache,
        } => command::up::run(ws, cfg, remove_existing_container, no_build, no_cache),
        Commands::Down => command::down::run(ws, cfg),
        Commands::Exec {
            user,
            workdir,
            command,
        } => command::exec::run(ws, cfg, user, workdir, command),
        Commands::Shell => command::shell::run(ws, cfg),
        Commands::Logs { follow, tail } => command::logs::run(ws, cfg, follow, tail),
        Commands::ReadConfiguration {
            include_merged_configuration,
        } => command::read_configuration::run(ws, cfg, include_merged_configuration),
    };

    if let Err(e) = result {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}

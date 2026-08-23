mod cli;
mod command;
mod compose;
mod config;
mod docker;
mod error;
mod host;
mod lifecycle;

use clap::Parser;

use cli::{Cli, Commands};

fn main() {
    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Build {
            workspace_folder,
            config,
            no_cache,
        } => command::build::run(workspace_folder, config, no_cache),
        Commands::Up {
            workspace_folder,
            config,
            remove_existing_container,
            no_build,
        } => command::up::run(
            workspace_folder,
            config,
            remove_existing_container,
            no_build,
        ),
        Commands::Down {
            workspace_folder,
            config,
        } => command::down::run(workspace_folder, config),
        Commands::Exec {
            workspace_folder,
            config,
            user,
            workdir,
            command,
        } => command::exec::run(workspace_folder, config, user, workdir, command),
        Commands::Shell {
            workspace_folder,
            config,
        } => command::shell::run(workspace_folder, config),
        Commands::Logs {
            workspace_folder,
            config,
            follow,
            tail,
        } => command::logs::run(workspace_folder, config, follow, tail),
        Commands::ReadConfiguration {
            workspace_folder,
            config,
        } => command::read_configuration::run(workspace_folder, config),
    };

    if let Err(e) = result {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}

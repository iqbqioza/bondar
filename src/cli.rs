use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "bondar", version, about = "devcontainer alternative without Node.js/VSCode dependencies", long_about = None)]
pub struct Cli {
    /// Path to workspace folder (defaults to current directory)
    #[arg(long, global = true)]
    pub workspace_folder: Option<PathBuf>,

    /// Override path to devcontainer.json
    #[arg(long, global = true)]
    pub config: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Build the dev container image
    Build {
        /// Do not use cache when building
        #[arg(long)]
        no_cache: bool,
    },

    /// Create and start the dev container
    Up {
        /// Remove existing container if it exists
        #[arg(long)]
        remove_existing_container: bool,

        /// Do not build image even if build is configured
        #[arg(long)]
        no_build: bool,

        /// Do not use cache when building (implies build)
        #[arg(long)]
        no_cache: bool,
    },

    /// Stop and remove the dev container
    Down,

    /// Execute a command inside the dev container
    Exec {
        /// User to run as (overrides remoteUser)
        #[arg(long)]
        user: Option<String>,

        /// Working directory inside container
        #[arg(long)]
        workdir: Option<String>,

        /// Command to execute
        #[arg(last = true, required = true)]
        command: Vec<String>,
    },

    /// Start an interactive shell inside the dev container
    Shell,

    /// Show container logs
    Logs {
        /// Follow log output
        #[arg(long)]
        follow: bool,

        /// Number of lines to show from the end
        #[arg(long)]
        tail: Option<String>,
    },

    /// Validate and print devcontainer configuration
    ReadConfiguration {
        /// Print the merged configuration (env, secrets, defaults applied)
        #[arg(long)]
        include_merged_configuration: bool,
    },
}

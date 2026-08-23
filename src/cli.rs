use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "bondar", version, about = "devcontainer alternative without Node.js/VSCode dependencies", long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Build the dev container image
    Build {
        /// Path to workspace folder (defaults to current directory)
        #[arg(long)]
        workspace_folder: Option<PathBuf>,

        /// Override path to devcontainer.json
        #[arg(long)]
        config: Option<PathBuf>,

        /// Do not use cache when building
        #[arg(long)]
        no_cache: bool,
    },

    /// Create and start the dev container
    Up {
        /// Path to workspace folder (defaults to current directory)
        #[arg(long)]
        workspace_folder: Option<PathBuf>,

        /// Override path to devcontainer.json
        #[arg(long)]
        config: Option<PathBuf>,

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
    Down {
        /// Path to workspace folder (defaults to current directory)
        #[arg(long)]
        workspace_folder: Option<PathBuf>,

        /// Override path to devcontainer.json
        #[arg(long)]
        config: Option<PathBuf>,
    },

    /// Execute a command inside the dev container
    Exec {
        /// Path to workspace folder (defaults to current directory)
        #[arg(long)]
        workspace_folder: Option<PathBuf>,

        /// Override path to devcontainer.json
        #[arg(long)]
        config: Option<PathBuf>,

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
    Shell {
        /// Path to workspace folder (defaults to current directory)
        #[arg(long)]
        workspace_folder: Option<PathBuf>,

        /// Override path to devcontainer.json
        #[arg(long)]
        config: Option<PathBuf>,
    },

    /// Show container logs
    Logs {
        /// Path to workspace folder (defaults to current directory)
        #[arg(long)]
        workspace_folder: Option<PathBuf>,

        /// Override path to devcontainer.json
        #[arg(long)]
        config: Option<PathBuf>,

        /// Follow log output
        #[arg(long)]
        follow: bool,

        /// Number of lines to show from the end
        #[arg(long)]
        tail: Option<String>,
    },

    /// Validate and print devcontainer configuration
    ReadConfiguration {
        /// Path to workspace folder (defaults to current directory)
        #[arg(long)]
        workspace_folder: Option<PathBuf>,

        /// Override path to devcontainer.json
        #[arg(long)]
        config: Option<PathBuf>,

        /// Print the merged configuration (env, secrets, defaults applied)
        #[arg(long)]
        include_merged_configuration: bool,
    },
}

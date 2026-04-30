mod build;
mod commands;

use crate::{build::rebuild, commands::*};
use anyhow::Result;
use clap::{Parser, Subcommand};
use justpkg::Shell;
use std::path::PathBuf;

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    /// Sets a custom config file
    #[arg(short, long, value_name = "FILE")]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Rebuild from config
    Rebuild,
    /// Adds a package
    Add {
        /// Name
        name: String,
        /// Git remote URL
        url: String,
        /// Current directory build script paths
        #[arg(long, short)]
        script: Option<PathBuf>,
        /// Commit hash
        #[arg(long, short)]
        commit: Option<String>,
        /// Repo relative binary paths (If not provided will search for binaries FIRST time)
        #[arg(long, short)]
        binaries: Vec<PathBuf>,
        /// Dependencies
        #[arg(long, short)]
        dependencies: Vec<String>,
    },
    /// Updates packages
    Update {
        /// Names or names and refs, each concatenated with '@'
        names: Vec<String>,
    },
    /// Removes packages
    Rm {
        // Names
        names: Vec<String>,
    },
    /// Lists packages
    Ls,
    /// Displays info of a package
    Info {
        /// Names
        name: String,
    },
    /// Generates shell completion
    Init {
        /// Shell
        shell: Shell,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    if let Some(config_path) = cli.config.as_deref() {
        println!("Value for config: {}", config_path.display());
    }

    match cli.command {
        Some(Commands::Rebuild) => {
            rebuild()?;
            Ok(())
        }
        Some(Commands::Add {
            name,
            url,
            script,
            commit,
            binaries,
            dependencies,
        }) => {
            let oid: Option<git2::Oid> = commit.map(|c| git2::Oid::from_str(&c)).transpose()?;
            add(name, url, script, oid, binaries, dependencies)?;
            Ok(())
        }
        Some(Commands::Update { names }) => {
            update(names)?;
            Ok(())
        }
        Some(Commands::Rm { names }) => {
            remove(names)?;
            Ok(())
        }
        Some(Commands::Ls) => {
            list()?;
            Ok(())
        }
        Some(Commands::Info { name }) => {
            info(name)?;
            Ok(())
        }
        Some(Commands::Init { shell }) => {
            init(shell)?;
            Ok(())
        }
        None => Ok(()),
    }
}

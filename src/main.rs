mod build;
mod commands;

use crate::commands::*;
use anyhow::Result;
use clap::{Parser, Subcommand};
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
    /// Adds a package
    Add {
        packages: Vec<String>,
        #[arg(short, long, value_name = "FILE")]
        script: Option<PathBuf>,
        #[arg(short, long, value_name = "COMMAND")]
        command: Option<String>,
        #[arg(short, long, value_name = "FILE")]
        binary: Option<PathBuf>,
    },
    /// Updates packages
    Update {
        packages: Vec<String>,
    },
    /// Removes packages
    Rm {
        packages: Vec<String>,
    },
    /// Lists packages
    Ls,

    Info {
        packages: Vec<String>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    if let Some(config_path) = cli.config.as_deref() {
        println!("Value for config: {}", config_path.display());
    }

    let base = microxdg::Xdg::new()?.data()?.join("justpkg");
    match &cli.command {
        Some(Commands::Add {
            packages,
            script,
            command,
            binary,
        }) => {
            add(
                packages,
                &base,
                script.as_deref(),
                command.as_deref(),
                binary.as_deref(),
            )?;
            Ok(())
        }
        Some(Commands::Update { packages }) => {
            update(packages, &base)?;
            Ok(())
        }
        Some(Commands::Rm { packages }) => {
            remove(packages, &base)?;
            Ok(())
        }
        Some(Commands::Ls) => {
            list(&base)?;
            Ok(())
        }
        Some(Commands::Info { packages }) => {
            info(&packages, &base)?;
            Ok(())
        }
        None => return Ok(()),
    }

    // Continued program logic goes here...
}

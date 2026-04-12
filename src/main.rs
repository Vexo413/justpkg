mod file;

use crate::file::*;
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
        urls: Vec<String>,
    },
    /// Updates packages
    Update {
        urls: Vec<String>,
    },
    /// Removes packages
    Rm {
        urls: Vec<String>,
    },
    /// Lists packages
    Ls,

    Info {
        urls: Vec<String>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    if let Some(config_path) = cli.config.as_deref() {
        println!("Value for config: {}", config_path.display());
    }

    let base = microxdg::Xdg::new()?.data()?.join("justpkg");
    match &cli.command {
        Some(Commands::Add { urls }) => {
            add(urls, base)?;
            Ok(())
        }
        Some(Commands::Update { urls }) => {
            update(urls, base)?;
            Ok(())
        }
        Some(Commands::Rm { urls }) => {
            remove(urls, base)?;
            Ok(())
        }
        Some(Commands::Ls) => {
            list(base)?;
            Ok(())
        }
        Some(Commands::Info { urls }) => {
            info(&urls, base)?;
            Ok(())
        }
        None => return Ok(()),
    }

    // Continued program logic goes here...
}

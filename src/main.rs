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
    /// does testing things
    Add {
        url: String,
    },
    Update {
        packages: Vec<String>,
    },
    Rm {
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
        Some(Commands::Add { url }) => {
            add(url, base)?;
            Ok(())
        }
        Some(Commands::Update { packages }) => {
            update(packages, base)?;
            Ok(())
        }
        Some(Commands::Rm { packages }) => {
            remove(packages, base)?;
            Ok(())
        }
        None => return Ok(()),
    }

    // Continued program logic goes here...
}

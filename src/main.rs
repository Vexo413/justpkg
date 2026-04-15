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
        package: String,
        build_script: PathBuf,
        #[arg(required = true)]
        binaries: Vec<PathBuf>,
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
        package: String,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    if let Some(config_path) = cli.config.as_deref() {
        println!("Value for config: {}", config_path.display());
    }

    let base = microxdg::Xdg::new()?.data()?.join("justpkg");

    match cli.command {
        Some(Commands::Add {
            package,
            build_script,
            binaries,
        }) => {
            add(package, build_script, binaries)?;
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
        Some(Commands::Ls) => {
            list(base)?;
            Ok(())
        }
        Some(Commands::Info { package }) => {
            info(package, base)?;
            Ok(())
        }
        None => Ok(()),
    }

    // Continued program logic goes here...
}

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
    /// Rebuild from config
    Rebuild,
    /// Adds a package
    Add {
        package: String,
        build_script: PathBuf,
        #[arg(long, short)]
        r: Option<String>,
        #[arg(long, short)]
        commit: Option<String>,
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

    match cli.command {
        Some(Commands::Rebuild) => {
            rebuild()?;
            Ok(())
        }
        Some(Commands::Add {
            package,
            build_script,
            r,
            commit,
            binaries,
        }) => {
            let oid: Option<git2::Oid> = commit.map(|c| git2::Oid::from_str(&c)).transpose()?;
            add(package, build_script, r, oid, binaries)?;
            Ok(())
        }
        Some(Commands::Update { packages }) => {
            update(packages)?;
            Ok(())
        }
        Some(Commands::Rm { packages }) => {
            remove(packages)?;
            Ok(())
        }
        Some(Commands::Ls) => {
            list()?;
            Ok(())
        }
        Some(Commands::Info { package }) => {
            info(package)?;
            Ok(())
        }
        None => Ok(()),
    }
}

mod build;
mod commands;

use crate::{build::rebuild, commands::*};
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
        name: String,
        url: String,
        #[arg(long, short)]
        build_script: Option<PathBuf>,
        #[arg(long, short)]
        commit: Option<String>,
        #[arg(required = true)]
        binaries: Vec<PathBuf>,
    },
    /// Updates packages
    Update {
        names: Vec<String>,
    },
    /// Removes packages
    Rm {
        names: Vec<String>,
    },
    /// Lists packages
    Ls,

    Info {
        name: String,
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
            build_script,
            commit,
            binaries,
        }) => {
            let oid: Option<git2::Oid> = commit.map(|c| git2::Oid::from_str(&c)).transpose()?;
            add(name, url, build_script, oid, binaries)?;
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
        None => Ok(()),
    }
}

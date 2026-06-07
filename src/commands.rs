use crate::build::rebuild;
use anyhow::{Context, Result, anyhow};
use git2::Oid;
use justpkg::{Package, Shell, get_packages, millis_to_datetime, resolve_remote_ref, save_repos};
use microxdg::Xdg;
use sha2::Digest;
use std::{
    env, fs,
    io::Write,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

pub fn init(shell: Shell) -> Result<()> {
    let xdg = Xdg::new().context("Failed to find XDG directories")?;
    let bin_path = xdg
        .data()
        .context("Failed to get XDG data directory")?
        .join("justpkg/bin");

    let bin_path_str = bin_path.to_string_lossy();

    match shell {
        Shell::Bash | Shell::Zsh => {
            println!("export PATH=\"{}:$PATH\"", bin_path_str);
        }
        Shell::Fish => {
            println!("fish_add_path \"{}\"", bin_path_str);
        }
        Shell::Nu => {
            println!(
                "$env.PATH = ($env.PATH | split-row (char esep) | prepend '{}' | uniq)",
                bin_path_str
            );
        }
    }

    Ok(())
}

pub fn add(
    name: String,
    url: String,
    install_script: Option<PathBuf>,
    uninstall_script: Option<PathBuf>,
    commit: Option<Oid>,
    dependencies: Vec<String>,
) -> Result<()> {
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(anyhow!("{name} is not a valid package name"));
    }
    let config_path = Xdg::new()
        .context("Failed to initialize XDG directories")?
        .config()
        .context("Failed to get XDG config directory")?
        .join("justpkg");
    let path = config_path.join("repos.json");

    let mut packages = get_packages(&path).context("Failed to load package database")?;

    let install_scripts_path = Xdg::new()?.config()?.join("justpkg/install-scripts");
    fs::create_dir_all(&install_scripts_path)?;
    let install_script = match install_script {
        Some(path) => {
            let src = env::current_dir()?.join(&path);
            let dst = install_scripts_path.join(format!("{}.sh", &name));
            fs::copy(src, &dst)?;
            dst
        }
        None => {
            let editor = env::var("EDITOR").unwrap_or_else(|_| "vi".to_string());
            let path = install_scripts_path.join(format!("{}.sh", &name));
            let mut file = fs::File::create(&path)?;
            file.write_all(String::from("#!/usr/bin/env bash\nset -euo pipefail").as_bytes())?;
            Command::new(editor).arg(&path).status()?;
            path
        }
    };
    let install_hash = hex::encode(sha2::Sha256::digest(std::fs::read(&install_script)?));

    let uninstall_scripts_path = Xdg::new()?.config()?.join("justpkg/uninstall-scripts");
    fs::create_dir_all(&uninstall_scripts_path)?;
    let uninstall_script = match uninstall_script {
        Some(path) => {
            let src = env::current_dir()?.join(&path);
            let dst = uninstall_scripts_path.join(format!("{}.sh", &name));
            fs::copy(src, &dst)?;
            dst
        }
        None => {
            let editor = env::var("EDITOR").unwrap_or_else(|_| "vi".to_string());
            let path = uninstall_scripts_path.join(format!("{}.sh", &name));
            let mut file = fs::File::create(&path)?;
            file.write_all(String::from("#!/usr/bin/env bash\nset -euo pipefail").as_bytes())?;
            Command::new(editor).arg(&path).status()?;
            path
        }
    };
    let uninstall_hash = hex::encode(sha2::Sha256::digest(std::fs::read(&uninstall_script)?));

    let commit = match commit {
        Some(c) => c,
        None => resolve_remote_ref(&url, "HEAD")
            .with_context(|| format!("Failed to resolve HEAD for {}", url))?,
    }
    .to_string();

    let entry = Package {
        commit,
        url,
        synced_at: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("Failed to get current time")?
            .as_millis(),
        install_script,
        install_hash,
        uninstall_script,
        uninstall_hash,
        dependencies: dependencies.into_iter().collect(),
    };

    let changed = match packages.get(&name) {
        Some(old) => old != &entry,
        None => true,
    };

    if changed {
        packages.insert(name, entry);
        save_repos(&packages).context("Failed to save package database")?;
    }

    rebuild().context("Failed to rebuild packages")?;

    Ok(())
}

fn split_name_ref(s: &str) -> (&str, Option<&str>) {
    match s.split_once('@') {
        Some((name, reference)) => (name, Some(reference)),
        None => (s, None),
    }
}

pub fn update(names: Vec<String>) -> Result<()> {
    let config_path = Xdg::new()
        .context("Failed to initialize XDG directories")?
        .config()
        .context("Failed to get XDG config directory")?
        .join("justpkg");
    let path = config_path.join("repos.json");

    let mut packages = get_packages(&path).context("Failed to load package database")?;

    let mut changed = false;

    for (name, reference) in names.iter().map(|n| split_name_ref(n)) {
        let package = packages
            .get_mut(name)
            .ok_or_else(|| anyhow!("{} not found", name))?;

        let latest =
            resolve_remote_ref(&package.url, reference.unwrap_or("HEAD")).with_context(|| {
                format!(
                    "Failed to resolve remote ref '{}' for {}",
                    reference.unwrap_or("HEAD"),
                    package.url
                )
            })?;

        let current = git2::Oid::from_str(&package.commit)
            .with_context(|| format!("Failed to parse commit hash '{}'", package.commit))?;

        if current != latest {
            package.commit = latest.to_string();
            package.synced_at = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .context("Failed to get current time")?
                .as_millis();

            changed = true;
        }
    }

    if changed {
        save_repos(&packages).context("Failed to save package database")?;
        rebuild().context("Failed to rebuild packages")?;
    }

    println!("Finished");
    Ok(())
}

pub fn remove(names: Vec<String>) -> Result<()> {
    let config_path = Xdg::new()
        .context("Failed to initialize XDG directories")?
        .config()
        .context("Failed to get XDG config directory")?
        .join("justpkg");
    let path = config_path.join("repos.json");

    let mut packages = get_packages(&path).context("Failed to load package database")?;

    let mut changed = false;

    for name in names {
        if packages.remove(&name).is_some() {
            changed = true;
            println!("Removed: {}", name);
        }
    }

    if changed {
        save_repos(&packages).context("Failed to save package database")?;
        rebuild().context("Failed to rebuild packages")?;
    }

    println!("Finished");
    Ok(())
}

pub fn list() -> Result<()> {
    let config_path = Xdg::new()
        .context("Failed to initialize XDG directories")?
        .config()
        .context("Failed to get XDG config directory")?
        .join("justpkg");
    let path = config_path.join("repos.json");

    let packages = get_packages(&path).context("Failed to load package database")?;

    for (name, package) in packages.iter() {
        println!(
            "{}: {} | {}",
            name,
            package.url,
            millis_to_datetime(package.synced_at as u64),
        );
    }

    Ok(())
}

pub fn info(name: String) -> Result<()> {
    let config_path = Xdg::new()
        .context("Failed to initialize XDG directories")?
        .config()
        .context("Failed to get XDG config directory")?
        .join("justpkg");
    let path = config_path.join("repos.json");

    let packages = get_packages(&path).context("Failed to load package database")?;

    let package = packages
        .get(&name)
        .ok_or_else(|| anyhow!("{} not found", name))?;

    println!("Name: {}", name);
    println!("URL: {}", package.url);
    println!(
        "Synced at: {}",
        millis_to_datetime(package.synced_at as u64)
    );
    println!("Commit: {}", package.commit);
    println!("Dependencies: {:?}", package.dependencies);

    Ok(())
}

use crate::build::rebuild;
use anyhow::{Context, Result, anyhow};
use git2::Oid;
use justpkg::{Package, Shell, get_packages, millis_to_datetime, resolve_remote_ref, save_repos};
use microxdg::Xdg;
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
    build_script: Option<PathBuf>,
    commit: Option<Oid>,
    binaries: Vec<PathBuf>,
) -> Result<()> {
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(anyhow!("{name} is not a valid package name"));
    }
    let mut repo_infos = get_packages().context("Failed to load package database")?;

    let build_script_paths = Xdg::new()?.config()?.join("justpkg/build-scripts");
    fs::create_dir_all(&build_script_paths)?;
    let build_script = match build_script {
        Some(path) => {
            let src = env::current_dir()?.join(&path);
            let dst = build_script_paths.join(format!("{}.sh", &name));
            fs::copy(src, &dst)?;
            dst
        }
        None => {
            let editor = env::var("EDITOR").unwrap_or_else(|_| "vi".to_string());
            let path = build_script_paths.join(format!("{}.sh", &name));
            let mut file = fs::File::create(&path)?;
            file.write_all(String::from("#!/usr/bin/env bash\nset -euo pipefail").as_bytes())?;
            Command::new(editor).arg(&path).status()?;
            path
        }
    };

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
        binaries,
        build_script,
    };

    let changed = match repo_infos.get(&name) {
        Some(old) => old != &entry,
        None => true,
    };

    if changed {
        repo_infos.insert(name, entry);
        save_repos(&repo_infos).context("Failed to save package database")?;
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
    let mut repo_infos = get_packages().context("Failed to load package database")?;
    let mut changed = false;

    for (name, reference) in names.iter().map(|n| split_name_ref(n)) {
        let package = repo_infos
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
        save_repos(&repo_infos).context("Failed to save package database")?;
        rebuild().context("Failed to rebuild packages")?;
    }

    println!("Finished");
    Ok(())
}

pub fn remove(names: Vec<String>) -> Result<()> {
    let mut repo_infos = get_packages().context("Failed to load package database")?;
    let mut changed = false;

    for name in names {
        if repo_infos.remove(&name).is_some() {
            changed = true;
            println!("Removed: {}", name);
        }
    }

    if changed {
        save_repos(&repo_infos).context("Failed to save package database")?;
        rebuild().context("Failed to rebuild packages")?;
    }

    println!("Finished");
    Ok(())
}

pub fn list() -> Result<()> {
    let repo_infos = get_packages().context("Failed to load package database")?;

    for (name, repo_info) in repo_infos.iter() {
        println!(
            "{}: {} | {} | {:?}",
            name,
            repo_info.url,
            millis_to_datetime(repo_info.synced_at as u64),
            repo_info.binaries
        );
    }

    Ok(())
}

pub fn info(name: String) -> Result<()> {
    let repo_infos = get_packages().context("Failed to load package database")?;

    let repo_info = repo_infos
        .get(&name)
        .ok_or_else(|| anyhow!("{} not found", name))?;

    println!("Name: {}", name);
    println!("URL: {}", repo_info.url);
    println!(
        "Synced at: {}",
        millis_to_datetime(repo_info.synced_at as u64)
    );
    println!("Commit: {}", repo_info.commit);
    println!("Binaries: {:?}", repo_info.binaries);

    Ok(())
}

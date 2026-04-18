use crate::build::rebuild;
use anyhow::{Context, Result, anyhow};
use git2::Oid;
use justpkg::{
    Package, get_packages, hash_string, millis_to_datetime, normalize_url, resolve_package,
    resolve_remote_ref, save_repos,
};
use microxdg::Xdg;
use std::{
    env,
    path::PathBuf,
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

pub fn add(
    package: String,
    build_script: Option<PathBuf>,
    r: Option<String>,
    commit: Option<Oid>,
    binaries: Vec<PathBuf>,
) -> Result<()> {
    let mut repo_infos = get_packages().context("Failed to load package database")?;

    let normalized = normalize_url(&package).context("Failed to normalize URL")?;
    let hash = hash_string(&normalized);

    let build_script = match build_script {
        Some(path) => env::current_dir()?.join(&path),
        None => {
            let editor = env::var("EDITOR").unwrap_or_else(|_| "vi".to_string());
            let path = Xdg::new()?
                .config()?
                .join("justpkg/build-scripts")
                .join(format!("{}.sh", &hash));
            Command::new(editor).arg(&path).status()?;
            path
        }
    };

    let reference = match r {
        Some(r) => r,
        None => String::from("HEAD"),
    };
    let commit = match commit {
        Some(c) => c,
        None => resolve_remote_ref(&package, &reference).with_context(|| {
            format!("Failed to resolve remote ref {} for {}", reference, package)
        })?,
    }
    .to_string();

    let entry = Package {
        commit,
        reference,
        url: package,
        synced_at: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .context("Failed to get current time")?
            .as_millis(),
        binaries,
        build_script,
    };

    let changed = match repo_infos.get(&hash) {
        Some(old) => old != &entry,
        None => true,
    };

    if changed {
        repo_infos.insert(hash, entry);
        save_repos(&repo_infos).context("Failed to save package database")?;
    }

    rebuild().context("Failed to rebuild packages")?;

    Ok(())
}

pub fn update(packages: Vec<String>) -> Result<()> {
    let mut repo_infos = get_packages().context("Failed to load package database")?;
    let mut changed = false;

    let targets: Vec<String> = if packages.is_empty() {
        repo_infos.keys().cloned().collect()
    } else {
        packages
    };

    for input in targets {
        let hash = resolve_package(&input, &repo_infos)
            .with_context(|| format!("Failed to resolve package '{}'", input))?
            .ok_or_else(|| anyhow!("{} not found", input))?;

        let pkg = repo_infos
            .get_mut(&hash)
            .ok_or_else(|| anyhow!("{} not found", input))?;

        let latest = resolve_remote_ref(&pkg.url, &pkg.reference).with_context(|| {
            format!(
                "Failed to resolve remote ref '{}' for {}",
                pkg.reference, pkg.url
            )
        })?;

        let current = git2::Oid::from_str(&pkg.commit)
            .with_context(|| format!("Failed to parse commit hash '{}'", pkg.commit))?;

        if current != latest {
            pkg.commit = latest.to_string();
            pkg.synced_at = SystemTime::now()
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

pub fn remove(packages: Vec<String>) -> Result<()> {
    let mut repo_infos = get_packages().context("Failed to load package database")?;
    let mut changed = false;

    for input in packages {
        let hash = resolve_package(&input, &repo_infos)
            .with_context(|| format!("Failed to resolve package '{}'", input))?
            .ok_or_else(|| anyhow!("{} not found", input))?;

        if repo_infos.remove(&hash).is_some() {
            changed = true;
            println!("Removed: {}", input);
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

    for (hash, repo_info) in repo_infos.iter() {
        println!(
            "{} | {} | {} | {} | {:?}",
            hash,
            repo_info.url,
            repo_info.reference,
            millis_to_datetime(repo_info.synced_at as u64),
            repo_info.binaries
        );
    }

    Ok(())
}

pub fn info(package: String) -> Result<()> {
    let repo_infos = get_packages().context("Failed to load package database")?;

    let hash = resolve_package(&package, &repo_infos)
        .with_context(|| format!("Failed to resolve package '{}'", package))?
        .ok_or_else(|| anyhow!("{} not found", package))?;

    let repo_info = repo_infos
        .get(&hash)
        .ok_or_else(|| anyhow!("{} not found", package))?;

    println!("Hash: {}", hash);
    println!("Url: {}", repo_info.url);
    println!("Ref: {}", repo_info.reference);
    println!(
        "Synced at: {}",
        millis_to_datetime(repo_info.synced_at as u64)
    );
    println!("Commit: {}", repo_info.commit);
    println!("Binaries: {:?}", repo_info.binaries);

    Ok(())
}

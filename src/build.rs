use anyhow::{Context, Result, anyhow};
use justpkg::{Package, get_packages};
use microxdg::Xdg;
use sha2::Digest;
use std::{fs, os::unix::fs::PermissionsExt, path::Path, process::Command};
use topological_sort::TopologicalSort;

pub fn rebuild() -> Result<()> {
    let xdg = Xdg::new().context("Failed to find XDG directories")?;
    let config_path = xdg
        .config()
        .context("Failed to get XDG config directory")?
        .join("justpkg");
    let data_path = xdg
        .data()
        .context("Failed to get XDG data directory")?
        .join("justpkg");

    let user_config_path = config_path.join("repos.json");
    let internal_config_path = data_path.join("repos.json");

    let user_packages =
        get_packages(&user_config_path).context("Failed to load package database from config")?;

    let repos_path = data_path.join("repos");
    fs::create_dir_all(&repos_path)
        .with_context(|| format!("Failed to create repos directory: {}", repos_path.display()))?;

    fs::create_dir_all(&config_path).with_context(|| {
        format!(
            "Failed to create config directory: {}",
            config_path.display()
        )
    })?;

    let mut ts = TopologicalSort::<&String>::new();
    for (name, package) in &user_packages {
        ts.insert(name);
        for dependency in &package.dependencies {
            if !user_packages.contains_key(dependency) {
                return Err(anyhow!(
                    "Package {} depends on unknown package {}",
                    name,
                    dependency
                ));
            }
            ts.add_dependency(dependency, name);
        }
    }

    let mut sorted_packages = Vec::new();
    while let Some(name) = ts.pop() {
        sorted_packages.push(name);
    }

    if !ts.is_empty() {
        return Err(anyhow!("Circular dependency detected in packages"));
    }

    // Install
    for name in sorted_packages {
        let package = &user_packages[name];
        if !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(anyhow!("{name} is not a valid package name"));
        }
        let repo_path = repos_path.join(name);
        let exists = repo_path.exists();
        let original_head = if exists {
            git2::Repository::open(&repo_path)
                .ok()
                .and_then(|r| r.head().ok()?.target())
        } else {
            None
        };

        match install_package(name, package, &repos_path, &config_path) {
            Err(e) => {
                eprintln!("{} install failed: {e}", package.url);
                if exists
                    && let Some(head) = original_head
                    && let Ok(repo) = git2::Repository::open(&repo_path)
                {
                    let _ = repo.set_head_detached(head);
                    let _ = repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()));
                    let _ = Command::new("git")
                        .args(["clean", "-fd"])
                        .current_dir(&repo_path)
                        .status();
                } else if repo_path.exists() {
                    let _ = fs::remove_dir_all(&repo_path);
                }
            }
            Ok(()) => {
                println!("{} install succeeded", &name)
            }
        }
    }

    let internal_packages =
        get_packages(&internal_config_path).context("Failed to load package database from data")?;

    let removed_packages = internal_packages
        .keys()
        .filter(|k| !user_packages.contains_key(*k));

    for name in removed_packages {
        let package = &internal_packages[name];
        let repo_path = repos_path.join(name);
        let exists = repo_path.exists();
        let original_head = if exists {
            git2::Repository::open(&repo_path)
                .ok()
                .and_then(|r| r.head().ok()?.target())
        } else {
            None
        };

        match uninstall_package(name, package, &repos_path) {
            Err(e) => {
                eprintln!("{} uninstall failed: {e}", package.url);
                if exists
                    && let Some(head) = original_head
                    && let Ok(repo) = git2::Repository::open(&repo_path)
                {
                    let _ = repo.set_head_detached(head);
                    let _ = repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()));
                    let _ = Command::new("git")
                        .args(["clean", "-fd"])
                        .current_dir(&repo_path)
                        .status();
                }
            }
            Ok(()) => {
                println!("{} uninstall succeeded", &name)
            }
        }
    }
    fs::copy(&user_config_path, &internal_config_path)?;

    Ok(())
}

fn install_package(
    name: &str,
    package: &Package,
    repos_path: &Path,
    config_path: &Path,
) -> Result<()> {
    if name.contains("..") || name.contains('/') {
        return Err(anyhow!("Invalid package name: {name}"));
    }

    let repo_path = repos_path.join(name);

    let repo = match git2::Repository::open(&repo_path) {
        Ok(r) => r,
        Err(_) => git2::Repository::clone(&package.url, &repo_path)
            .with_context(|| format!("Failed to clone repository: {}", package.url))?,
    };

    let target = git2::Oid::from_str(&package.commit)
        .with_context(|| format!("Failed to parse commit hash '{}'", package.commit))?;

    let needs_update = repo.head()?.peel_to_commit()?.id() != target
        || package.install_hash
            != hex::encode(sha2::Sha256::digest(std::fs::read(
                &package.install_script,
            )?))
        || package.uninstall_hash
            != hex::encode(sha2::Sha256::digest(std::fs::read(
                &package.uninstall_script,
            )?));

    if needs_update {
        println!("Building {}", package.url);
        let mut remote = repo
            .find_remote("origin")
            .context("Failed to find origin remote")?;

        let mut fetch_opts = git2::FetchOptions::new();

        remote
            .fetch(
                &[
                    "refs/heads/*:refs/remotes/origin/*",
                    "refs/tags/*:refs/tags/*",
                ],
                Some(&mut fetch_opts),
                None,
            )
            .with_context(|| format!("Failed to fetch from origin for {}", package.url))?;

        repo.set_head_detached(target)
            .with_context(|| format!("Failed to set HEAD to commit {}", package.commit))?;
        repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))
            .with_context(|| format!("Failed to checkout commit {}", package.commit))?;

        let install_script = config_path.join(&package.install_script);

        let mut perms = fs::metadata(&install_script)?.permissions();
        let mode = perms.mode();
        perms.set_mode(mode | 0o111);
        fs::set_permissions(&install_script, perms)?;

        let status = Command::new(&install_script)
            .current_dir(&repo_path)
            .status()
            .with_context(|| {
                format!(
                    "Failed to execute install script: {}",
                    install_script.display()
                )
            })?;

        if !status.success() {
            let error_msg = match status.code() {
                Some(code) => format!("install failed for {} with exit code {}", name, code),
                None => format!("install process terminated unexpectedly for {}", name),
            };
            return Err(anyhow!(error_msg));
        }
    }

    Ok(())
}

fn uninstall_package(name: &str, package: &Package, repos_path: &Path) -> Result<()> {
    let repo_path = repos_path.join(name);

    let status = Command::new(&package.uninstall_script)
        .current_dir(&repo_path)
        .status()
        .with_context(|| {
            format!(
                "Failed to execute uninstall script: {}",
                package.uninstall_script.display()
            )
        })?;

    if !status.success() {
        let error_msg = match status.code() {
            Some(code) => format!("uninstall failed for {} with exit code {}", name, code),
            None => format!("uninstall process terminated unexpectedly for {}", name),
        };
        return Err(anyhow!(error_msg));
    }
    fs::remove_dir_all(repos_path.join(name))?;
    Ok(())
}

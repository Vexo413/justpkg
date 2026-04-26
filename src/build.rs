use anyhow::{Context, Result, anyhow};
use justpkg::{Package, get_packages};
use microxdg::Xdg;
use std::{
    collections::HashSet,
    fs,
    os::unix::fs::PermissionsExt,
    path::Path,
    process::{self, Command},
};

pub fn rebuild() -> Result<()> {
    // Setup
    let packages = get_packages().context("Failed to load package database")?;
    let xdg = Xdg::new().context("Failed to find XDG directories")?;

    let data_path = xdg
        .data()
        .context("Failed to get XDG data directory")?
        .join("justpkg");

    let repos_path = data_path.join("repos");
    fs::create_dir_all(&repos_path)
        .with_context(|| format!("Failed to create repos directory: {}", repos_path.display()))?;

    let bin_path = data_path.join("bin");
    fs::create_dir_all(&bin_path)
        .with_context(|| format!("Failed to create bin directory: {}", bin_path.display()))?;

    let config_path = xdg
        .config()
        .context("Failed to get XDG config directory")?
        .join("justpkg");
    fs::create_dir_all(&config_path).with_context(|| {
        format!(
            "Failed to create config directory: {}",
            config_path.display()
        )
    })?;

    // Install
    for (name, package) in packages.iter() {
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

        match build_package(&name, &package, &repos_path, &bin_path, &config_path) {
            Err(e) => {
                eprintln!("{} build failed: {e}", package.url);
                if exists {
                    if let Some(head) = original_head {
                        if let Ok(repo) = git2::Repository::open(&repo_path) {
                            let _ = repo.set_head_detached(head);
                            let _ = repo
                                .checkout_head(Some(git2::build::CheckoutBuilder::new().force()));
                            let _ = Command::new("git")
                                .args(["clean", "-fd"])
                                .current_dir(&repo_path)
                                .status();
                        }
                    }
                } else if repo_path.exists() {
                    let _ = fs::remove_dir_all(&repo_path);
                }
            }
            Ok(()) => {
                println!("{} build succeeded", package.url)
            }
        }
    }
    println!("Cleaning...");
    let valid_repos: HashSet<&str> = packages.keys().map(|s| s.as_str()).collect();
    for entry in fs::read_dir(&repos_path)
        .with_context(|| format!("Failed to read data directory: {}", repos_path.display()))?
    {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() {
            if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                if !valid_repos.contains(name) {
                    fs::remove_dir_all(&path).with_context(|| {
                        format!("Failed to remove stale repo: {}", path.display())
                    })?;
                }
            }
        }
    }

    let valid_binaries: HashSet<&str> = packages
        .values()
        .flat_map(|pkg| pkg.binaries.iter())
        .filter_map(|p| p.file_name()?.to_str())
        .collect();
    for entry in fs::read_dir(&bin_path)
        .with_context(|| format!("Failed to read bin directory: {}", bin_path.display()))?
    {
        let entry = entry?;
        let path = entry.path();

        if path.is_file() {
            if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                if !valid_binaries.contains(name) {
                    fs::remove_file(&path).with_context(|| {
                        format!("Failed to remove stale binary symlink: {}", path.display())
                    })?;
                }
            }
        }
    }

    Ok(())
}

fn build_package(
    name: &str,
    package: &Package,
    repos_path: &Path,
    bin_path: &Path,
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

    // TODO add build script change detection
    let needs_update = repo.head()?.peel_to_commit()?.id() != target
        || !package.binaries.iter().all(|b| repo_path.join(b).exists());

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

        let build_script = config_path.join(&package.build_script);

        let mut perms = fs::metadata(&build_script)?.permissions();
        let mode = perms.mode();
        perms.set_mode(mode | 0o111);
        fs::set_permissions(&build_script, perms)?;

        let status = Command::new(&build_script)
            .current_dir(&repo_path)
            .status()
            .with_context(|| {
                format!("Failed to execute build script: {}", build_script.display())
            })?;

        if !status.success() {
            let error_msg = match status.code() {
                Some(code) => format!("build failed for {} with exit code {}", name, code),
                None => format!("build process terminated unexpectedly for {}", name),
            };
            return Err(anyhow!(error_msg));
        }
    }

    println!("Linking {}", package.url);
    for binary in package.binaries.iter() {
        let symlink_path = bin_path.join(
            binary
                .file_name()
                .ok_or_else(|| anyhow!("Binary path has no file name: {}", binary.display()))?
                .to_string_lossy()
                .as_ref(),
        );
        let binary_path = repo_path.join(binary);
        let _ = fs::remove_file(&symlink_path);
        std::os::unix::fs::symlink(&binary_path, &symlink_path).with_context(|| {
            format!(
                "Failed to symlink binary '{}' from '{}'",
                binary.display(),
                binary_path.display()
            )
        })?;
    }
    Ok(())
}

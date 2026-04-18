use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Utc};
use git2::Oid;
use microxdg::Xdg;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File},
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct Package {
    pub url: String,
    pub r: String,
    pub commit: String,
    pub synced_at: u128,
    pub build_script: PathBuf,
    pub binaries: Vec<PathBuf>,
}

pub fn add(
    package: String,
    build_script: PathBuf,
    r: Option<String>,
    commit: Option<Oid>,
    binaries: Vec<PathBuf>,
) -> Result<()> {
    let mut repo_infos = get_packages().context("Failed to load package database")?;

    let normalized = normalize_url(&package).context("Failed to normalize URL")?;
    let hash = hash_string(&normalized);
    let r = match r {
        Some(r) => r,
        None => String::from("HEAD"),
    };
    let commit = match commit {
        Some(c) => c,
        None => resolve_remote_ref(&package, "HEAD")
            .with_context(|| format!("Failed to resolve remote ref 'HEAD' for {}", package))?,
    }
    .to_string();

    let entry = Package {
        commit,
        r,
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

pub fn resolve_remote_ref(url: &str, r: &str) -> Result<git2::Oid> {
    let repo = git2::Repository::init_bare(std::path::Path::new("/tmp/git2-lookup"))
        .context("Failed to create temporary git repository")?;
    let mut remote = repo
        .remote_anonymous(url)
        .with_context(|| format!("Failed to create remote for {}", url))?;

    remote
        .connect(git2::Direction::Fetch)
        .with_context(|| format!("Failed to connect to remote {}", url))?;

    let refs = remote.list().context("Failed to list remote refs")?;

    if r == "HEAD" {
        for head in refs {
            if head.name() == "HEAD" {
                let name = head
                    .symref_target()
                    .ok_or_else(|| anyhow!("HEAD is not symbolic"))?;

                fs::remove_dir_all("/tmp/git2-lookup")
                    .context("Failed to clean up temp directory")?;
                return resolve_remote_ref(url, name.trim_start_matches("refs/heads/"));
            }
        }
    }

    for head in refs {
        let name = head.name();
        if name.ends_with(r) {
            fs::remove_dir_all("/tmp/git2-lookup").context("Failed to clean up temp directory")?;
            return Ok(head.oid());
        }
    }

    fs::remove_dir_all("/tmp/git2-lookup").context("Failed to clean up temp directory")?;

    Err(anyhow!("ref not found: {}", r))
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

        let latest = resolve_remote_ref(&pkg.url, &pkg.r)
            .with_context(|| format!("Failed to resolve remote ref '{}' for {}", pkg.r, pkg.url))?;

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

fn normalize_url(url: &str) -> Result<String> {
    let mut normalized = url.to_string();

    if let Some(pos) = normalized.find("://") {
        normalized = normalized[(pos + 3)..].to_string();
    }

    if normalized.starts_with("git@") {
        normalized = normalized[4..].to_string();
    }

    normalized = normalized.replace(':', "/");

    if normalized.ends_with(".git") {
        normalized = normalized[..normalized.len() - 4].to_string();
    }

    while normalized.contains("//") {
        normalized = normalized.replace("//", "/");
    }

    Ok(normalized)
}

fn hash_string(s: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(s.as_bytes());
    hex::encode(hasher.finalize())
}

fn get_packages() -> Result<HashMap<String, Package>> {
    let config_path = Xdg::new()
        .context("Failed to initialize XDG directories")?
        .config()
        .context("Failed to get XDG config directory")?
        .join("justpkg");
    let path = config_path.join("repos.json");

    if path.exists() {
        let file = File::open(&path)
            .with_context(|| format!("Failed to open config file: {}", path.display()))?;
        Ok(serde_json::from_reader(file)
            .with_context(|| format!("Failed to parse config file: {}", path.display()))?)
    } else {
        Ok(HashMap::new())
    }
}

fn save_repos(repo_infos: &HashMap<String, Package>) -> Result<()> {
    let config_path = Xdg::new()
        .context("Failed to initialize XDG directories")?
        .config()
        .context("Failed to get XDG config directory")?
        .join("justpkg");
    let path = config_path.join("repos.json");
    let json =
        serde_json::to_string_pretty(repo_infos).context("Failed to serialize package database")?;
    fs::write(&path, json)
        .with_context(|| format!("Failed to write config file: {}", path.display()))?;
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

fn millis_to_datetime(ms: u64) -> DateTime<Utc> {
    let system_time = UNIX_EPOCH + Duration::from_millis(ms);
    system_time.into()
}

pub fn list() -> Result<()> {
    let repo_infos = get_packages().context("Failed to load package database")?;

    for (hash, repo_info) in repo_infos.iter() {
        println!(
            "{} | {} | {} | {} | {:?}",
            hash,
            repo_info.url,
            repo_info.r,
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
    println!("Ref: {}", repo_info.r);
    println!(
        "Synced at: {}",
        millis_to_datetime(repo_info.synced_at as u64)
    );
    println!("Commit: {}", repo_info.commit);
    println!("Binaries: {:?}", repo_info.binaries);

    Ok(())
}

fn resolve_package(input: &str, repo_infos: &HashMap<String, Package>) -> Result<Option<String>> {
    if input.contains("://") || input.contains("git@") || input.contains("github.com") {
        let normalized = normalize_url(input)?;
        let hash = hash_string(&normalized);
        if repo_infos.contains_key(&hash) {
            return Ok(Some(hash));
        }
    }

    for (hash, info) in repo_infos {
        if info.binaries.iter().any(|b| match b.file_name() {
            Some(name) => name.to_string_lossy().as_ref() == input,
            None => false,
        }) {
            return Ok(Some(hash.clone()));
        }
    }

    let mut matches = Vec::new();
    for hash in repo_infos.keys() {
        if hash.starts_with(input) {
            matches.push(hash.clone());
        }
    }

    if matches.len() == 1 {
        return Ok(Some(matches[0].clone()));
    } else if matches.len() > 1 {
        return Err(anyhow!("Ambiguous package identifier: {}", input));
    }

    Ok(None)
}

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
    for (hash, package) in packages.iter() {
        let repo_path = repos_path.join(hash);
        let exists = repo_path.exists();
        let original_head = if exists {
            git2::Repository::open(&repo_path)
                .ok()
                .and_then(|r| r.head().ok()?.target())
        } else {
            None
        };

        match build_package(&package, &hash, &repos_path, &bin_path, &config_path) {
            Err(e) => {
                eprintln!("{} build failed: {e}", package.url);
                if exists {
                    if let Some(head) = original_head {
                        if let Ok(repo) = git2::Repository::open(&repo_path) {
                            let _ = repo.set_head_detached(head);
                            let _ = repo.checkout_head(Some(
                                git2::build::CheckoutBuilder::new().force(),
                            ));
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
    package: &Package,
    hash: &str,
    repos_path: &Path,
    bin_path: &Path,
    config_path: &Path,
) -> Result<()> {
    if hash.contains("..") || hash.contains('/') {
        return Err(anyhow!("Invalid package hash: {hash}"));
    }

    let repo_path = repos_path.join(hash);

    let repo = match git2::Repository::open(&repo_path) {
        Ok(r) => r,
        Err(_) => git2::Repository::clone(&package.url, &repo_path)
            .with_context(|| format!("Failed to clone repository: {}", package.url))?,
    };

    let target = git2::Oid::from_str(&package.commit)
        .with_context(|| format!("Failed to parse commit hash '{}'", package.commit))?;

    let needs_update = repo
        .resolve_reference_from_short_name(&package.r)
        .ok()
        .and_then(|h| h.target())
        != Some(target)
        || !package.binaries.iter().all(|b| b.exists());

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

        let status = Command::new("sh")
            .arg(&build_script)
            .current_dir(&repo_path)
            .status()
            .with_context(|| format!("Failed to execute build script: {}", build_script.display()))?;

        if !status.success() {
            let error_msg = match status.code() {
                Some(code) => format!("build failed for {} with exit code {}", hash, code),
                None => format!("build process terminated unexpectedly for {}", hash),
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

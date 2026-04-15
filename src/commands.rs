use crate::build::{build_repo, install_binaries, rebuild};
use anyhow::{Result, anyhow};
use chrono::{DateTime, Utc};
use git2::{FetchOptions, Oid, RemoteCallbacks, build::RepoBuilder};
use microxdg::Xdg;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File},
    path::{Path, PathBuf},
    process::{Command, ExitStatus},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct Package {
    pub url: String,
    pub commit: String,
    pub fetched_at: u128,
    pub build_script: PathBuf,
    pub binaries: Vec<PathBuf>,
}

pub fn add(package: String, build_script: PathBuf, binaries: Vec<PathBuf>) -> Result<()> {
    let xdg = Xdg::new()?;
    let justpkg_data = xdg.data()?.join("justpkg");
    std::fs::create_dir_all(justpkg_data)?;

    let mut repo_infos = get_packages()?;

    let normalized = normalize_url(&package)?;
    let hash = hash_string(&normalized);

    let entry = Package {
        commit: resolve_remote_ref(&package, "HEAD")?.to_string(),
        url: package,
        fetched_at: SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis(),
        binaries,
        build_script,
    };

    let changed = match repo_infos.get(&hash) {
        Some(old) => old != &entry,
        None => true,
    };

    if changed {
        repo_infos.insert(hash, entry);
        save_repos(&repo_infos)?;
    }

    build()?;

    Ok(())
}

pub fn resolve_remote_ref(url: &str, r: &str) -> Result<Oid> {
    let repo = git2::Repository::init_bare(std::path::Path::new("/tmp/git2-temp"))?;

    let mut remote = repo.remote_anonymous(url)?;

    let mut fetch_opts = FetchOptions::new();
    let callbacks = RemoteCallbacks::new();

    fetch_opts.remote_callbacks(callbacks);

    remote.fetch(
        &[
            "refs/heads/*:refs/remotes/origin/*",
            "refs/tags/*:refs/tags/*",
        ],
        Some(&mut fetch_opts),
        None,
    )?;

    if r == "HEAD" {
        let head = repo.find_reference("refs/remotes/origin/HEAD")?;
        return Ok(head
            .resolve()?
            .target()
            .ok_or_else(|| anyhow!("invalid HEAD"))?);
    }

    let branch = format!("refs/remotes/origin/{}", r);
    if let Ok(reference) = repo.find_reference(&branch) {
        return Ok(reference.target().ok_or_else(|| anyhow!("no target"))?);
    }

    let tag = format!("refs/tags/{}", r);
    if let Ok(reference) = repo.find_reference(&tag) {
        return Ok(reference.target().ok_or_else(|| anyhow!("no tag target"))?);
    }

    Err(anyhow!("ref not found: {}", r))
}

fn update_package(
    base: &Path,
    repo_infos: &mut HashMap<String, Package>,
    package: &str,
) -> Result<()> {
    let hash =
        resolve_package(package, repo_infos)?.ok_or_else(|| anyhow!("{} not found", package))?;

    let repo_info = repo_infos
        .get_mut(&hash)
        .ok_or_else(|| anyhow!("{} not found", package))?;

    rebuild(base, &hash, repo_info)?;
    Ok(())
}

pub fn update(packages: Vec<String>, base: PathBuf) -> Result<()> {
    std::fs::create_dir_all(&base)?;
    let mut repo_infos = get_packages(&base)?;
    let mut changed = false;

    if packages.is_empty() {
        for (hash, repo_info) in repo_infos.iter_mut() {
            match rebuild(&base, hash, repo_info) {
                Ok(c) => {
                    if c {
                        changed = true;
                    }
                }
                Err(e) => {
                    eprintln!("Update failed: {e}");
                }
            }
        }
    } else {
        for package in packages {
            match update_package(&base, &mut repo_infos, &package) {
                Ok(()) => changed = true,
                Err(e) => {
                    eprintln!("Update failed: {e}");
                }
            }
        }
    }

    if changed {
        save_repos(&base, &repo_infos)?;
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
    let config_path = Xdg::new()?.config()?;
    let path = config_path.join("repos.json");

    if path.exists() {
        let file = File::open(&path)?;
        Ok(serde_json::from_reader(file)?)
    } else {
        Ok(HashMap::new())
    }
}

fn save_repos(repo_infos: &HashMap<String, Package>) -> Result<()> {
    let config_path = Xdg::new()?.config()?;
    let path = config_path.join("repos.json");
    let json = serde_json::to_string_pretty(repo_infos)?;
    fs::write(path, json)?;
    Ok(())
}

fn remove_package(
    base: &Path,
    repo_infos: &mut HashMap<String, Package>,
    package: &str,
) -> Result<()> {
    let hash =
        resolve_package(package, repo_infos)?.ok_or_else(|| anyhow!("{} not found", package))?;

    let repo_info = repo_infos
        .remove(&hash)
        .ok_or_else(|| anyhow!("{} not found", package))?;

    let repo_path = base.join(&hash);
    if repo_path.exists() {
        std::fs::remove_dir_all(repo_path)?;
    }

    let xdg = microxdg::Xdg::new()?;
    for binary in repo_info.binaries {
        let symlink_path = xdg.bin()?.join(
            binary
                .file_name()
                .ok_or(anyhow!("Binary is not a file"))?
                .to_string_lossy()
                .as_ref(),
        );
        if symlink_path.exists() {
            std::fs::remove_file(symlink_path)?;
        }
    }

    println!("Deleted: {}", package);
    Ok(())
}

pub fn remove(packages: Vec<String>, base: PathBuf) -> Result<()> {
    std::fs::create_dir_all(&base)?;
    let mut repo_infos = get_packages(&base)?;
    let mut changed = false;

    for package in packages {
        match remove_package(&base, &mut repo_infos, &package) {
            Ok(()) => changed = true,
            Err(e) => {
                eprintln!("Remove failed: {e}");
            }
        }
    }

    if changed {
        save_repos(&base, &repo_infos)?;
    }
    println!("Finished");
    Ok(())
}

fn millis_to_datetime(ms: u64) -> DateTime<Utc> {
    let system_time = UNIX_EPOCH + Duration::from_millis(ms);
    system_time.into()
}

pub fn list(base: PathBuf) -> Result<()> {
    std::fs::create_dir_all(&base)?;
    let repo_infos = get_packages(&base)?;
    for (hash, repo_info) in repo_infos.iter() {
        println!(
            "{} | {} | {} | {:?}",
            hash,
            repo_info.url,
            millis_to_datetime(repo_info.fetched_at as u64),
            repo_info.binaries
        );
    }
    Ok(())
}

fn info_package(repo_infos: &HashMap<String, Package>, package: &str) -> Result<()> {
    let hash =
        resolve_package(package, repo_infos)?.ok_or_else(|| anyhow!("{} not found", package))?;

    let repo_info = repo_infos
        .get(&hash)
        .ok_or_else(|| anyhow!("{} not found", package))?;

    println!("{}", hash);
    println!("Url: {}", repo_info.url);
    println!(
        "Fetched at: {}",
        millis_to_datetime(repo_info.fetched_at as u64),
    );
    println!("Binaries: {:?}", repo_info.binaries);
    Ok(())
}

pub fn info(package: String, base: PathBuf) -> Result<()> {
    std::fs::create_dir_all(&base)?;
    let repo_infos = get_packages(&base)?;
    info_package(&repo_infos, &package)?;
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

pub fn build() -> Result<()> {
    let packages = get_packages()?;
    let xdg = Xdg::new()?;

    let justpkg_data = xdg.data()?.join("justpkg");
    let justpkg_config = xdg.config()?.join("justpkg");
    let justpkg_bin = xdg.bin()?.join("justpkg");

    for (hash, package) in packages.iter() {
        if hash.contains("..") || hash.contains('/') {
            return Err(anyhow!("invalid package hash: {hash}"));
        }

        let repo_path = justpkg_data.join(hash);

        let repo = match git2::Repository::open(&repo_path) {
            Ok(r) => r,
            Err(_) => git2::Repository::clone(&package.url, &repo_path)?,
        };

        let target = git2::Oid::from_str(&package.commit)?;

        let needs_update = repo.head().ok().and_then(|h| h.target()) != Some(target);

        if needs_update {
            let mut remote = repo.find_remote("origin")?;

            let mut fetch_opts = git2::FetchOptions::new();

            remote.fetch(
                &["refs/heads/*:refs/remotes/origin/*"],
                Some(&mut fetch_opts),
                None,
            )?;

            repo.set_head_detached(target)?;
            repo.checkout_head(Some(git2::build::CheckoutBuilder::new().force()))?;
        }

        let build_script = justpkg_config.join(&package.build_script);

        let status = Command::new("sh")
            .arg(&build_script)
            .current_dir(&repo_path)
            .status()?
            .code();

        match status {
            Some(0) => {}
            Some(code) => {
                return Err(anyhow!("build failed for {} with exit code {}", hash, code));
            }
            None => {
                return Err(anyhow!(
                    "build process terminated unexpectedly for {}",
                    hash
                ));
            }
        }

        for binary in package.binaries.iter() {
            let symlink_path = justpkg_bin.join(
                binary
                    .file_name()
                    .ok_or(anyhow!("Binary is not a file"))?
                    .to_string_lossy()
                    .as_ref(),
            );
            let binary_path = repo_path.join(binary);
            let _ = fs::remove_file(&symlink_path);
            std::os::unix::fs::symlink(binary_path, symlink_path)?;
        }
    }

    let valid_repos: HashSet<&str> = packages.keys().map(|s| s.as_str()).collect();
    for entry in fs::read_dir(&justpkg_data)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() {
            if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                if !valid_repos.contains(name) {
                    fs::remove_dir_all(&path)?;
                }
            }
        }
    }

    let valid_binaries: HashSet<&str> = packages
        .values()
        .flat_map(|pkg| pkg.binaries.iter())
        .filter_map(|p| p.file_name()?.to_str())
        .collect();
    for entry in fs::read_dir(&justpkg_bin)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_file() {
            if let Some(name) = path.file_name().and_then(|s| s.to_str()) {
                if !valid_binaries.contains(name) {
                    fs::remove_file(&path)?;
                }
            }
        }
    }

    Ok(())
}

use crate::build::{build_repo, install_binaries, rebuild};
use anyhow::{Result, anyhow};
use chrono::{DateTime, Utc};
use git2::{FetchOptions, build::RepoBuilder};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs::{self, File},
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[derive(Serialize, Deserialize, Clone)]
pub struct RepoInfo {
    pub url: String,
    pub last_commit: Option<String>,
    pub fetched_at: u128,
    pub binaries: Vec<PathBuf>,
    pub command: String,
}

pub fn add(package: String, base: &Path, command: String, binaries: Vec<PathBuf>) -> Result<()> {
    std::fs::create_dir_all(base)?;
    let mut repo_infos = get_repos(base)?;
    let mut changed = false;

    let normalized = normalize_url(&package)?;
    let hash = hash_string(&normalized);
    let repo_path = base.join(&hash);

    if repo_path.exists() {
        if let Some(repo_info) = repo_infos.get_mut(&hash) {
            match rebuild(base, &hash, repo_info) {
                Ok(c) => {
                    if c {
                        changed = true;
                    }
                }
                Err(e) => {
                    return Err(e);
                }
            }
        }
    } else {
        let mut fetch_options = FetchOptions::new();
        fetch_options.depth(1);
        let repo = RepoBuilder::new()
            .fetch_options(fetch_options)
            .clone(&package, &repo_path)?;

        build_repo(&repo_path, &command)?;
        install_binaries(&repo_path, &binaries)?;
        let last_commit = repo.head()?.target().map(|oid| oid.to_string());
        let repo_info = RepoInfo {
            url: package.to_string(),
            last_commit,
            fetched_at: SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis(),
            binaries,
            command,
        };

        repo_infos.insert(hash, repo_info);
        changed = true;
        println!("Added: {}", package);
    }

    if changed {
        save_repos(base, &repo_infos)?;
    }
    println!("Finished");
    Ok(())
}

fn update_package(
    base: &Path,
    repo_infos: &mut HashMap<String, RepoInfo>,
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
    let mut repo_infos = get_repos(&base)?;
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

fn get_repos(base: &Path) -> Result<HashMap<String, RepoInfo>> {
    let path = base.join("repos.json");

    if path.exists() {
        let file = File::open(&path)?;
        Ok(serde_json::from_reader(file)?)
    } else {
        Ok(HashMap::new())
    }
}

fn save_repos(base: &Path, repo_infos: &HashMap<String, RepoInfo>) -> Result<()> {
    let path = base.join("repos.json");
    let json = serde_json::to_string_pretty(repo_infos)?;
    fs::write(path, json)?;
    Ok(())
}

fn remove_package(
    base: &Path,
    repo_infos: &mut HashMap<String, RepoInfo>,
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
    let mut repo_infos = get_repos(&base)?;
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
    let repo_infos = get_repos(&base)?;
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

fn info_package(repo_infos: &HashMap<String, RepoInfo>, package: &str) -> Result<()> {
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
    let repo_infos = get_repos(&base)?;
    info_package(&repo_infos, &package)?;
    Ok(())
}

fn resolve_package(input: &str, repo_infos: &HashMap<String, RepoInfo>) -> Result<Option<String>> {
    if input.contains("://") || input.contains("git@") || input.contains("github.com") {
        let normalized = normalize_url(input)?;
        let hash = hash_string(&normalized);
        if repo_infos.contains_key(&hash) {
            return Ok(Some(hash));
        }
    }

    for (hash, info) in repo_infos {
        if info.binaries.iter().any(|b| b == input) {
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

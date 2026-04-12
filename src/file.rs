use std::{
    collections::HashMap,
    fs::{self, File},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Result, anyhow};
use chrono::{DateTime, Utc};
use git2::{FetchOptions, ObjectType, ResetType, build::RepoBuilder};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Serialize, Deserialize, Clone)]
pub struct RepoInfo {
    pub url: String,
    pub last_commit: Option<String>,
    pub fetched_at: u128,
    pub binaries: Vec<String>,
}

pub fn add(packages: &Vec<String>, base: PathBuf) -> Result<()> {
    std::fs::create_dir_all(&base)?;
    let mut repo_infos = get_repos(&base)?;
    let mut changed = false;
    for package in packages {
        let normalized = normalize_url(&package)?;
        let hash = hash_string(&normalized);
        let repo_path = base.join(&hash);

        let repo = if repo_path.exists() {
            update(&vec![package.to_string()], base.clone())?;
            continue;
        } else {
            let mut fetch_options = FetchOptions::new();
            fetch_options.depth(1);
            RepoBuilder::new()
                .fetch_options(fetch_options)
                .clone(&package, &repo_path)?
        };

        let binaries = build_repo(&repo_path)?;

        let last_commit = repo.head()?.target().map(|oid| oid.to_string());
        let repo_info = RepoInfo {
            url: package.to_string(),
            last_commit,
            fetched_at: SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis(),
            binaries,
        };

        repo_infos.insert(hash, repo_info);
        changed = true;
        println!("Added: {}", package);
    }
    if changed {
        save_repos(&base, &repo_infos)?;
    }
    println!("Finished");
    Ok(())
}

pub fn build_repo(repo_path: &Path) -> Result<Vec<String>> {
    let justfile = repo_path.join("justfile");
    let makefile = repo_path.join("Makefile");
    let cargo_toml = repo_path.join("Cargo.toml");
    let cmake = repo_path.join("CMakeLists.txt");

    let binaries = if justfile.exists() {
        build_just(repo_path)?
    } else if makefile.exists() {
        build_make(repo_path)?
    } else if cargo_toml.exists() {
        build_cargo(repo_path)?
    } else if cmake.exists() {
        build_cmake(repo_path)?
    } else {
        return Err(anyhow!("No supported build system found"));
    };

    let bin_dir = microxdg::Xdg::new()?.bin()?;
    fs::create_dir_all(&bin_dir)?;

    for binary in binaries.iter() {
        if let Some(name) = binary.file_name() {
            let dest = bin_dir.join(name);
            let _ = fs::remove_file(&dest);
            std::os::unix::fs::symlink(&binary, &dest)?;
        }
    }
    Ok(binaries
        .iter()
        .map(|path| {
            let v = path.file_name().ok_or(anyhow!("Not a file"))?;
            let v2 = v.to_string_lossy().to_string();
            Ok(v2)
        })
        .collect::<Result<Vec<String>>>()?)
}

pub fn update(packages: &Vec<String>, base: PathBuf) -> Result<()> {
    let mut changed = false;
    std::fs::create_dir_all(&base)?;
    let mut repo_infos = get_repos(&base)?;
    let xdg = microxdg::Xdg::new()?;

    if packages.is_empty() {
        for (hash, repo_info) in repo_infos.iter_mut() {
            let repo = git2::Repository::open(base.join(hash))?;
            {
                let mut remote = repo.find_remote("origin")?;
                remote.fetch(&[] as &[&str], None, None)?;
            }

            let oid = repo.refname_to_id("refs/remotes/origin/HEAD")?;
            let remote_obj = repo.find_object(oid, Some(ObjectType::Commit))?;

            repo.reset(&remote_obj, ResetType::Hard, None)?;

            let head_oid = repo.head()?.target().map(|v| v.to_string());
            if repo_info.last_commit != head_oid {
                for binary in repo_info.binaries.iter() {
                    let bin_path = xdg.bin()?.join(binary);
                    if bin_path.exists() {
                        std::fs::remove_file(bin_path)?;
                    }
                }
                let binaries = build_repo(&base.join(hash))?;
                repo_info.binaries = binaries;
                repo_info.last_commit = head_oid;
                repo_info.fetched_at = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
                changed = true;
                println!("Updated: {}", repo_info.url);
            } else {
                println!("{} is already up-to-date", repo_info.url);
            }
        }
        if changed {
            save_repos(&base, &repo_infos)?;
        }
        println!("Finished");
    } else {
        for package in packages {
            let hash = if let Some(h) = resolve_package(package, &repo_infos)? {
                h
            } else {
                println!("{} not found", package);
                continue;
            };

            if let Some(repo_info) = repo_infos.get_mut(&hash) {
                let repo = git2::Repository::open(base.join(&hash))?;
                {
                    let mut remote = repo.find_remote("origin")?;
                    remote.fetch(&[] as &[&str], None, None)?;
                }

                let oid = repo.refname_to_id("refs/remotes/origin/HEAD")?;
                let remote_obj = repo.find_object(oid, Some(ObjectType::Commit))?;

                repo.reset(&remote_obj, ResetType::Hard, None)?;

                let head_oid = repo.head()?.target().map(|v| v.to_string());
                if repo_info.last_commit != head_oid {
                    for binary in repo_info.binaries.iter() {
                        let bin_path = xdg.bin()?.join(binary);
                        if bin_path.exists() {
                            std::fs::remove_file(bin_path)?;
                        }
                    }
                    let binaries = build_repo(&base.join(hash))?;
                    repo_info.binaries = binaries;
                    repo_info.last_commit = head_oid;
                    changed = true;
                    println!("Updated: {}", repo_info.url);
                } else {
                    println!("{} is already up-to-date", repo_info.url);
                }
            }
        }
        if changed {
            save_repos(&base, &repo_infos)?;
        }
        println!("Finished");
    }
    Ok(())
}

fn build_just(repo_path: &Path) -> Result<Vec<PathBuf>> {
    Command::new("just")
        .arg("build")
        .current_dir(repo_path)
        .status()?;
    find_binaries_in_dir(repo_path)
}

fn build_make(repo_path: &Path) -> Result<Vec<PathBuf>> {
    Command::new("make")
        .arg("build")
        .current_dir(repo_path)
        .status()?;
    find_binaries_in_dir(repo_path)
}

fn build_cargo(repo_path: &Path) -> Result<Vec<PathBuf>> {
    Command::new("cargo")
        .arg("build")
        .arg("--release")
        .current_dir(repo_path)
        .status()?;

    let release_dir = repo_path.join("target/release");
    if release_dir.exists() {
        find_binaries_in_dir(&release_dir)
    } else {
        Ok(vec![])
    }
}

fn build_cmake(repo_path: &Path) -> Result<Vec<PathBuf>> {
    Command::new("cmake")
        .arg("--build")
        .arg(".")
        .current_dir(repo_path)
        .status()?;
    find_binaries_in_dir(repo_path)
}

fn find_binaries_in_dir(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() && is_executable(&path)? && !is_intermediate(&path) {
            out.push(path);
        }
    }
    Ok(out)
}

fn is_executable(path: &Path) -> Result<bool> {
    let meta = fs::metadata(path)?;
    Ok(meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

fn is_intermediate(path: &Path) -> bool {
    match path.extension().and_then(|e| e.to_str()) {
        Some("o") | Some("a") | Some("so") | Some("dylib") | Some("dll") | Some("rlib") => true,
        _ => false,
    }
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

pub fn remove(packages: &Vec<String>, base: PathBuf) -> Result<()> {
    std::fs::create_dir_all(&base)?;
    let mut repo_infos = get_repos(&base)?;
    let mut changed = false;
    let xdg = microxdg::Xdg::new()?;

    for package in packages {
        let hash = if let Some(h) = resolve_package(package, &repo_infos)? {
            h
        } else {
            println!("{} not found", package);
            continue;
        };

        if let Some(repo_info) = repo_infos.remove(&hash) {
            let repo_path = base.join(&hash);
            if repo_path.exists() {
                std::fs::remove_dir_all(repo_path)?;
            }
            for binary in repo_info.binaries {
                let bin_path = xdg.bin()?.join(binary);
                if bin_path.exists() {
                    std::fs::remove_file(bin_path)?;
                }
            }
            changed = true;
            println!("Deleted: {}", package);
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

pub fn info(packages: &Vec<String>, base: PathBuf) -> Result<()> {
    std::fs::create_dir_all(&base)?;
    let repo_infos = get_repos(&base)?;

    for package in packages {
        let hash = if let Some(h) = resolve_package(package, &repo_infos)? {
            h
        } else {
            println!("{} not found", package);
            continue;
        };

        if let Some(repo_info) = repo_infos.get(&hash) {
            println!("{}", hash);
            println!("Url: {}", repo_info.url);
            println!(
                "Fetched at: {}",
                millis_to_datetime(repo_info.fetched_at as u64),
            );
            println!("Binaries: {:?}", repo_info.binaries);
        }
    }
    Ok(())
}

fn resolve_package(input: &str, repo_infos: &HashMap<String, RepoInfo>) -> Result<Option<String>> {
    // 1. Is it a URL?
    if input.contains("://") || input.contains("git@") || input.contains("github.com") {
        let normalized = normalize_url(input)?;
        let hash = hash_string(&normalized);
        if repo_infos.contains_key(&hash) {
            return Ok(Some(hash));
        }
    }

    // 2. Is it any of the binaries names?
    for (hash, info) in repo_infos {
        if info.binaries.iter().any(|b| b == input) {
            return Ok(Some(hash.clone()));
        }
    }

    // 3. Matches the hash (prefix matching)
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

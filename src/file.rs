use std::{
    collections::HashMap,
    fs::{self, File},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Result, anyhow};
use git2::{ObjectType, ResetType, build::RepoBuilder};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Serialize, Deserialize, Clone)]
pub struct RepoInfo {
    pub url: String,
    pub last_commit: Option<String>,
    pub fetched_at: u128,
}

/// -------------------------
/// Add repo
/// -------------------------
pub fn add(url: &str, base: PathBuf) -> Result<()> {
    std::fs::create_dir_all(&base)?;

    let mut repos = get_repos(&base)?;

    // 1. Normalize then hash
    let normalized = normalize_url(url)?;
    let hash = hash_string(&normalized);
    let repo_path = base.join(&hash);

    // 2. Clone/Open repo
    let repo = if repo_path.exists() {
        return Err(anyhow!("{url} is already installed"));
    } else {
        RepoBuilder::new().clone(url, &repo_path)?
    };

    // 3. Build
    println!("Building: {}", url);
    build_repo(&repo_path)?;
    println!("Built: {}", url);

    // 4. Capture current state
    let last_commit = repo.head()?.target().map(|oid| oid.to_string());
    let repo_info = RepoInfo {
        url: url.to_string(),
        last_commit,
        fetched_at: SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis(),
    };

    repos.insert(hash, repo_info);
    save_repos(&base, &repos)?;

    Ok(())
}

/// -------------------------
/// Build repo + install binaries
/// -------------------------
pub fn build_repo(repo_path: &Path) -> Result<()> {
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

    // install step
    let bin_dir = microxdg::Xdg::new()?.bin()?;
    fs::create_dir_all(&bin_dir)?;

    for binary in binaries {
        if let Some(name) = binary.file_name() {
            let dest = bin_dir.join(name);
            fs::copy(&binary, &dest)?;
        }
    }

    Ok(())
}

pub fn update(packages: &Option<Vec<String>>, base: PathBuf) -> Result<()> {
    let mut changed = false;
    if packages.is_none() {
        println!("Updating all");
        std::fs::create_dir_all(&base)?;

        let mut repo_infos = get_repos(&base)?;

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
                println!("Rebuilding {}", repo_info.url);
                build_repo(&base.join(hash))?;
                repo_info.last_commit = head_oid;
                changed = true;
                println!("Rebuilt {}", repo_info.url);
            } else {
                println!("{} is already up-to-date", repo_info.url);
            }
        }
        if changed {
            save_repos(&base, &repo_infos)?;
        } else {
            println!("No packages to update");
        }
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
    #[cfg(unix)]
    {
        Ok(meta.is_file() && meta.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        Ok(path.extension().and_then(|e| e.to_str()) == Some("exe"))
    }
}

/// -------------------------
/// Filter intermediates
/// -------------------------
fn is_intermediate(path: &Path) -> bool {
    match path.extension().and_then(|e| e.to_str()) {
        Some("o") | Some("a") | Some("so") | Some("dylib") | Some("dll") | Some("rlib") => true,
        _ => false,
    }
}

/// -------------------------
/// Normalize URL
/// -------------------------
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

/// -------------------------
/// Load repos
/// -------------------------
fn get_repos(base: &Path) -> Result<HashMap<String, RepoInfo>> {
    let path = base.join("repos.json");

    if path.exists() {
        let file = File::open(&path)?;
        Ok(serde_json::from_reader(file)?)
    } else {
        Ok(HashMap::new())
    }
}

/// -------------------------
/// Save repos
/// -------------------------
fn save_repos(base: &Path, repo_infos: &HashMap<String, RepoInfo>) -> Result<()> {
    let path = base.join("repos.json");
    let json = serde_json::to_string_pretty(repo_infos)?;
    fs::write(path, json)?;
    Ok(())
}

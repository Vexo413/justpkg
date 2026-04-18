use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Utc};
use microxdg::Xdg;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    time::{Duration, UNIX_EPOCH},
};

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct Package {
    pub url: String,
    pub reference: String,
    pub commit: String,
    pub synced_at: u128,
    pub build_script: PathBuf,
    pub binaries: Vec<PathBuf>,
}

pub fn get_packages() -> Result<HashMap<String, Package>> {
    let config_path = Xdg::new()
        .context("Failed to initialize XDG directories")?
        .config()
        .context("Failed to get XDG config directory")?
        .join("justpkg");
    let path = config_path.join("repos.json");

    if path.exists() {
        let file = fs::File::open(&path)
            .with_context(|| format!("Failed to open config file: {}", path.display()))?;
        Ok(serde_json::from_reader(file)
            .with_context(|| format!("Failed to parse config file: {}", path.display()))?)
    } else {
        Ok(HashMap::new())
    }
}

pub fn resolve_remote_ref(url: &str, reference: &str) -> Result<git2::Oid> {
    let repo = git2::Repository::init_bare(std::path::Path::new("/tmp/git2-lookup"))
        .context("Failed to create temporary git repository")?;
    let mut remote = repo
        .remote_anonymous(url)
        .with_context(|| format!("Failed to create remote for {}", url))?;

    remote
        .connect(git2::Direction::Fetch)
        .with_context(|| format!("Failed to connect to remote {}", url))?;

    let refs = remote.list().context("Failed to list remote refs")?;

    if reference == "HEAD" {
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
        if name.ends_with(reference) {
            fs::remove_dir_all("/tmp/git2-lookup").context("Failed to clean up temp directory")?;
            return Ok(head.oid());
        }
    }

    fs::remove_dir_all("/tmp/git2-lookup").context("Failed to clean up temp directory")?;

    Err(anyhow!("ref not found: {}", reference))
}

pub fn normalize_url(url: &str) -> Result<String> {
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

pub fn hash_string(s: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(s.as_bytes());
    hex::encode(hasher.finalize())
}

pub fn save_repos(repo_infos: &HashMap<String, Package>) -> Result<()> {
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

pub fn millis_to_datetime(ms: u64) -> DateTime<Utc> {
    let system_time = UNIX_EPOCH + Duration::from_millis(ms);
    system_time.into()
}

pub fn resolve_package(
    input: &str,
    repo_infos: &HashMap<String, Package>,
) -> Result<Option<String>> {
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

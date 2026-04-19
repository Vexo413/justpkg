use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Utc};
use microxdg::Xdg;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    env, fs,
    path::PathBuf,
    time::{Duration, UNIX_EPOCH},
};

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct Package {
    pub url: String,
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

struct TempRepo(PathBuf);

impl Drop for TempRepo {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub fn resolve_remote_ref(url: &str, reference: &str) -> Result<git2::Oid> {
    let temp_path = env::temp_dir().join(format!(
        "justpkg-lookup-{}",
        hash_string(&format!("{}{}{}", url, reference, Utc::now()))
    ));
    let _guard = TempRepo(temp_path.clone());

    let repo = git2::Repository::init_bare(&temp_path)
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

                return resolve_remote_ref(url, name.trim_start_matches("refs/heads/"));
            }
        }
    }

    for head in refs {
        let name = head.name();
        if name.ends_with(reference) {
            return Ok(head.oid());
        }
    }

    Err(anyhow!("ref not found: {}", reference))
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

use crate::commands::RepoInfo;
use anyhow::{Result, anyhow};
use git2::{ObjectType, ResetType};
use microxdg::Xdg;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

pub fn rebuild(base: &Path, hash: &str, repo_info: &mut RepoInfo) -> Result<bool> {
    let xdg = Xdg::new()?;

    let repo = git2::Repository::open(base.join(hash))?;
    {
        let mut remote = repo.find_remote("origin")?;
        remote.fetch(&[] as &[&str], None, None)?;
    }

    let oid = repo.refname_to_id("refs/remotes/origin/HEAD")?;
    let remote_obj = repo.find_object(oid, Some(ObjectType::Commit))?;

    repo.reset(&remote_obj, ResetType::Hard, None)?;

    let head_oid = repo.head()?.target().map(|v| v.to_string());
    let changed = if repo_info.last_commit != head_oid {
        for binary in repo_info.binaries.iter() {
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
        let repo_path = base.join(hash);
        build_repo(&repo_path, &repo_info.command)?;
        install_binaries(&repo_path, &repo_info.binaries)?;
        repo_info.last_commit = head_oid;
        repo_info.fetched_at = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
        println!("Updated: {}", repo_info.url);
        true
    } else {
        println!("{} is already up-to-date", repo_info.url);
        false
    };

    Ok(changed)
}

pub fn build_repo(repo_path: &Path, command: &str) -> Result<()> {
    let command_parts = shell_words::split(command)?;
    match Command::new(&command_parts[0])
        .args(&command_parts[1..])
        .current_dir(repo_path)
        .status()?
        .code()
    {
        Some(0) => Ok(()),
        Some(e) => Err(anyhow!(e)),
        None => Err(anyhow!("Command terminated by outside process")),
    }
}

pub fn install_binaries(repo_path: &Path, binaries: &[PathBuf]) -> Result<()> {
    let bin_dir = microxdg::Xdg::new()?.bin()?;
    fs::create_dir_all(&bin_dir)?;

    for binary in binaries.iter() {
        let symlink_path = bin_dir.join(
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

    Ok(())
}

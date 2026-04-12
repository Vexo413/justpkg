use crate::commands::RepoInfo;
use anyhow::{Result, anyhow};
use git2::{ObjectType, ResetType};
use microxdg::Xdg;
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

pub fn rebuild(base: &PathBuf, hash: &str, repo_info: &mut RepoInfo) -> Result<bool> {
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
            let bin_path = xdg.bin()?.join(binary);
            if bin_path.exists() {
                std::fs::remove_file(bin_path)?;
            }
        }
        let binaries = build_repo(&base.join(hash))?;
        repo_info.binaries = binaries;
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
fn build_just(repo_path: &Path) -> Result<Vec<PathBuf>> {
    let status = Command::new("just")
        .arg("build")
        .current_dir(repo_path)
        .status()?;
    if !status.success() {
        return Err(anyhow!("just build failed"));
    }
    find_binaries_in_dir(repo_path)
}

fn build_make(repo_path: &Path) -> Result<Vec<PathBuf>> {
    let status = Command::new("make")
        .arg("build")
        .current_dir(repo_path)
        .status()?;
    if !status.success() {
        return Err(anyhow!("make build failed"));
    }
    find_binaries_in_dir(repo_path)
}

fn build_cargo(repo_path: &Path) -> Result<Vec<PathBuf>> {
    let status = Command::new("cargo")
        .arg("build")
        .arg("--release")
        .current_dir(repo_path)
        .status()?;
    if !status.success() {
        return Err(anyhow!("cargo build failed"));
    }

    let release_dir = repo_path.join("target/release");
    if release_dir.exists() {
        find_binaries_in_dir(&release_dir)
    } else {
        Ok(vec![])
    }
}

fn build_cmake(repo_path: &Path) -> Result<Vec<PathBuf>> {
    let build_dir = repo_path.join("build");
    if !build_dir.exists() {
        fs::create_dir_all(&build_dir)?;
        Command::new("cmake")
            .arg("-S")
            .arg(repo_path)
            .arg("-B")
            .arg(&build_dir)
            .current_dir(repo_path)
            .status()?;
    }

    Command::new("cmake")
        .arg("--build")
        .arg(&build_dir)
        .current_dir(&build_dir)
        .status()?;
    find_binaries_in_dir(&build_dir)
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

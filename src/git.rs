//! git에서 추적 파일 목록, 현재 커밋, 작업 트리 상태를 읽는다.
//!
//! git 실행 파일을 그대로 부른다. 네트워크를 쓰는 명령은 부르지 않는다.

use std::path::Path;
use std::process::Command;

use crate::error::LoreError;
use crate::source::sha256_hex;

fn git(root: &Path, args: &[&str]) -> Result<Vec<u8>, LoreError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(|error| LoreError::Git {
            message: format!("git을 실행하지 못했습니다: {error}"),
        })?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(LoreError::Git {
            message: String::from_utf8_lossy(&output.stderr).trim().to_string(),
        })
    }
}

fn split_nul(bytes: &[u8]) -> Vec<String> {
    bytes
        .split(|byte| *byte == 0)
        .filter(|part| !part.is_empty())
        .map(|part| String::from_utf8_lossy(part).into_owned())
        .collect()
}

/// git이 추적하는 파일 중 작업 트리에 있는 것. 프로젝트 홈 기준 경로이고 정렬되어 있다.
pub fn tracked_files(root: &Path) -> Result<Vec<String>, LoreError> {
    let mut files: Vec<String> = split_nul(&git(root, &["ls-files", "-z", "--cached"])?)
        .into_iter()
        .filter(|path| root.join(path).is_file())
        .collect();
    files.sort();
    files.dedup();
    Ok(files)
}

/// `dir`이 든 git 레포의 최상위 폴더. git 레포가 아니면 `None`이다.
pub fn toplevel(dir: &Path) -> Option<std::path::PathBuf> {
    git(dir, &["rev-parse", "--show-toplevel"])
        .ok()
        .map(|out| String::from_utf8_lossy(&out).trim().to_string())
        .filter(|path| !path.is_empty())
        .map(std::path::PathBuf::from)
}

/// 현재 커밋. 커밋이 하나도 없으면 `None`이다.
pub fn head_commit(root: &Path) -> Option<String> {
    git(root, &["rev-parse", "--verify", "--quiet", "HEAD"])
        .ok()
        .map(|out| String::from_utf8_lossy(&out).trim().to_string())
        .filter(|commit| !commit.is_empty())
}

/// 작업 트리의 상태를 나타내는 값. 커밋이나 바뀐 파일의 내용이 달라지면 바뀐다.
///
/// `exclude`로 시작하는 경로는 넣지 않는다. lore가 만드는 파일이 상태를 바꾸지 않게 하기 위해서다.
pub fn worktree_fingerprint(root: &Path, exclude: &str) -> Result<String, LoreError> {
    let head = head_commit(root).unwrap_or_default();
    let status = git(
        root,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?;

    let mut text = format!("head {head}\n");
    for entry in split_nul(&status) {
        // "XY 경로" 꼴이 보통이고, 이름 바꾸기의 원래 경로는 경로만 온다.
        let path = match entry.as_bytes().get(2) {
            Some(b' ') if entry.len() > 3 => &entry[3..],
            _ => entry.as_str(),
        };
        if path.starts_with(exclude) {
            continue;
        }
        let hash = std::fs::read(root.join(path))
            .map(|bytes| sha256_hex(&bytes))
            .unwrap_or_else(|_| "-".to_string());
        text.push_str(&format!("{entry}\t{hash}\n"));
    }
    Ok(sha256_hex(text.as_bytes()))
}

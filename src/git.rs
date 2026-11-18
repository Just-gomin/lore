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

/// 비교 기준 이후 바뀐 파일 하나.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub kind: ChangeKind,
    /// 지금 경로. 삭제된 파일은 지워지기 전 경로다.
    pub path: String,
    /// 이름이 바뀐 파일의 원래 경로.
    pub from: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeKind {
    Added,
    Modified,
    Deleted,
    Renamed,
}

/// `base`와 `HEAD`의 merge-base 이후 바뀐 파일. 프로젝트 홈 기준 경로다.
///
/// 커밋하지 않은 변경도 넣고, git이 추적하지 않는 새 파일은 넣지 않는다.
pub fn changed_files(root: &Path, base: &str) -> Result<Vec<Change>, LoreError> {
    let merge_base = git(root, &["merge-base", base, "HEAD"])?;
    let merge_base = String::from_utf8_lossy(&merge_base).trim().to_string();
    let output = git(
        root,
        &[
            "diff",
            "-M",
            "--name-status",
            "--no-color",
            "-z",
            "--relative",
            &merge_base,
        ],
    )?;
    Ok(parse_name_status(&split_nul(&output)))
}

/// `git diff --name-status -z`의 출력을 읽는다. 이름 변경과 복사는 경로가 두 개 온다.
fn parse_name_status(parts: &[String]) -> Vec<Change> {
    let mut changes = Vec::new();
    let mut parts = parts.iter();
    while let Some(status) = parts.next() {
        let Some(path) = parts.next() else { break };
        let change = match status.as_bytes().first() {
            Some(b'R' | b'C') => {
                let Some(to) = parts.next() else { break };
                Change {
                    kind: if status.starts_with('R') {
                        ChangeKind::Renamed
                    } else {
                        ChangeKind::Added
                    },
                    path: to.clone(),
                    from: status.starts_with('R').then(|| path.clone()),
                }
            }
            Some(b'A') => Change {
                kind: ChangeKind::Added,
                path: path.clone(),
                from: None,
            },
            Some(b'D') => Change {
                kind: ChangeKind::Deleted,
                path: path.clone(),
                from: None,
            },
            _ => Change {
                kind: ChangeKind::Modified,
                path: path.clone(),
                from: None,
            },
        };
        changes.push(change);
    }
    changes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 이름_변경은_원래_경로와_새_경로를_함께_읽는다() {
        let parts: Vec<String> = [
            "M", "a.ts", "R087", "old.ts", "new.ts", "A", "b.ts", "D", "c.ts",
        ]
        .iter()
        .map(|part| part.to_string())
        .collect();
        let changes = parse_name_status(&parts);
        assert_eq!(changes.len(), 4);
        assert_eq!(changes[0].kind, ChangeKind::Modified);
        assert_eq!(changes[1].kind, ChangeKind::Renamed);
        assert_eq!(changes[1].path, "new.ts");
        assert_eq!(changes[1].from.as_deref(), Some("old.ts"));
        assert_eq!(changes[2].kind, ChangeKind::Added);
        assert_eq!(changes[3].kind, ChangeKind::Deleted);
        assert_eq!(changes[3].path, "c.ts");
    }
}

//! 통합 테스트가 함께 쓰는 도우미. 고정 예제를 임시 git 레포로 복사한다.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use tempfile::TempDir;

pub fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).unwrap();
        }
    }
}

pub fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=lore", "-c", "user.email=lore@example.com"])
        .args([
            "-c",
            "commit.gpgsign=false",
            "-c",
            "init.defaultBranch=main",
        ])
        .args(args)
        .output()
        .expect("git을 실행할 수 있어야 한다");
    assert!(
        output.status.success(),
        "git {args:?} 실패: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

/// 고정 예제를 임시 폴더에 복사하고 모든 파일을 커밋한 git 레포로 만든다.
pub fn repo(name: &str) -> TempDir {
    let dir = TempDir::new().unwrap();
    copy_dir(&fixture(name), dir.path());
    git(dir.path(), &["init", "-q"]);
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-q", "-m", "fixture"]);
    dir
}

/// `dir`에서 lore 바이너리를 실행한다.
pub fn lore_in(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_lore"))
        .current_dir(dir)
        .args(args)
        .output()
        .expect("lore 바이너리를 실행할 수 있어야 한다")
}

/// `dir`에서 lore 바이너리를 실행하고 `input`을 표준 입력으로 넘긴다.
pub fn lore_with_input(dir: &Path, args: &[&str], input: &str) -> Output {
    use std::io::Write;
    use std::process::Stdio;

    let mut child = Command::new(env!("CARGO_BIN_EXE_lore"))
        .current_dir(dir)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("lore 바이너리를 실행할 수 있어야 한다");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

pub fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("표준 출력은 UTF-8이어야 한다")
}

pub fn json(output: &Output) -> serde_json::Value {
    serde_json::from_str(stdout(output).trim()).expect("표준 출력은 JSON이어야 한다")
}

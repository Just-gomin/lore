//! 임시 git 레포에 커밋을 만들고 `lore-connector-git`의 출력을 확인한다.

use std::path::Path;
use std::process::{Command, Output};

use serde_json::Value;
use tempfile::TempDir;

fn git(dir: &Path, date: &str, args: &[&str]) {
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
        .env("GIT_AUTHOR_DATE", date)
        .env("GIT_COMMITTER_DATE", date)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn commit(dir: &Path, date: &str, file: &str, message: &str) {
    let path = dir.join(file);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, format!("{message}\n")).unwrap();
    git(dir, date, &["add", "-A"]);
    git(dir, date, &["commit", "-q", "-m", message]);
}

/// 커밋 세 개가 있는 레포. 오래된 것부터 서비스 코드, 문서, squash merge한 PR입니다.
fn repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    git(dir.path(), "2026-11-01T10:00:00+00:00", &["init", "-q"]);
    commit(
        dir.path(),
        "2026-11-01T10:00:00+00:00",
        "services/payment/retry.ts",
        "feat: 재시도를 큐로 옮김\n\n인라인 재시도가 요청을 막아서 큐로 옮깁니다.",
    );
    commit(
        dir.path(),
        "2026-11-05T23:30:00+00:00",
        "README.md",
        "docs: 설치 방법",
    );
    commit(
        dir.path(),
        "2026-11-10T09:00:00+00:00",
        "services/payment/policy.ts",
        "feat: 재시도 횟수를 5회로 (#12)",
    );
    dir
}

fn run(dir: &Path, args: &[&str]) -> (Option<i32>, Value) {
    let output: Output = Command::new(env!("CARGO_BIN_EXE_lore-connector-git"))
        .current_dir(dir)
        .args(args)
        .env("TZ", "UTC")
        .output()
        .unwrap();
    let json = serde_json::from_slice(&output.stdout).unwrap_or_else(|_| {
        panic!(
            "JSON이 아닙니다: {}",
            String::from_utf8_lossy(&output.stdout)
        )
    });
    (output.status.code(), json)
}

fn titles(report: &Value) -> Vec<String> {
    report["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|candidate| candidate["title"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn 첫_부모를_따라간_커밋을_최신순으로_후보로_낸다() {
    let dir = repo();
    let (code, report) = run(dir.path(), &["--channel", "git"]);
    assert_eq!(code, Some(0));
    assert_eq!(report["ok"], true);
    assert_eq!(
        titles(&report),
        [
            "feat: 재시도 횟수를 5회로 (#12)",
            "docs: 설치 방법",
            "feat: 재시도를 큐로 옮김"
        ]
    );
    let oldest = &report["candidates"][2];
    let sha = oldest["origin_version"].as_str().unwrap();
    assert_eq!(sha.len(), 40);
    assert_eq!(oldest["origin"], format!("git:{sha}"));
    assert_eq!(oldest["kind"], "evidence");
    assert_eq!(oldest["channel"], "git");
    assert_eq!(oldest["occurred_at"], "2026-11-01");
    assert_eq!(
        oldest["text"],
        "feat: 재시도를 큐로 옮김\n\n인라인 재시도가 요청을 막아서 큐로 옮깁니다.\n\n바뀐 파일:\n- services/payment/retry.ts"
    );
    // 같은 입력에서 같은 출력을 낸다.
    assert_eq!(run(dir.path(), &["--channel", "git"]).1, report);
}

#[test]
fn 날짜는_실행_환경의_로컬_시간대로_본다() {
    let dir = repo();
    let output = Command::new(env!("CARGO_BIN_EXE_lore-connector-git"))
        .current_dir(dir.path())
        .args(["--channel", "git"])
        .env("TZ", "Asia/Seoul")
        .output()
        .unwrap();
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    // 2026-11-05T23:30Z는 서울 시간으로 11월 6일이다.
    assert_eq!(report["candidates"][1]["occurred_at"], "2026-11-06");
}

#[test]
fn 경로와_날짜와_개수로_범위를_좁힌다() {
    let dir = repo();
    let (_, report) = run(dir.path(), &["--channel", "git", "--paths", "services/**"]);
    assert_eq!(
        titles(&report),
        [
            "feat: 재시도 횟수를 5회로 (#12)",
            "feat: 재시도를 큐로 옮김"
        ]
    );
    let (_, report) = run(dir.path(), &["--channel", "git", "--since", "2026-11-03"]);
    assert_eq!(titles(&report).len(), 2);
    let (_, report) = run(dir.path(), &["--channel", "git", "--limit", "1"]);
    assert_eq!(titles(&report), ["feat: 재시도 횟수를 5회로 (#12)"]);
    let (_, report) = run(
        dir.path(),
        &["--channel", "git", "--exclude-pr-commits", "--limit", "1"],
    );
    assert_eq!(titles(&report), ["docs: 설치 방법"]);
}

#[test]
fn github_원격이면_커밋의_웹_주소를_원본으로_쓴다() {
    let dir = repo();
    git(
        dir.path(),
        "2026-11-10T09:00:00+00:00",
        &[
            "remote",
            "add",
            "origin",
            "git@github.com:Just-gomin/blog.git",
        ],
    );
    let (_, report) = run(dir.path(), &["--channel", "git"]);
    let candidate = &report["candidates"][0];
    let sha = candidate["origin_version"].as_str().unwrap();
    assert_eq!(
        candidate["origin"],
        format!("https://github.com/Just-gomin/blog/commit/{sha}")
    );
}

#[test]
fn 최신_판_모드는_커밋_원본만_적힌_판_그대로_낸다() {
    let dir = repo();
    let sources = dir.path().join("docs/sources");
    std::fs::create_dir_all(&sources).unwrap();
    let note = |name: &str, origin: &str, version: &str| {
        std::fs::write(
            sources.join(name),
            format!("---\norigin: {origin}\norigin_version: '{version}'\n---\n"),
        )
        .unwrap();
    };
    note("SRC-a.md", "git:abc", "abc");
    note("SRC-b.md", "https://github.com/o/r/commit/def", "def");
    note("SRC-c.md", "https://github.com/o/r/pull/3", "2026-11-01");
    let output = Command::new(env!("CARGO_BIN_EXE_lore-connector-git"))
        .current_dir(dir.path())
        .args(["--origins", "docs"])
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap().trim(),
        r#"[{"origin":"git:abc","origin_version":"abc"},{"origin":"https://github.com/o/r/commit/def","origin_version":"def"}]"#
    );
}

#[test]
fn 채널이_없거나_git_레포가_아니면_json_오류다() {
    let dir = repo();
    let (code, report) = run(dir.path(), &[]);
    assert_eq!(code, Some(2));
    assert_eq!(report["ok"], false);
    assert_eq!(report["error"]["code"], "usage");

    let empty = TempDir::new().unwrap();
    let (code, report) = run(empty.path(), &["--channel", "git"]);
    assert_eq!(code, Some(1));
    assert_eq!(report["error"]["code"], "git");
}

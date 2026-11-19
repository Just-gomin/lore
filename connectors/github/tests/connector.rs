//! 고정 응답 파일로 `lore-connector-github`의 출력을 확인한다. 네트워크를 쓰지 않는다.

use std::path::Path;
use std::process::Command;

use serde_json::Value;

const FIXTURE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures");

fn run(dir: &Path, args: &[&str]) -> (Option<i32>, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_lore-connector-github"))
        .current_dir(dir)
        .args(args)
        .args(["--fixture", FIXTURE])
        .env("TZ", "UTC")
        .output()
        .unwrap();
    (
        output.status.code(),
        String::from_utf8(output.stdout).unwrap(),
    )
}

fn candidates(args: &[&str]) -> Vec<Value> {
    let dir = tempfile::tempdir().unwrap();
    let mut all = vec!["--channel", "github", "--repo", "o/r"];
    all.extend(args);
    let (code, stdout) = run(dir.path(), &all);
    assert_eq!(code, Some(0), "{stdout}");
    let report: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(report["ok"], true);
    report["candidates"].as_array().unwrap().clone()
}

fn titles(list: &[Value]) -> Vec<&str> {
    list.iter().map(|c| c["title"].as_str().unwrap()).collect()
}

#[test]
fn 병합된_pr과_닫힌_이슈를_최신순으로_낸다() {
    let list = candidates(&[]);
    // 병합하지 않고 닫은 PR(#2)과 이슈 목록에 섞인 PR은 빠진다.
    assert_eq!(titles(&list), ["재시도를 5회로", "중복 결제", "큐 도입"]);
    let pull = &list[0];
    assert_eq!(pull["kind"], "discussion");
    assert_eq!(pull["channel"], "github");
    assert_eq!(pull["origin"], "https://github.com/o/r/pull/3");
    assert_eq!(pull["occurred_at"], "2026-11-10");
    assert_eq!(
        pull["text"],
        "재시도를 5회로\n\n고객 요청으로 늘립니다.\n\n## 코멘트\n\n@pm: 중복 결제 위험은?\n\n@dev: 멱등 키로 막습니다.\n\n## 리뷰\n\n@lead (APPROVED): 좋습니다\n\n@lead (retry.ts): 상수로 빼 주세요"
    );
    assert_eq!(pull["origin_version"].as_str().unwrap().len(), 64);
    // 본문이 없는 PR은 제목만 남는다.
    assert_eq!(list[2]["text"], "큐 도입");
    // 같은 입력에서 같은 출력을 낸다.
    assert_eq!(candidates(&[]), list);
}

#[test]
fn 종류와_날짜와_개수로_범위를_좁힌다() {
    assert_eq!(titles(&candidates(&["--include", "issues"])), ["중복 결제"]);
    assert_eq!(
        titles(&candidates(&["--since", "2026-11-05"])),
        ["재시도를 5회로", "중복 결제"]
    );
    assert_eq!(titles(&candidates(&["--limit", "1"])), ["재시도를 5회로"]);
}

#[test]
fn 날짜는_실행_환경의_로컬_시간대로_본다() {
    let dir = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_lore-connector-github"))
        .current_dir(dir.path())
        .args(["--channel", "github", "--repo", "o/r", "--fixture", FIXTURE])
        .env("TZ", "Asia/Seoul")
        .output()
        .unwrap();
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    // 2026-11-10T23:30Z는 서울 시간으로 11월 11일이다.
    assert_eq!(report["candidates"][0]["occurred_at"], "2026-11-11");
}

#[test]
fn 최신_판_모드는_pr과_이슈_원본의_원문_해시를_낸다() {
    let dir = tempfile::tempdir().unwrap();
    let pulls = candidates(&[]);
    let sources = dir.path().join("docs/sources");
    std::fs::create_dir_all(&sources).unwrap();
    let note = |name: &str, origin: &str, version: &str| {
        std::fs::write(
            sources.join(name),
            format!("---\norigin: {origin}\norigin_version: '{version}'\n---\n"),
        )
        .unwrap();
    };
    note("SRC-a.md", "https://github.com/o/r/pull/3", "옛 판");
    note("SRC-b.md", "https://github.com/o/r/issues/5", "옛 판");
    note("SRC-c.md", "https://github.com/o/r/commit/abc", "abc");
    note("SRC-d.md", "https://github.com/o/r/pull/99", "없는 PR");
    let (code, stdout) = run(dir.path(), &["--origins", "docs"]);
    assert_eq!(code, Some(0), "{stdout}");
    let list: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(
        list,
        serde_json::json!([
            {"origin": "https://github.com/o/r/issues/5", "origin_version": pulls[1]["origin_version"]},
            {"origin": "https://github.com/o/r/pull/3", "origin_version": pulls[0]["origin_version"]},
        ])
    );
}

#[test]
fn 인자가_틀리면_json_사용법_오류다() {
    let dir = tempfile::tempdir().unwrap();
    let (code, stdout) = run(dir.path(), &["--repo", "o/r"]);
    assert_eq!(code, Some(2));
    let report: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(report["error"]["code"], "usage");

    let (code, _) = run(
        dir.path(),
        &[
            "--channel",
            "github",
            "--repo",
            "o/r",
            "--since",
            "2026-1-3",
        ],
    );
    assert_eq!(code, Some(2));
}

#[test]
fn 최신_판_모드에서_gh를_부르지_못하면_빈_목록이_아니라_실패다() {
    let dir = tempfile::tempdir().unwrap();
    let sources = dir.path().join("docs/sources");
    std::fs::create_dir_all(&sources).unwrap();
    std::fs::write(
        sources.join("SRC-a.md"),
        "---\norigin: https://github.com/o/r/pull/3\norigin_version: v\n---\n",
    )
    .unwrap();
    // 고정 응답 없이, gh를 찾을 수 없는 PATH로 실행한다.
    let output = Command::new(env!("CARGO_BIN_EXE_lore-connector-github"))
        .current_dir(dir.path())
        .args(["--origins", "docs"])
        .env("PATH", "")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["ok"], false);
    assert_eq!(report["error"]["code"], "github");
}

#[test]
fn 레포는_owner_name_꼴이어야_한다() {
    let dir = tempfile::tempdir().unwrap();
    let (code, stdout) = run(dir.path(), &["--channel", "github", "--repo", "o"]);
    assert_eq!(code, Some(2), "{stdout}");
}

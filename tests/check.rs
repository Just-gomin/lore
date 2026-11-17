//! `lore check`의 기계적 검사를 고정 예제로 확인한다.

mod common;

use std::path::Path;

use common::{git, json, lore_in, repo, stdout};
use serde_json::Value;
use tempfile::TempDir;

/// 고정 예제에 안내문을 설치하고 커밋해, 모든 검사가 통과하는 레포를 만든다.
fn clean_repo() -> TempDir {
    let dir = repo("payment");
    assert!(
        lore_in(dir.path(), &["install", "--no-skills"])
            .status
            .success()
    );
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-q", "-m", "lore install"]);
    dir
}

fn status_of(report: &Value, id: &str) -> String {
    check_of(report, id)["status"].as_str().unwrap().to_string()
}

fn check_of<'a>(report: &'a Value, id: &str) -> &'a Value {
    report["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|check| check["id"] == id)
        .unwrap_or_else(|| panic!("{id} 검사가 없습니다: {report}"))
}

fn codes(report: &Value, id: &str) -> Vec<String> {
    check_of(report, id)["problems"]
        .as_array()
        .map(|problems| {
            problems
                .iter()
                .map(|p| p["code"].as_str().unwrap().to_string())
                .collect()
        })
        .unwrap_or_default()
}

fn edit(dir: &Path, path: &str, from: &str, to: &str) {
    let file = dir.join(path);
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains(from), "{path}에 {from:?}가 없습니다");
    std::fs::write(file, text.replace(from, to)).unwrap();
}

#[test]
fn 어긋난_곳이_없으면_모든_검사가_통과한다() {
    let dir = clean_repo();
    let output = lore_in(dir.path(), &["check"]);
    assert_eq!(output.status.code(), Some(0), "{}", stdout(&output));
    let report = json(&output);
    assert_eq!(report["ok"], true);
    for id in ["paths", "references", "frontmatter", "index", "guide"] {
        assert_eq!(status_of(&report, id), "pass", "{id}");
        assert_eq!(check_of(&report, id)["blocking"], true);
    }
    // 검사는 파일을 고치지 않는다.
    assert_eq!(git(dir.path(), &["status", "--porcelain"]), "");
}

#[test]
fn 없는_파일을_가리키는_경로는_경로_유효성_실패다() {
    let dir = clean_repo();
    edit(
        dir.path(),
        "docs/specs/payment-notification.md",
        "services/notification/**",
        "services/sms/**",
    );
    let output = lore_in(dir.path(), &["check"]);
    assert_eq!(output.status.code(), Some(1));
    let report = json(&output);
    assert_eq!(report["ok"], false);
    assert_eq!(status_of(&report, "paths"), "fail");
    assert_eq!(codes(&report, "paths"), ["no_matching_files"]);
    assert!(check_of(&report, "paths")["fix"].is_string());
    assert_eq!(status_of(&report, "references"), "pass");
    // 소스에 문제가 있으면 인덱스는 다시 만들 수 없어 건너뛴다.
    assert_eq!(status_of(&report, "index"), "skip");
    assert!(check_of(&report, "index")["reason"].is_string());
}

#[test]
fn 없는_아이디_참조는_참조_유효성_실패다() {
    let dir = clean_repo();
    edit(
        dir.path(),
        "docs/decisions/ADR-20261110-log-format.md",
        "specs: []",
        "specs: [payment-sms]",
    );
    let report = json(&lore_in(dir.path(), &["check"]));
    assert_eq!(status_of(&report, "references"), "fail");
    assert_eq!(codes(&report, "references"), ["unknown_reference"]);
    assert_eq!(status_of(&report, "paths"), "pass");
    assert_eq!(status_of(&report, "frontmatter"), "pass");
}

#[test]
fn 빠진_필수_항목은_머리말_형식_실패다() {
    let dir = clean_repo();
    edit(
        dir.path(),
        "docs/decisions/ADR-20261110-log-format.md",
        "status: proposed\n",
        "",
    );
    let report = json(&lore_in(dir.path(), &["check"]));
    assert_eq!(status_of(&report, "frontmatter"), "fail");
    assert_eq!(codes(&report, "frontmatter"), ["missing_field"]);
}

#[test]
fn 인덱스가_소스와_다르면_실패하고_고치지_않는다() {
    let dir = clean_repo();
    edit(
        dir.path(),
        "docs/specs/payment.md",
        "title: 결제\n",
        "title: 결제 진입점\n",
    );
    let before = std::fs::read_to_string(dir.path().join("docs/INDEX.md")).unwrap();
    let output = lore_in(dir.path(), &["check"]);
    assert_eq!(output.status.code(), Some(1));
    let report = json(&output);
    assert_eq!(status_of(&report, "index"), "fail");
    assert_eq!(codes(&report, "index"), ["index_outdated"]);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("docs/INDEX.md")).unwrap(),
        before
    );

    // 안내한 대로 lore build를 실행하면 통과한다.
    assert!(lore_in(dir.path(), &["build"]).status.success());
    assert_eq!(
        status_of(&json(&lore_in(dir.path(), &["check"])), "index"),
        "pass"
    );

    std::fs::remove_file(dir.path().join("docs/INDEX.md")).unwrap();
    let report = json(&lore_in(dir.path(), &["check"]));
    assert_eq!(codes(&report, "index"), ["index_missing"]);
}

#[test]
fn 안내문_구간이_없거나_다르면_실패한다() {
    let dir = repo("payment");
    let report = json(&lore_in(dir.path(), &["check"]));
    assert_eq!(codes(&report, "guide"), ["guide_missing"]);

    assert!(
        lore_in(dir.path(), &["install", "--no-skills"])
            .status
            .success()
    );
    assert_eq!(
        status_of(&json(&lore_in(dir.path(), &["check"])), "guide"),
        "pass"
    );

    edit(dir.path(), "AGENTS.md", "### 원칙", "### 우리 원칙");
    let report = json(&lore_in(dir.path(), &["check"]));
    assert_eq!(codes(&report, "guide"), ["guide_outdated"]);

    edit(dir.path(), "AGENTS.md", "<!-- lore:end -->\n", "");
    let report = json(&lore_in(dir.path(), &["check"]));
    assert_eq!(codes(&report, "guide"), ["broken_marker"]);
}

#[test]
fn 도구별_파일은_있을_때만_구간을_본다() {
    let dir = clean_repo();
    std::fs::write(dir.path().join("CLAUDE.md"), "# 규칙\n").unwrap();
    let output = lore_in(dir.path(), &["check"]);
    assert_eq!(output.status.code(), Some(1));
    let report = json(&output);
    let problems = check_of(&report, "guide")["problems"].as_array().unwrap();
    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0]["path"], "CLAUDE.md");
    assert_eq!(problems[0]["code"], "guide_missing");

    assert!(
        lore_in(dir.path(), &["install", "--no-skills"])
            .status
            .success()
    );
    assert_eq!(
        status_of(&json(&lore_in(dir.path(), &["check"])), "guide"),
        "pass"
    );
}

#[test]
fn 사람용_출력은_실패를_먼저_보여_주고_표준_출력으로_낸다() {
    let dir = clean_repo();
    std::fs::remove_file(dir.path().join("docs/INDEX.md")).unwrap();
    let output = lore_in(dir.path(), &["check", "--for-human"]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        stdout(&output),
        "lore check: 실패 1, 통과 4

실패  인덱스 일치
  docs/INDEX.md 인덱스 파일이 없습니다.
  고치려면: lore build를 실행하고 바뀐 인덱스를 커밋하세요.

통과  경로 유효성

통과  참조 유효성

통과  머리말 형식

통과  안내문 일치
"
    );
}

#[test]
fn 잘못된_고정_예제는_문제를_검사별로_나눈다() {
    let dir = repo("invalid");
    let output = lore_in(dir.path(), &["check"]);
    assert_eq!(output.status.code(), Some(1));
    let report = json(&output);
    // 소스 읽기의 문제 17개에, lore build라면 거기서 멈춰 보지 못했을 경로 문제 1개가 더해진다.
    for (id, count) in [("paths", 1), ("references", 6), ("frontmatter", 11)] {
        assert_eq!(status_of(&report, id), "fail", "{id}");
        assert_eq!(codes(&report, id).len(), count, "{id}");
    }
}

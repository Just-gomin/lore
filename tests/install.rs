//! `lore install`이 빈 레포와 이미 파일이 있는 레포에 안내문과 뼈대를 심는 과정을 확인한다.

mod common;

use std::path::Path;

use common::{git, json, lore_in};
use serde_json::{Value, json};
use tempfile::TempDir;

fn empty_repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    git(dir.path(), &["init", "-q"]);
    dir
}

fn changes(report: &Value) -> Vec<(String, String)> {
    report["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            (
                f["path"].as_str().unwrap().to_string(),
                f["change"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

fn read(dir: &Path, path: &str) -> String {
    std::fs::read_to_string(dir.join(path)).unwrap()
}

#[test]
fn 빈_레포에_설정_뼈대_안내문을_만들고_빌드가_된다() {
    let dir = empty_repo();
    let sub = dir.path().join("src");
    std::fs::create_dir_all(&sub).unwrap();
    let report = json(&lore_in(&sub, &["install"]));
    assert_eq!(report["ok"], true);
    assert_eq!(
        changes(&report),
        [
            ("lore.config.yaml", "created"),
            ("docs/specs/.gitkeep", "created"),
            ("docs/decisions/.gitkeep", "created"),
            ("docs/incidents/.gitkeep", "created"),
            ("docs/sources/.gitkeep", "created"),
            ("AGENTS.md", "created"),
        ]
        .map(|(p, c)| (p.to_string(), c.to_string()))
    );
    assert_eq!(
        read(dir.path(), "AGENTS.md"),
        lore::install::section(lore::install::AGENTS_BODY)
    );
    assert!(lore_in(dir.path(), &["build"]).status.success());
}

#[test]
fn 두_번_실행하면_아무것도_바뀌지_않는다() {
    let dir = empty_repo();
    std::fs::write(dir.path().join("CLAUDE.md"), "# 규칙\n").unwrap();
    assert!(lore_in(dir.path(), &["install"]).status.success());
    let second = json(&lore_in(dir.path(), &["install"]));
    assert!(
        changes(&second)
            .iter()
            .all(|(_, change)| change == "unchanged"),
        "{second}"
    );
}

#[test]
fn 구간_밖의_글은_그대로_두고_도구별_파일에는_한_줄만_넣는다() {
    let dir = empty_repo();
    let agents = "# AGENTS.md\n\n이 레포의 규칙.\n";
    std::fs::write(dir.path().join("AGENTS.md"), agents).unwrap();
    std::fs::write(dir.path().join("CLAUDE.md"), "# 규칙\n").unwrap();
    assert!(lore_in(dir.path(), &["install"]).status.success());

    let text = read(dir.path(), "AGENTS.md");
    assert!(text.starts_with(&format!(
        "{agents}\n<!-- lore:begin v1 -->\n\n## lore로 작업하기\n"
    )));
    assert_eq!(
        read(dir.path(), "CLAUDE.md"),
        "# 규칙\n\n<!-- lore:begin v1 -->\n\n- 작업 기록과 설계 절차는 `AGENTS.md`의 lore 구간을 따릅니다.\n\n<!-- lore:end -->\n"
    );
    assert!(!dir.path().join("GEMINI.md").exists());
}

#[test]
fn 낡은_구간은_새_안내문으로_바꾼다() {
    let dir = empty_repo();
    std::fs::write(
        dir.path().join("AGENTS.md"),
        "앞\n\n<!-- lore:begin v1 -->\n\n옛 안내문\n\n<!-- lore:end -->\n\n뒤\n",
    )
    .unwrap();
    let report = json(&lore_in(dir.path(), &["install"]));
    assert!(changes(&report).contains(&("AGENTS.md".to_string(), "updated".to_string())));
    let text = read(dir.path(), "AGENTS.md");
    assert!(text.starts_with("앞\n\n<!-- lore:begin v1 -->\n\n## lore로 작업하기\n"));
    assert!(text.ends_with("<!-- lore:end -->\n\n뒤\n"));
    assert!(!text.contains("옛 안내문"));
}

#[test]
fn 설정이_있으면_그_docs_폴더에_뼈대를_만든다() {
    let dir = empty_repo();
    std::fs::write(
        dir.path().join("lore.config.yaml"),
        "version: 1\nproject:\n  docs: notes\n",
    )
    .unwrap();
    std::fs::create_dir_all(dir.path().join("notes/specs")).unwrap();
    let report = json(&lore_in(dir.path(), &["install"]));
    let found = changes(&report);
    assert!(found.contains(&("lore.config.yaml".to_string(), "unchanged".to_string())));
    assert!(found.contains(&("notes/specs/".to_string(), "unchanged".to_string())));
    assert!(found.contains(&(
        "notes/decisions/.gitkeep".to_string(),
        "created".to_string()
    )));
}

#[test]
fn dry_run은_목록만_내고_쓰지_않는다() {
    let dir = empty_repo();
    let report = json(&lore_in(dir.path(), &["install", "--dry-run"]));
    assert_eq!(report["dry_run"], true);
    assert_eq!(changes(&report).len(), 6);
    assert_eq!(
        git(
            dir.path(),
            &["status", "--porcelain", "--untracked-files=all"]
        ),
        ""
    );
}

#[test]
fn 깨진_표식은_고치지_않고_오류를_낸다() {
    let dir = empty_repo();
    let broken = "규칙\n<!-- lore:begin v1 -->\n안내문\n";
    std::fs::write(dir.path().join("AGENTS.md"), broken).unwrap();
    let output = lore_in(dir.path(), &["install"]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        json(&output)["error"],
        json!({"code": "broken_marker", "path": "AGENTS.md"})
    );
    assert_eq!(read(dir.path(), "AGENTS.md"), broken);
}

#[test]
fn git_레포가_아니면_not_a_git_repo다() {
    let dir = TempDir::new().unwrap();
    let output = lore_in(dir.path(), &["install"]);
    assert_eq!(json(&output)["error"]["code"], "not_a_git_repo");
}

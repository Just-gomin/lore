//! `lore uninstall`이 `lore install`이 넣은 것만 걷어내는지 확인한다.

mod common;

use std::path::Path;

use common::{git, json, lore_in};
use serde_json::Value;
use tempfile::TempDir;

fn empty_repo() -> TempDir {
    let dir = TempDir::new().unwrap();
    git(dir.path(), &["init", "-q"]);
    dir
}

fn change_of(report: &Value, path: &str) -> String {
    report["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["path"] == path)
        .map(|f| f["change"].as_str().unwrap().to_string())
        .unwrap_or_else(|| panic!("{path}가 출력에 없습니다: {report}"))
}

/// 작업 트리의 파일 목록. `.git`은 뺀다.
fn files(dir: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        for entry in std::fs::read_dir(current).unwrap() {
            let path = entry.unwrap().path();
            let relative = path
                .strip_prefix(dir)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            if relative == ".git" {
                continue;
            }
            if path.is_dir() {
                found.push(format!("{relative}/"));
                stack.push(path);
            } else {
                found.push(relative);
            }
        }
    }
    found.sort();
    found
}

#[test]
fn 빈_레포에서_걷어내면_문서_뼈대와_설정만_남는다() {
    let dir = empty_repo();
    assert!(lore_in(dir.path(), &["install"]).status.success());
    let report = json(&lore_in(dir.path(), &["uninstall"]));
    assert_eq!(change_of(&report, "AGENTS.md"), "removed");
    assert_eq!(
        change_of(&report, ".claude/skills/lore-record/SKILL.md"),
        "removed"
    );
    assert_eq!(change_of(&report, ".github/workflows/lore.yml"), "removed");
    assert_eq!(
        files(dir.path()),
        [
            "docs/",
            "docs/decisions/",
            "docs/decisions/.gitkeep",
            "docs/incidents/",
            "docs/incidents/.gitkeep",
            "docs/sources/",
            "docs/sources/.gitkeep",
            "docs/specs/",
            "docs/specs/.gitkeep",
            "lore.config.yaml",
        ]
    );
}

#[test]
fn 원래_있던_안내_파일은_바이트_단위로_돌아온다() {
    let dir = empty_repo();
    let agents = "# AGENTS.md\n\n이 레포의 규칙.\n";
    let claude = "# 규칙\n\n- 한국어로 답합니다.\n";
    std::fs::write(dir.path().join("AGENTS.md"), agents).unwrap();
    std::fs::write(dir.path().join("CLAUDE.md"), claude).unwrap();
    assert!(lore_in(dir.path(), &["install"]).status.success());

    let report = json(&lore_in(dir.path(), &["uninstall"]));
    assert_eq!(change_of(&report, "AGENTS.md"), "updated");
    assert_eq!(change_of(&report, "CLAUDE.md"), "updated");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("AGENTS.md")).unwrap(),
        agents
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("CLAUDE.md")).unwrap(),
        claude
    );
}

#[test]
fn 구간만_남은_도구별_파일도_지우지_않는다() {
    let dir = empty_repo();
    std::fs::write(dir.path().join("CLAUDE.md"), "").unwrap();
    assert!(lore_in(dir.path(), &["install"]).status.success());
    assert!(lore_in(dir.path(), &["uninstall"]).status.success());
    assert_eq!(
        std::fs::read_to_string(dir.path().join("CLAUDE.md")).unwrap(),
        ""
    );
}

#[test]
fn 사람이_고친_스킬은_남기고_예전_버전의_스킬은_지운다() {
    let dir = empty_repo();
    assert!(lore_in(dir.path(), &["install"]).status.success());
    let record = dir.path().join(".claude/skills/lore-record/SKILL.md");
    let edited = std::fs::read_to_string(&record).unwrap() + "\n## 우리 팀 규칙\n";
    std::fs::write(&record, &edited).unwrap();
    let source = dir.path().join(".claude/skills/lore-source/SKILL.md");
    std::fs::write(
        &source,
        lore::install::installed_skill(
            "---\nname: lore-source\ndescription: 옛 설명\n---\n\n옛 본문\n",
        ),
    )
    .unwrap();

    let report = json(&lore_in(dir.path(), &["uninstall"]));
    assert_eq!(
        change_of(&report, ".claude/skills/lore-record/SKILL.md"),
        "kept"
    );
    assert_eq!(
        change_of(&report, ".claude/skills/lore-source/SKILL.md"),
        "removed"
    );
    assert_eq!(std::fs::read_to_string(&record).unwrap(), edited);
    assert!(!dir.path().join(".claude/skills/lore-source").exists());
}

#[test]
fn with_config면_설정도_지우고_기록은_남긴다() {
    let dir = empty_repo();
    assert!(lore_in(dir.path(), &["install"]).status.success());
    std::fs::write(
        dir.path().join("docs/decisions/ADR-20261117-a.md"),
        "기록\n",
    )
    .unwrap();
    let report = json(&lore_in(dir.path(), &["uninstall", "--with-config"]));
    assert_eq!(change_of(&report, "lore.config.yaml"), "removed");
    assert!(!dir.path().join("lore.config.yaml").exists());
    assert!(dir.path().join("docs/decisions/ADR-20261117-a.md").exists());
}

#[test]
fn dry_run은_목록만_내고_지우지_않는다() {
    let dir = empty_repo();
    assert!(lore_in(dir.path(), &["install"]).status.success());
    let before = files(dir.path());
    let report = json(&lore_in(
        dir.path(),
        &["uninstall", "--dry-run", "--with-config"],
    ));
    assert_eq!(report["dry_run"], true);
    assert_eq!(change_of(&report, "AGENTS.md"), "removed");
    assert_eq!(files(dir.path()), before);
}

#[test]
fn 사람이_고친_워크플로는_지우지_않는다() {
    let dir = empty_repo();
    assert!(lore_in(dir.path(), &["install"]).status.success());
    let path = dir.path().join(".github/workflows/lore.yml");
    let edited = std::fs::read_to_string(&path)
        .unwrap()
        .replace("ubuntu-latest", "ubuntu-24.04");
    std::fs::write(&path, &edited).unwrap();
    std::fs::write(dir.path().join(".github/workflows/ci.yml"), "name: ci\n").unwrap();

    let report = json(&lore_in(dir.path(), &["uninstall"]));
    assert_eq!(change_of(&report, ".github/workflows/lore.yml"), "kept");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), edited);
}

#[test]
fn 다른_워크플로가_있으면_github_폴더는_남긴다() {
    let dir = empty_repo();
    assert!(lore_in(dir.path(), &["install"]).status.success());
    std::fs::write(dir.path().join(".github/workflows/ci.yml"), "name: ci\n").unwrap();
    let report = json(&lore_in(dir.path(), &["uninstall"]));
    assert_eq!(change_of(&report, ".github/workflows/lore.yml"), "removed");
    assert!(dir.path().join(".github/workflows/ci.yml").exists());
}

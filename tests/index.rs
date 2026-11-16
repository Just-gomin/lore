//! `lore build`가 만드는 `docs/INDEX.md`를 확인한다.

mod common;

use common::{fixture, git, lore_in, repo};

fn index(dir: &std::path::Path) -> String {
    std::fs::read_to_string(dir.join("docs/INDEX.md")).unwrap()
}

#[test]
fn 커밋된_인덱스와_같은_소스에서는_작업_트리가_그대로다() {
    let dir = repo("payment");
    assert!(lore_in(dir.path(), &["build"]).status.success());
    assert_eq!(git(dir.path(), &["status", "--porcelain"]), "");
    assert_eq!(
        index(dir.path()),
        std::fs::read_to_string(fixture("payment").join("docs/INDEX.md")).unwrap()
    );
}

#[test]
fn 인덱스가_없으면_새로_만든다() {
    let dir = repo("payment");
    git(dir.path(), &["rm", "-q", "docs/INDEX.md"]);
    git(dir.path(), &["commit", "-q", "-m", "인덱스 지우기"]);
    assert!(lore_in(dir.path(), &["build"]).status.success());
    assert_eq!(
        git(dir.path(), &["status", "--porcelain"]),
        "?? docs/INDEX.md\n"
    );
}

#[test]
fn 대체된_결정에는_대체한_결정을_적는다() {
    let dir = repo("payment");
    assert!(lore_in(dir.path(), &["build"]).status.success());
    let line = index(dir.path())
        .lines()
        .find(|line| line.contains("`ADR-20260918-retry-inline`"))
        .unwrap()
        .to_string();
    assert!(
        line.ends_with(" · [대체됨: ADR-20261105-retry-queue]"),
        "{line}"
    );
}

#[test]
fn 스펙에_연결되지_않은_기록은_맨_아래에_모은다() {
    let dir = repo("payment");
    assert!(lore_in(dir.path(), &["build"]).status.success());
    let text = index(dir.path());
    let (_, tail) = text
        .split_once("\n## 스펙에 연결되지 않은 기록\n\n")
        .unwrap();
    assert_eq!(
        tail,
        "- `ADR-20261110-log-format` · 로그를 JSON 한 줄로 남긴다 · [결정](decisions/ADR-20261110-log-format.md)\n"
    );
}

#[test]
fn 소스_노트는_근거가_되는_기록의_스펙에_묶인다() {
    let dir = repo("payment");
    assert!(lore_in(dir.path(), &["build"]).status.success());
    let text = index(dir.path());
    let section = |spec: &str| {
        let start = text.find(&format!("\n## {spec}\n")).unwrap();
        let rest = &text[start + 1..];
        let end = rest[1..].find("\n## ").map_or(rest.len(), |i| i + 1);
        rest[..end].to_string()
    };
    assert!(section("payment-core").contains("`SRC-20261020-incident-thread`"));
    assert!(!section("payment-core").contains("`SRC-20261103-retry-meeting`"));
    assert!(section("payment-retry").contains("`SRC-20261103-retry-meeting`"));
}

#[test]
fn 소스가_잘못되면_인덱스를_건드리지_않는다() {
    let dir = repo("payment");
    let spec = dir.path().join("docs/specs/payment.md");
    let text = std::fs::read_to_string(&spec).unwrap();
    std::fs::write(&spec, text.replace("title: 결제\n", "title: 결제 진입점\n")).unwrap();
    std::fs::write(
        dir.path().join("docs/decisions/ADR-20261111-bad.md"),
        "---\nid: ADR-20261111-bad\n---\n",
    )
    .unwrap();
    assert_eq!(lore_in(dir.path(), &["build"]).status.code(), Some(1));
    assert!(!index(dir.path()).contains("결제 진입점"));
}

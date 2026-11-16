//! 소스 폴더를 읽어 머리말을 해석하는 과정을 고정 예제로 확인한다.

use std::path::{Path, PathBuf};

use lore::config;
use lore::error::LoreError;
use lore::source::{self, Corpus, Kind, Record};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

fn read(name: &str) -> Result<Corpus, LoreError> {
    let config = config::load(&fixture(name)).expect("고정 예제의 설정은 읽혀야 한다");
    source::read(&config)
}

#[test]
fn 정상_예제는_오류_없이_종류별로_읽힌다() {
    let corpus = read("payment").expect("정상 예제는 오류가 없어야 한다");
    let count = |kind| corpus.of_kind(kind).count();
    assert_eq!(count(Kind::Spec), 4);
    assert_eq!(count(Kind::Decision), 4);
    assert_eq!(count(Kind::Incident), 1);
    assert_eq!(count(Kind::Source), 2);

    let ids: Vec<&str> = corpus.documents.iter().map(|doc| doc.id.as_str()).collect();
    let mut sorted = ids.clone();
    sorted.sort();
    assert_eq!(ids, sorted, "문서는 ID 순으로 정렬되어야 한다");
}

#[test]
fn 밑줄_폴더와_계획_폴더는_읽지_않는다() {
    let corpus = read("payment").unwrap();
    assert!(corpus.get("ADR-YYYYMMDD-slug").is_none());
    assert!(
        corpus
            .documents
            .iter()
            .all(|doc| !doc.path.contains("_templates") && !doc.path.contains("plans"))
    );
}

#[test]
fn 머리말_항목을_구조체로_옮긴다() {
    let corpus = read("payment").unwrap();

    let retry = corpus.get("payment-retry").unwrap();
    assert_eq!(retry.path, "docs/specs/payment-retry.md");
    let spec = retry.spec().unwrap();
    assert_eq!(spec.paths, ["services/payment/retry/**"]);
    assert_eq!(spec.depends_on, ["payment-core"]);
    assert_eq!(spec.parent.as_deref(), Some("payment"));
    assert!(retry.body.starts_with("## 지금 동작"));

    let queue = corpus.get("ADR-20261105-retry-queue").unwrap();
    assert_eq!(queue.date().as_deref(), Some("2026-11-05"));
    let decision = queue.decision().unwrap();
    assert_eq!(
        decision.supersedes.as_deref(),
        Some("ADR-20260918-retry-inline")
    );
    assert_eq!(
        decision.sources,
        [
            "SRC-20261103-retry-meeting",
            "https://github.com/org/repo/pull/231"
        ]
    );

    let incident = corpus.get("INC-20261020-duplicate-charge").unwrap();
    assert!(
        matches!(&incident.record, Record::Incident(i) if i.decisions == ["ADR-20261022-idempotency-key"])
    );

    let note = corpus.get("SRC-20261103-retry-meeting").unwrap();
    assert!(matches!(&note.record, Record::Source(s) if s.sensitivity == "sensitive"));
    assert_eq!(note.hash.len(), 64);
}

#[test]
fn 잘못된_예제는_파일마다_기대한_오류를_한_번에_낸다() {
    let problems = match read("invalid") {
        Err(LoreError::InvalidSources { problems }) => problems,
        other => panic!("InvalidSources가 아니다: {other:?}"),
    };
    let found: Vec<(&str, Option<&str>, &str)> = problems
        .iter()
        .map(|p| (p.path.as_str(), p.field.as_deref(), p.code))
        .collect();

    let decisions = "docs/decisions/";
    let expected = [
        (
            "ADR-20261101-missing-status.md",
            Some("owner"),
            "unknown_field",
        ),
        (
            "ADR-20261101-missing-status.md",
            Some("status"),
            "missing_field",
        ),
        ("ADR-20261102-wrong-name.md", Some("id"), "id_mismatch"),
        (
            "ADR-20261105-no-frontmatter.md",
            None,
            "missing_frontmatter",
        ),
        ("ADR-20261106-bad-yaml.md", None, "invalid_yaml"),
        (
            "ADR-20261107-bad-ref.md",
            Some("confidence"),
            "invalid_value",
        ),
        (
            "ADR-20261107-bad-ref.md",
            Some("sources"),
            "invalid_reference",
        ),
        (
            "ADR-20261107-bad-ref.md",
            Some("specs"),
            "invalid_reference",
        ),
        (
            "ADR-20261107-bad-ref.md",
            Some("supersedes"),
            "invalid_reference",
        ),
    ]
    .map(|(file, field, code)| (format!("{decisions}{file}"), field, code));
    let others = [
        (
            "docs/incidents/INC-20261103-no-title.md",
            Some("decisions"),
            "unknown_reference",
        ),
        (
            "docs/incidents/INC-20261103-no-title.md",
            Some("sources"),
            "unknown_reference",
        ),
        (
            "docs/incidents/INC-20261103-no-title.md",
            Some("title"),
            "missing_field",
        ),
        (
            "docs/sources/SRC-20261104-bad.md",
            Some("extracted_at"),
            "invalid_value",
        ),
        (
            "docs/sources/SRC-20261104-bad.md",
            Some("origin"),
            "missing_field",
        ),
        (
            "docs/sources/SRC-20261104-bad.md",
            Some("sensitivity"),
            "invalid_value",
        ),
        ("docs/specs/no-paths.md", Some("paths"), "missing_field"),
        (
            "docs/specs/orders.md",
            Some("depends_on"),
            "unknown_reference",
        ),
    ];
    let expected: Vec<(&str, Option<&str>, &str)> = expected
        .iter()
        .map(|(path, field, code)| (path.as_str(), *field, *code))
        .chain(others)
        .collect();

    assert_eq!(found, expected);
    assert!(problems.iter().all(|p| !p.message.is_empty()));
}

/// 블로그 레포로 확인할 때만 돌린다: `LORE_BLOG=<경로> cargo test -- --ignored`
#[test]
#[ignore]
fn 블로그_레포의_문서가_오류_없이_읽힌다() {
    let root = std::env::var("LORE_BLOG").expect("LORE_BLOG에 블로그 레포 경로를 넣는다");
    let config = config::load(Path::new(&root)).unwrap();
    let corpus = source::read(&config).unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(corpus.of_kind(Kind::Spec).count(), 5);
    assert_eq!(corpus.of_kind(Kind::Decision).count(), 1);
}

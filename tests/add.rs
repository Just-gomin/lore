//! `lore add decision`, `lore add incident`, `lore add spec`이 고정 예제에 문서를 만드는 과정을 확인한다.

mod common;

use common::{json, lore_in, repo, stdout};
use serde_json::json;

const RETRY_5: [&str; 12] = [
    "add",
    "decision",
    "--slug",
    "retry-5",
    "--title",
    "재시도를 최대 5회로 늘린다",
    "--specs",
    "payment-retry",
    "--supersedes",
    "ADR-20261105-retry-queue",
    "--date",
    "2026-11-12",
];

#[test]
fn 대체하는_결정을_만들고_확인할_스펙을_알려_준다() {
    let dir = repo("payment");
    let output = lore_in(dir.path(), &RETRY_5);
    assert!(output.status.success(), "{}", stdout(&output));
    assert_eq!(
        json(&output),
        json!({
            "ok": true,
            "id": "ADR-20261112-retry-5",
            "kind": "decision",
            "path": "docs/decisions/ADR-20261112-retry-5.md",
            "specs_to_review": ["payment-retry"]
        })
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("docs/decisions/ADR-20261112-retry-5.md")).unwrap(),
        "---
id: ADR-20261112-retry-5
title: 재시도를 최대 5회로 늘린다
status: accepted # proposed | accepted
supersedes: ADR-20261105-retry-queue
specs: [payment-retry]
sources: []
confidence: inferred # stated | inferred
---

## 배경

## 결정

## 검토한 대안

## 결과와 감수한 비용
"
    );
}

#[test]
fn 만든_결정은_빌드와_조회에_바로_쓰인다() {
    let dir = repo("payment");
    assert!(lore_in(dir.path(), &RETRY_5).status.success());
    assert!(lore_in(dir.path(), &["build"]).status.success());
    let report = json(&lore_in(dir.path(), &["why", "payment-retry"]));
    let decisions = report["features"][0]["decisions"].as_array().unwrap();
    let old = decisions
        .iter()
        .find(|d| d["id"] == "ADR-20261105-retry-queue")
        .unwrap();
    assert_eq!(old["superseded_by"], "ADR-20261112-retry-5");
}

#[test]
fn 같은_아이디가_있으면_id_exists로_실패한다() {
    let dir = repo("payment");
    assert!(lore_in(dir.path(), &RETRY_5).status.success());
    let output = lore_in(dir.path(), &RETRY_5);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(json(&output)["error"]["code"], "id_exists");
}

#[test]
fn 없는_문서를_가리키면_파일을_만들지_않는다() {
    let dir = repo("payment");
    let output = lore_in(
        dir.path(),
        &[
            "add",
            "incident",
            "--slug",
            "lost-alert",
            "--title",
            "알림 누락",
            "--specs",
            "payment-sms",
            "--decisions",
            "ADR-20261105-retry-queue",
            "--sources",
            "SRC-20261101-none",
            "--date",
            "2026-11-15",
        ],
    );
    assert_eq!(output.status.code(), Some(1));
    let report = json(&output);
    assert_eq!(report["error"]["code"], "invalid_sources");
    let fields: Vec<&str> = report["error"]["problems"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["field"].as_str().unwrap())
        .collect();
    assert_eq!(fields, ["sources", "specs"]);
    assert!(
        !dir.path()
            .join("docs/incidents/INC-20261115-lost-alert.md")
            .exists()
    );
}

#[test]
fn 다른_문서의_문제는_새_기록을_막지_않는다() {
    let dir = repo("payment");
    let spec = dir.path().join("docs/specs/payment.md");
    let text = std::fs::read_to_string(&spec).unwrap();
    std::fs::write(&spec, text.replace("depends_on: []", "depends_on: [nope]")).unwrap();
    let output = lore_in(
        dir.path(),
        &[
            "add",
            "incident",
            "--slug",
            "lost-alert",
            "--title",
            "알림 누락: 재시도 뒤",
            "--specs",
            "payment-notification",
            "--date",
            "2026-11-15",
        ],
    );
    assert!(output.status.success(), "{}", stdout(&output));
    let text =
        std::fs::read_to_string(dir.path().join("docs/incidents/INC-20261115-lost-alert.md"))
            .unwrap();
    assert!(text.contains("title: \"알림 누락: 재시도 뒤\"\n"), "{text}");
    assert!(text.contains("specs: [payment-notification] # 사고가 난 기능\n"));
}

#[test]
fn 날짜를_주지_않으면_오늘로_발급한다() {
    let dir = repo("payment");
    let report = json(&lore_in(
        dir.path(),
        &[
            "add",
            "decision",
            "--slug",
            "log-level",
            "--title",
            "로그 수준",
        ],
    ));
    let today = lore::add::today().replace('-', "");
    assert_eq!(report["id"], format!("ADR-{today}-log-level"));
}

#[test]
fn 스펙을_슬러그_아이디로_만들고_빌드에_바로_쓰인다() {
    let dir = repo("payment");
    let output = lore_in(
        dir.path(),
        &[
            "add",
            "spec",
            "--slug",
            "payment-charge",
            "--title",
            "카드 승인",
            "--paths",
            "services/payment/core/{charge,refund}.ts",
            "--tests",
            "services/payment/core/**/*.test.ts",
            "--depends-on",
            "payment-retry",
            "--parent",
            "payment",
        ],
    );
    assert!(output.status.success(), "{}", stdout(&output));
    assert_eq!(
        json(&output),
        json!({
            "ok": true,
            "id": "payment-charge",
            "kind": "feature",
            "path": "docs/specs/payment-charge.md",
        })
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("docs/specs/payment-charge.md")).unwrap(),
        "---
id: payment-charge
title: 카드 승인
paths: [\"services/payment/core/{charge,refund}.ts\"]
tests: [services/payment/core/**/*.test.ts]
depends_on: [payment-retry]
parent: payment
---

## 지금 동작
"
    );
    assert!(lore_in(dir.path(), &["build"]).status.success());
}

#[test]
fn 없는_상위_스펙을_가리키면_스펙을_만들지_않고_빈_목록은_그대로_적는다() {
    let dir = repo("payment");
    let output = lore_in(
        dir.path(),
        &[
            "add", "spec", "--slug", "sms", "--title", "문자", "--parent", "notify",
        ],
    );
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(json(&output)["error"]["problems"][0]["field"], "parent");
    assert!(!dir.path().join("docs/specs/sms.md").exists());

    let output = lore_in(
        dir.path(),
        &["add", "spec", "--slug", "sms", "--title", "문자"],
    );
    assert!(output.status.success());
    assert_eq!(
        std::fs::read_to_string(dir.path().join("docs/specs/sms.md")).unwrap(),
        "---\nid: sms\ntitle: 문자\npaths: []\ntests: []\ndepends_on: []\n---\n\n## 지금 동작\n"
    );

    let output = lore_in(
        dir.path(),
        &["add", "spec", "--slug", "payment", "--title", "결제"],
    );
    assert_eq!(json(&output)["error"]["code"], "id_exists");

    // YAML이 숫자로 읽는 슬러그도 따옴표로 감싸 문자열 ID로 남긴다.
    let output = lore_in(
        dir.path(),
        &["add", "spec", "--slug", "2026", "--title", "연도"],
    );
    assert!(output.status.success(), "{}", stdout(&output));
    assert!(
        std::fs::read_to_string(dir.path().join("docs/specs/2026.md"))
            .unwrap()
            .starts_with("---\nid: \"2026\"\n")
    );
}

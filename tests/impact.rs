//! `lore impact`가 고정 예제에서 영향 범위를 모으는 과정을 확인한다.

mod common;

use common::{json, lore_in, repo, stdout};
use serde_json::json;

fn ids(items: &serde_json::Value) -> Vec<&str> {
    items
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["id"].as_str().unwrap())
        .collect()
}

#[test]
fn 기본_홉_1은_대상에_의존하는_기능까지_닿는다() {
    let dir = repo("payment");
    let output = lore_in(dir.path(), &["impact", "payment-core"]);
    assert!(output.status.success(), "{}", stdout(&output));
    let report = json(&output);
    assert_eq!(report["hop"], 1);
    assert_eq!(report["edges"], json!(["depends_on", "belongs_to"]));
    assert_eq!(ids(&report["features"]), ["payment-core", "payment-retry"]);
    assert_eq!(
        report["features"][1]["path"],
        json!([{
            "src": "payment-retry", "dst": "payment-core",
            "kind": "depends_on", "origin": "frontmatter"
        }])
    );
}

#[test]
fn 홉을_늘리면_간접으로_의존하는_기능까지_닿는다() {
    let dir = repo("payment");
    let report = json(&lore_in(dir.path(), &["impact", "payment-core", "--hop=2"]));
    assert_eq!(
        ids(&report["features"]),
        ["payment-core", "payment-retry", "payment-notification"]
    );
    assert_eq!(report["features"][2]["hop"], 2);
    assert_eq!(report["features"][2]["path"].as_array().unwrap().len(), 2);
}

#[test]
fn 닿은_기능의_파일_테스트_결정_사고를_다섯_묶음으로_모은다() {
    let dir = repo("payment");
    let report = json(&lore_in(dir.path(), &["impact", "payment-core"]));
    assert_eq!(
        ids(&report["files"]),
        [
            "services/payment/core/charge.ts",
            "services/payment/retry/policy.ts",
            "services/payment/retry/queue.ts"
        ]
    );
    assert_eq!(
        ids(&report["tests"]),
        [
            "services/payment/core/charge.test.ts",
            "services/payment/retry/queue.test.ts"
        ]
    );
    // 대체된 ADR-20260918-retry-inline은 지켜야 할 결정이 아니다.
    assert_eq!(
        ids(&report["decisions"]),
        ["ADR-20261022-idempotency-key", "ADR-20261105-retry-queue"]
    );
    assert_eq!(ids(&report["incidents"]), ["INC-20261020-duplicate-charge"]);
    let last = report["tests"][1]["path"]
        .as_array()
        .unwrap()
        .last()
        .unwrap();
    assert_eq!(last["kind"], "verifies");
}

#[test]
fn 파일_경로는_그_파일의_기능에서_출발한다() {
    let dir = repo("payment");
    let report = json(&lore_in(
        dir.path(),
        &["impact", "services/payment/retry/queue.ts"],
    ));
    assert_eq!(report["target"]["kind"], "file");
    assert_eq!(
        ids(&report["features"]),
        ["payment-retry", "payment-notification"]
    );
    assert_eq!(report["features"][0]["path"][0]["kind"], "implements");
}

#[test]
fn 건널_엣지를_고르면_그_엣지로만_퍼진다() {
    let dir = repo("payment");
    let both = json(&lore_in(dir.path(), &["impact", "payment"]));
    assert_eq!(
        ids(&both["features"]),
        ["payment", "payment-core", "payment-retry"]
    );
    let depends = json(&lore_in(
        dir.path(),
        &["impact", "payment", "--edges=depends_on"],
    ));
    assert_eq!(ids(&depends["features"]), ["payment"]);
}

#[test]
fn 사람용_출력은_묶음마다_개수와_경로를_보여_준다() {
    let dir = repo("payment");
    let output = lore_in(dir.path(), &["impact", "payment-retry", "--for-human"]);
    assert_eq!(
        stdout(&output),
        "대상  payment-retry  (기능)

영향받는 기능  2개
  payment-retry         직접
  payment-notification  1홉  payment-notification 의존함 payment-retry (선언)

영향받는 파일  3개
  services/payment/retry/policy.ts      (payment-retry)
  services/payment/retry/queue.ts       (payment-retry)
  services/notification/retry-alert.ts  (payment-notification)

돌려야 할 테스트  1개
  services/payment/retry/queue.test.ts  (payment-retry)

지켜야 할 결정  2개
  ADR-20261022-idempotency-key  결제 요청에 멱등 키를 붙인다  (payment-retry)
  ADR-20261105-retry-queue      큐 기반 재시도로 전환, 최대 3회  (payment-retry)

관련 사고  1개
  INC-20261020-duplicate-charge  재시도 중 중복 결제 발생  (payment-retry)
"
    );
}

#[test]
fn 의존이_순환해도_끝난다() {
    let dir = repo("payment");
    let spec = dir.path().join("docs/specs/payment-core.md");
    let text = std::fs::read_to_string(&spec).unwrap();
    std::fs::write(
        &spec,
        text.replace("depends_on: []", "depends_on: [payment-notification]"),
    )
    .unwrap();
    let report = json(&lore_in(
        dir.path(),
        &["impact", "payment-core", "--hop=10"],
    ));
    assert_eq!(
        ids(&report["features"]),
        ["payment-core", "payment-retry", "payment-notification"]
    );
}

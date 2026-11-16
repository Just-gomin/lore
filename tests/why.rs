//! `lore why`가 고정 예제에서 결정과 사고를 조회하는 과정을 확인한다.

mod common;

use common::{json, lore_in, repo, stdout};

fn ids(records: &serde_json::Value) -> Vec<&str> {
    records
        .as_array()
        .unwrap()
        .iter()
        .map(|record| record["id"].as_str().unwrap())
        .collect()
}

#[test]
fn 파일_경로로_기능의_결정과_사고를_시간순으로_낸다() {
    let dir = repo("payment");
    let output = lore_in(dir.path(), &["why", "services/payment/retry/queue.ts"]);
    assert!(output.status.success(), "{}", stdout(&output));
    let report = json(&output);
    assert_eq!(report["target"]["kind"], "file");
    assert_eq!(report["features"].as_array().unwrap().len(), 1);
    let feature = &report["features"][0];
    assert_eq!(feature["id"], "payment-retry");
    assert_eq!(
        ids(&feature["decisions"]),
        [
            "ADR-20260918-retry-inline",
            "ADR-20261022-idempotency-key",
            "ADR-20261105-retry-queue"
        ]
    );
    assert_eq!(
        feature["decisions"][0]["superseded_by"],
        "ADR-20261105-retry-queue"
    );
    assert_eq!(
        ids(&feature["incidents"]),
        ["INC-20261020-duplicate-charge"]
    );
}

#[test]
fn 사람용_출력은_시나리오_문서의_형식을_따른다() {
    let dir = repo("payment");
    let output = lore_in(
        &dir.path().join("services/payment/retry"),
        &["why", "queue.ts", "--for-human"],
    );
    assert!(output.status.success());
    assert_eq!(
        stdout(&output),
        "기능  payment-retry  결제 재시도

결정
  2026-09-18  ADR-20260918-retry-inline     요청 안에서 즉시 재시도          [대체됨]
  2026-10-22  ADR-20261022-idempotency-key  결제 요청에 멱등 키를 붙인다     [유효]
  2026-11-05  ADR-20261105-retry-queue      큐 기반 재시도로 전환, 최대 3회  [유효]

사고
  2026-10-20  INC-20261020-duplicate-charge  재시도 중 중복 결제 발생

자세히: lore why ADR-20261105-retry-queue --depth doc
"
    );
}

#[test]
fn 테스트_파일은_검증하는_기능으로_찾는다() {
    let dir = repo("payment");
    let report = json(&lore_in(
        dir.path(),
        &["why", "services/payment/core/charge.test.ts"],
    ));
    assert_eq!(report["features"][0]["id"], "payment-core");
}

#[test]
fn 결정_아이디로_그_결정과_관계를_낸다() {
    let dir = repo("payment");
    let report = json(&lore_in(
        dir.path(),
        &["why", "ADR-20261022-idempotency-key"],
    ));
    assert_eq!(report["target"]["kind"], "decision");
    assert_eq!(report["features"], serde_json::json!([]));
    let record = &report["records"][0];
    assert_eq!(
        record["features"],
        serde_json::json!(["payment-core", "payment-retry"])
    );
    assert_eq!(
        record["related"],
        serde_json::json!(["INC-20261020-duplicate-charge"])
    );
    assert!(record.get("body").is_none());
}

#[test]
fn 깊이를_내리면_본문과_소스_노트가_붙는다() {
    let dir = repo("payment");
    let doc = json(&lore_in(
        dir.path(),
        &["why", "ADR-20261105-retry-queue", "--depth", "doc"],
    ));
    assert!(
        doc["records"][0]["body"]
            .as_str()
            .unwrap()
            .contains("최대 3회")
    );
    assert!(doc["records"][0].get("sources").is_none());

    let source = json(&lore_in(
        dir.path(),
        &["why", "ADR-20261105-retry-queue", "--depth", "source"],
    ));
    let record = &source["records"][0];
    assert_eq!(ids(&record["sources"]), ["SRC-20261103-retry-meeting"]);
    assert!(record["sources"][0]["body"].is_string());
    assert_eq!(
        record["links"],
        serde_json::json!(["https://github.com/org/repo/pull/231"])
    );
}

#[test]
fn 소스가_바뀌면_조회_전에_다시_빌드한다() {
    let dir = repo("payment");
    let first = json(&lore_in(dir.path(), &["why", "payment-retry"]));
    assert_eq!(first["rebuilt"], true);
    let second = json(&lore_in(dir.path(), &["why", "payment-retry"]));
    assert_eq!(second["rebuilt"], false);

    let spec = dir.path().join("docs/specs/payment-retry.md");
    let text = std::fs::read_to_string(&spec).unwrap();
    std::fs::write(
        &spec,
        text.replace("title: 결제 재시도", "title: 결제 다시 시도"),
    )
    .unwrap();
    let third = json(&lore_in(dir.path(), &["why", "payment-retry"]));
    assert_eq!(third["rebuilt"], true);
    assert_eq!(third["features"][0]["title"], "결제 다시 시도");
}

#[test]
fn 심볼은_6단계에서_지원한다는_오류를_낸다() {
    let dir = repo("payment");
    let output = lore_in(
        dir.path(),
        &["why", "services/payment/retry/queue.ts#enqueue"],
    );
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(json(&output)["error"]["code"], "unsupported_target");
}

#[test]
fn 없는_대상은_target_not_found다() {
    let dir = repo("payment");
    let output = lore_in(dir.path(), &["why", "README.md"]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(json(&output)["error"]["code"], "target_not_found");
}

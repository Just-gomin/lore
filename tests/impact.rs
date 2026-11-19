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
  payment-notification  1홉  선언  payment-notification 의존함 payment-retry (선언)

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

fn features(report: &serde_json::Value) -> Vec<(String, u64, String)> {
    report["features"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| {
            (
                item["id"].as_str().unwrap().to_string(),
                item["hop"].as_u64().unwrap(),
                item["evidence"].as_str().unwrap().to_string(),
            )
        })
        .collect()
}

fn feature(id: &str, hop: u64, evidence: &str) -> (String, u64, String) {
    (id.to_string(), hop, evidence.to_string())
}

#[test]
fn 선언하지_않은_의존이_추론으로_나오고_경로가_import한_파일을_보여_준다() {
    let dir = repo("analysis");
    let output = lore_in(dir.path(), &["impact", "payment-core"]);
    assert!(output.status.success(), "{}", stdout(&output));
    let report = json(&output);
    assert_eq!(
        features(&report),
        [
            feature("payment-core", 0, "declared"),
            feature("payment-notification", 1, "inferred"),
        ]
    );
    assert_eq!(
        report["features"][1]["path"],
        json!([
            {"src": "services/payment/core/index.ts", "dst": "payment-core", "kind": "implements", "origin": "frontmatter"},
            {"src": "services/notification/alert.ts", "dst": "services/payment/core/index.ts", "kind": "depends_on", "origin": "analysis"},
            {"src": "services/notification/alert.ts", "dst": "payment-notification", "kind": "implements", "origin": "frontmatter"},
        ])
    );
}

#[test]
fn 분석을_끄면_선언하지_않은_의존은_나오지_않는다() {
    let dir = repo("analysis");
    std::fs::write(
        dir.path().join("lore.config.yaml"),
        "version: 1\n\nproject:\n  docs: docs\n",
    )
    .unwrap();
    let report = json(&lore_in(dir.path(), &["impact", "payment-core"]));
    assert_eq!(features(&report), [feature("payment-core", 0, "declared")]);
}

#[test]
fn 의존함을_건너지_않으면_분석_엣지도_건너지_않는다() {
    let dir = repo("analysis");
    let report = json(&lore_in(
        dir.path(),
        &["impact", "payment-core", "--edges=belongs_to"],
    ));
    assert_eq!(features(&report), [feature("payment-core", 0, "declared")]);
}

#[test]
fn 파일이_대상이면_그_파일에_의존하는_파일의_기능만_한_홉이다() {
    let dir = repo("analysis");
    // types.ts는 format.ts가 직접 가져온다. 같은 기능의 index.ts를 거친 길은 세지 않는다.
    let report = json(&lore_in(
        dir.path(),
        &["impact", "services/payment/core/types.ts"],
    ));
    assert_eq!(
        features(&report),
        [
            feature("payment-core", 0, "declared"),
            feature("payment-notification", 1, "inferred"),
        ]
    );
    let path = report["features"][1]["path"].as_array().unwrap();
    assert_eq!(path[0]["src"], "services/notification/format.ts");
    assert_eq!(path[0]["dst"], "services/payment/core/types.ts");

    // charge.ts는 같은 기능의 index.ts가 다시 내보내고, 알림 기능은 index.ts를 거쳐 charge를 쓴다.
    // 파일의 심볼에 의존하는 것도 따라가므로 index.ts를 거친 길이 나온다.
    let report = json(&lore_in(
        dir.path(),
        &["impact", "services/payment/core/charge.ts"],
    ));
    assert_eq!(
        features(&report),
        [
            feature("payment-core", 0, "declared"),
            feature("payment-notification", 1, "inferred"),
        ]
    );
    let path = report["features"][1]["path"].as_array().unwrap();
    assert_eq!(path[1]["src"], "services/notification/alert.ts#alert");
    assert_eq!(path[1]["dst"], "services/payment/core/charge.ts#charge");
}

#[test]
fn 같은_기능의_다른_파일만_가져가는_기능은_나오지_않는다() {
    let dir = repo("analysis");
    let core = dir.path().join("services/payment/core");
    std::fs::write(core.join("util.ts"), "export const unit = 100;\n").unwrap();
    std::fs::write(
        core.join("helper.ts"),
        "import { unit } from \"./util\";\nexport const helper = () => unit;\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("services/notification/extra.ts"),
        "import { helper } from \"../payment/core/helper\";\nexport const extra = helper;\n",
    )
    .unwrap();
    common::git(dir.path(), &["add", "-A"]);
    common::git(dir.path(), &["commit", "-qm", "helper"]);
    let report = json(&lore_in(
        dir.path(),
        &["impact", "services/payment/core/util.ts"],
    ));
    assert_eq!(features(&report), [feature("payment-core", 0, "declared")]);
    let report = json(&lore_in(
        dir.path(),
        &["impact", "services/payment/core/helper.ts"],
    ));
    assert_eq!(
        features(&report),
        [
            feature("payment-core", 0, "declared"),
            feature("payment-notification", 1, "inferred"),
        ]
    );
}

#[test]
fn 스펙에_속하지_않은_파일도_의존하는_파일의_기능을_한_홉으로_낸다() {
    let dir = repo("analysis");
    let report = json(&lore_in(
        dir.path(),
        &["impact", "packages/money/src/lib/round.ts", "--hop=2"],
    ));
    // round.ts ← money/src/index.ts(스펙 밖) ← core/charge.ts
    assert_eq!(
        features(&report),
        [
            feature("payment-core", 1, "inferred"),
            feature("payment-notification", 2, "inferred"),
        ]
    );
    let files = report["files"].as_array().unwrap();
    let money = files
        .iter()
        .find(|item| item["id"] == "packages/money/src/index.ts")
        .unwrap();
    assert_eq!(money["hop"], 0);
    assert_eq!(money["evidence"], "inferred");
}

#[test]
fn 분석_엣지로_닿은_파일도_영향받는_파일이다() {
    let dir = repo("analysis");
    let report = json(&lore_in(
        dir.path(),
        &["impact", "services/notification/alert.ts"],
    ));
    assert!(ids(&report["files"]).contains(&"scripts/seed.mjs"));
    let text = stdout(&lore_in(
        dir.path(),
        &["impact", "services/notification/alert.ts", "--for-human"],
    ));
    assert!(
        text.contains("scripts/seed.mjs                 (의존함 services/notification/alert.ts)"),
        "{text}"
    );
}

#[test]
fn 심볼이_대상이면_그_심볼을_쓰는_파일의_기능을_낸다() {
    let dir = repo("analysis");
    let output = lore_in(
        dir.path(),
        &["impact", "services/payment/core/charge.ts#charge"],
    );
    assert!(output.status.success(), "{}", stdout(&output));
    let report = json(&output);
    assert_eq!(report["target"]["kind"], "symbol");
    assert_eq!(
        features(&report),
        [
            feature("payment-core", 0, "declared"),
            feature("payment-notification", 1, "inferred"),
        ]
    );
    // index.ts의 export *를 거쳐 가져간 alert가 가장 먼저다.
    let path = report["features"][1]["path"].as_array().unwrap();
    assert_eq!(path[0]["src"], "services/notification/alert.ts#alert");
    assert_eq!(path[0]["dst"], "services/payment/core/charge.ts#charge");

    // 레포 밖 경로나 현재 폴더 기준 경로로도 심볼을 받는다.
    let report = json(&lore_in(
        &dir.path().join("services/payment"),
        &["impact", "core/charge.ts#charge"],
    ));
    assert_eq!(
        report["target"]["id"],
        "services/payment/core/charge.ts#charge"
    );
}

#[test]
fn 한_기능에_선언과_추론_경로가_둘_다_있으면_둘_다_낸다() {
    let dir = repo("analysis");
    let spec = dir.path().join("docs/specs/payment-notification.md");
    let text = std::fs::read_to_string(&spec).unwrap();
    std::fs::write(
        &spec,
        text.replace("depends_on: []", "depends_on:\n  - payment-core"),
    )
    .unwrap();
    let report = json(&lore_in(dir.path(), &["impact", "payment-core"]));
    assert_eq!(
        features(&report),
        [
            feature("payment-core", 0, "declared"),
            feature("payment-notification", 1, "declared"),
            feature("payment-notification", 1, "inferred"),
        ]
    );
}

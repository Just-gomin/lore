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
fn 분석하지_않는_레포의_심볼은_target_not_found다() {
    let dir = repo("payment");
    let output = lore_in(
        dir.path(),
        &["why", "services/payment/retry/queue.ts#enqueue"],
    );
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(json(&output)["error"]["code"], "target_not_found");
}

#[test]
fn 없는_대상은_target_not_found다() {
    let dir = repo("payment");
    let output = lore_in(dir.path(), &["why", "README.md"]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(json(&output)["error"]["code"], "target_not_found");
}

fn write(dir: &std::path::Path, path: &str, text: &str) {
    let path = dir.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn decision(id: &str, extra: &str, body: &str) -> String {
    format!(
        "---\nid: {id}\ntitle: {id}\nstatus: accepted\nspecs: [payment-core]\nsources: []\nconfidence: stated\n{extra}---\n\n## 결정\n\n{body}\n"
    )
}

/// 분석용 고정 예제에 결제 처리의 결정 넷과 사고 둘을 더한다.
fn analysis_with_records() -> tempfile::TempDir {
    let dir = repo("analysis");
    let root = dir.path();
    write(
        root,
        "docs/decisions/ADR-20261101-first.md",
        &decision(
            "ADR-20261101-first",
            "",
            "`charge`는 카드사에 한 번만 요청합니다.",
        ),
    );
    write(
        root,
        "docs/decisions/ADR-20261102-types.md",
        &decision(
            "ADR-20261102-types",
            "",
            "금액 타입은 services/payment/core/types.ts에 모읍니다.",
        ),
    );
    write(
        root,
        "docs/decisions/ADR-20261103-words.md",
        &decision(
            "ADR-20261103-words",
            "",
            "recharge와 charges는 다른 기능입니다.",
        ),
    );
    write(
        root,
        "docs/decisions/ADR-20261104-second.md",
        &decision(
            "ADR-20261104-second",
            "supersedes: ADR-20261101-first\n",
            "charge()는 실패하면 큐에 넣습니다.",
        ),
    );
    for (id, body) in [
        ("INC-20261001-timeout", "카드사 응답이 늦었습니다."),
        (
            "INC-20261002-double",
            "services/payment/core/charge.ts가 두 번 불렸습니다.",
        ),
    ] {
        write(
            root,
            &format!("docs/incidents/{id}.md"),
            &format!(
                "---\nid: {id}\ntitle: {id}\nspecs: [payment-core]\ndecisions: []\nsources: []\n---\n\n## 무슨 일이 있었나\n\n{body}\n"
            ),
        );
    }
    // 세 글자 미만 이름의 심볼
    write(
        root,
        "services/payment/core/id.ts",
        "export const id = 1;\n",
    );
    write(
        root,
        "docs/decisions/ADR-20261105-id.md",
        &decision(
            "ADR-20261105-id",
            "",
            "결제마다 `id`를 하나씩 붙입니다. ID와 uuid는 쓰지 않습니다.",
        ),
    );
    common::git(root, &["add", "-A"]);
    common::git(root, &["commit", "-qm", "records"]);
    dir
}

#[test]
fn 세_글자_미만_이름도_언급으로_찾는다() {
    let dir = analysis_with_records();
    let report = json(&lore_in(
        dir.path(),
        &["why", "services/payment/core/id.ts#id"],
    ));
    let decisions = &report["features"][0]["decisions"];
    assert_eq!(decisions[0]["id"], "ADR-20261105-id");
    assert_eq!(decisions[0]["mentioned"], serde_json::json!(["id"]));
    assert!(decisions[1].get("mentioned").is_none());
}

#[test]
fn 심볼은_담은_파일의_스펙으로_답하고_언급된_기록을_위로_올린다() {
    let dir = analysis_with_records();
    let output = lore_in(
        dir.path(),
        &["why", "services/payment/core/charge.ts#charge"],
    );
    assert!(output.status.success(), "{}", common::stdout(&output));
    let report = json(&output);
    assert_eq!(report["target"]["kind"], "symbol");
    assert_eq!(
        report["target"]["mentions"],
        serde_json::json!([
            "charge",
            "services/payment/core/charge.ts#charge",
            "services/payment/core/charge.ts"
        ])
    );
    let feature = &report["features"][0];
    assert_eq!(feature["id"], "payment-core");
    // 대체된 first는 언급되었지만 올리지 않는다. words의 recharge, charges는 낱말의 일부라 언급이 아니다.
    assert_eq!(
        ids(&feature["decisions"]),
        [
            "ADR-20261104-second",
            "ADR-20261101-first",
            "ADR-20261102-types",
            "ADR-20261103-words",
            "ADR-20261105-id"
        ]
    );
    assert_eq!(
        feature["decisions"][0]["mentioned"],
        serde_json::json!(["charge"])
    );
    assert_eq!(
        feature["decisions"][1]["mentioned"],
        serde_json::json!(["charge"])
    );
    assert!(feature["decisions"][3].get("mentioned").is_none());
    assert!(feature["decisions"][4].get("mentioned").is_none());
    assert_eq!(
        ids(&feature["incidents"]),
        ["INC-20261002-double", "INC-20261001-timeout"]
    );

    let text = common::stdout(&lore_in(
        dir.path(),
        &[
            "why",
            "services/payment/core/charge.ts#charge",
            "--for-human",
        ],
    ));
    assert!(text.contains("[유효]    언급됨(추론)"), "{text}");
    assert!(text.contains("[대체됨]  언급됨(추론)"), "{text}");
}

#[test]
fn 파일_경로로_물으면_경로가_언급된_기록을_위로_올린다() {
    let dir = analysis_with_records();
    let report = json(&lore_in(
        dir.path(),
        &["why", "services/payment/core/types.ts"],
    ));
    assert_eq!(
        ids(&report["features"][0]["decisions"]),
        [
            "ADR-20261102-types",
            "ADR-20261101-first",
            "ADR-20261103-words",
            "ADR-20261104-second",
            "ADR-20261105-id"
        ]
    );
}

#[test]
fn 기능으로_물으면_시간순_그대로다() {
    let dir = analysis_with_records();
    let report = json(&lore_in(dir.path(), &["why", "payment-core"]));
    assert_eq!(
        ids(&report["features"][0]["decisions"]),
        [
            "ADR-20261101-first",
            "ADR-20261102-types",
            "ADR-20261103-words",
            "ADR-20261104-second",
            "ADR-20261105-id"
        ]
    );
    assert!(report["target"].get("mentions").is_none());
}

#[test]
fn 스펙에_속하지_않은_파일의_심볼은_스펙이_없다고_알린다() {
    let dir = repo("analysis");
    let output = lore_in(
        dir.path(),
        &[
            "why",
            "packages/money/src/lib/round.ts#round",
            "--for-human",
        ],
    );
    assert!(output.status.success());
    assert!(
        common::stdout(&output).contains("이 심볼을 담은 파일을 맡을 스펙"),
        "{}",
        common::stdout(&output)
    );
}

#[test]
fn 기본_내보내기는_이름으로_찾지_않고_멤버는_클래스_이름과_함께_찾는다() {
    let dir = repo("analysis");
    for (id, body) in [
        ("ADR-20261101-word", "default 값은 비워 둡니다."),
        ("ADR-20261102-send", "Sender.send는 한 번만 보냅니다."),
    ] {
        write(
            dir.path(),
            &format!("docs/decisions/{id}.md"),
            &decision(id, "", body).replace("payment-core", "payment-notification"),
        );
    }
    common::git(dir.path(), &["add", "-A"]);
    common::git(dir.path(), &["commit", "-qm", "records"]);

    let report = json(&lore_in(
        dir.path(),
        &["why", "services/notification/sender.ts#default"],
    ));
    assert_eq!(
        report["target"]["mentions"],
        serde_json::json!([
            "services/notification/sender.ts#default",
            "services/notification/sender.ts"
        ])
    );
    let decisions = &report["features"][0]["decisions"];
    assert!(decisions[0].get("mentioned").is_none());
    assert!(decisions[1].get("mentioned").is_none());

    let report = json(&lore_in(
        dir.path(),
        &["why", "services/notification/sender.ts#Sender.send"],
    ));
    let decisions = &report["features"][0]["decisions"];
    assert_eq!(decisions[0]["id"], "ADR-20261102-send");
    assert_eq!(
        decisions[0]["mentioned"],
        serde_json::json!(["Sender.send"])
    );
}

#[test]
fn 자세히_안내는_위로_올린_기록을_먼저_가리킨다() {
    let dir = analysis_with_records();
    let text = common::stdout(&lore_in(
        dir.path(),
        &["why", "services/payment/core/types.ts", "--for-human"],
    ));
    assert!(
        text.contains("자세히: lore why ADR-20261102-types --depth doc"),
        "{text}"
    );
}

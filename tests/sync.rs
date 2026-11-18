//! `lore sync`를 고정 예제로 확인한다.

mod common;

use std::path::Path;

use common::{git, json, lore_in, repo, stdout};
use serde_json::Value;
use tempfile::TempDir;

/// 고정 예제에서 `feature` 브랜치를 만든다. `main`이 비교 기준이다.
fn feature_repo() -> TempDir {
    let dir = repo("payment");
    git(dir.path(), &["checkout", "-q", "-b", "feature"]);
    dir
}

fn move_file(dir: &Path, from: &str, to: &str) {
    std::fs::create_dir_all(dir.join(to).parent().unwrap()).unwrap();
    git(dir, &["mv", from, to]);
    git(dir, &["commit", "-q", "-m", "move"]);
}

fn read(dir: &Path, path: &str) -> String {
    std::fs::read_to_string(dir.join(path)).unwrap()
}

fn paths_status(dir: &Path) -> String {
    let report = json(&lore_in(dir, &["check"]));
    report["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|check| check["id"] == "paths")
        .unwrap()["status"]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn 글자_그대로_적힌_경로의_이름을_바꾸면_항목을_고치고_경로_검사가_통과한다() {
    let dir = feature_repo();
    move_file(
        dir.path(),
        "services/payment/index.ts",
        "services/payment/main.ts",
    );
    assert_eq!(paths_status(dir.path()), "fail");
    let before = read(dir.path(), "docs/specs/payment.md");

    let output = lore_in(dir.path(), &["sync", "--base", "main"]);
    assert_eq!(output.status.code(), Some(0), "{}", stdout(&output));
    let report = json(&output);
    assert_eq!(report["ok"], true);
    assert_eq!(report["dry_run"], false);
    assert_eq!(
        report["fixed"],
        serde_json::json!([{
            "spec": "payment",
            "path": "docs/specs/payment.md",
            "field": "paths",
            "from": "services/payment/index.ts",
            "to": "services/payment/main.ts",
        }])
    );
    // 머리말의 그 항목만 바뀐다.
    assert_eq!(
        read(dir.path(), "docs/specs/payment.md"),
        before.replace(
            "  - services/payment/index.ts\n",
            "  - services/payment/main.ts\n"
        )
    );
    assert_eq!(paths_status(dir.path()), "pass");
    // 본문은 고치지 않고 확인할 스펙으로 낸다.
    assert_eq!(
        report["specs_to_review"],
        serde_json::json!([{
            "spec": "payment",
            "path": "docs/specs/payment.md",
            "files": ["services/payment/index.ts"],
        }])
    );
    // 결정 기록은 만들지 않는다.
    assert_eq!(
        git(dir.path(), &["status", "--porcelain"]),
        " M docs/specs/payment.md\n"
    );
}

#[test]
fn dry_run은_고칠_내용만_내고_파일을_고치지_않는다() {
    let dir = feature_repo();
    move_file(
        dir.path(),
        "services/payment/index.ts",
        "services/payment/main.ts",
    );
    let report = json(&lore_in(
        dir.path(),
        &["sync", "--base", "main", "--dry-run"],
    ));
    assert_eq!(report["dry_run"], true);
    assert_eq!(report["fixed"].as_array().unwrap().len(), 1);
    assert_eq!(git(dir.path(), &["status", "--porcelain"]), "");
}

#[test]
fn 글롭으로_맞던_파일을_글롭_밖으로_옮기면_확인할_목록에_나온다() {
    let dir = feature_repo();
    move_file(
        dir.path(),
        "services/payment/retry/policy.ts",
        "services/billing/policy.ts",
    );
    let report = json(&lore_in(dir.path(), &["sync", "--base", "main"]));
    assert_eq!(report["fixed"], serde_json::json!([]));
    assert_eq!(
        report["to_check"],
        serde_json::json!([{
            "spec": "payment-retry",
            "path": "docs/specs/payment-retry.md",
            "field": "paths",
            "entry": "services/payment/retry/**",
            "from": "services/payment/retry/policy.ts",
            "to": "services/billing/policy.ts",
            "reason": "glob",
        }])
    );
    // 글롭 항목은 고치지 않는다.
    assert_eq!(git(dir.path(), &["status", "--porcelain"]), "");
}

#[test]
fn 글롭_안에서_옮긴_파일은_확인할_목록에_없다() {
    let dir = feature_repo();
    move_file(
        dir.path(),
        "services/payment/retry/policy.ts",
        "services/payment/retry/backoff/policy.ts",
    );
    let report = json(&lore_in(dir.path(), &["sync", "--base", "main"]));
    assert_eq!(report["to_check"], serde_json::json!([]));
    let specs: Vec<Value> = report["specs_to_review"]
        .as_array()
        .unwrap()
        .iter()
        .map(|review| review["spec"].clone())
        .collect();
    assert_eq!(specs, ["payment-retry"]);
}

#[test]
fn 본문을_함께_고친_스펙은_확인할_스펙에서_뺀다() {
    let dir = feature_repo();
    let policy = dir.path().join("services/payment/retry/policy.ts");
    let code = std::fs::read_to_string(&policy).unwrap();
    std::fs::write(&policy, format!("{code}// 간격 조정\n")).unwrap();
    let charge = dir.path().join("services/payment/core/charge.ts");
    let code = std::fs::read_to_string(&charge).unwrap();
    std::fs::write(&charge, format!("{code}// 주석\n")).unwrap();
    let spec = dir.path().join("docs/specs/payment-retry.md");
    let text = std::fs::read_to_string(&spec).unwrap();
    std::fs::write(&spec, format!("{text}\n간격을 늘려 가며 재시도합니다.\n")).unwrap();

    let report = json(&lore_in(dir.path(), &["sync", "--base", "main"]));
    assert_eq!(
        report["specs_to_review"],
        serde_json::json!([{
            "spec": "payment-core",
            "path": "docs/specs/payment-core.md",
            "files": ["services/payment/core/charge.ts"],
        }])
    );
}

#[test]
fn 사람용_출력은_고친_항목과_확인할_것을_보여_준다() {
    let dir = feature_repo();
    move_file(
        dir.path(),
        "services/payment/index.ts",
        "services/payment/main.ts",
    );
    let output = lore_in(dir.path(), &["sync", "--base", "main", "--for-human"]);
    assert_eq!(
        stdout(&output),
        "lore sync: 고친 항목 1, 확인할 항목 0, 본문을 확인할 스펙 1

고친 항목
  docs/specs/payment.md [paths] services/payment/index.ts → services/payment/main.ts

본문을 확인할 스펙
  payment (docs/specs/payment.md): services/payment/index.ts
"
    );
}

#[test]
fn 비교_기준이_없으면_사용법_오류다() {
    let dir = feature_repo();
    let output = lore_in(dir.path(), &["sync"]);
    assert_eq!(output.status.code(), Some(2));
}

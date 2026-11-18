//! `lore check`의 기계적 검사와 판단이 드는 검사를 고정 예제로 확인한다.

mod common;

use std::path::Path;

use common::{git, json, lore_in, repo, stdout};
use serde_json::Value;
use tempfile::TempDir;

/// 고정 예제에 안내문을 설치하고 커밋해, 모든 검사가 통과하는 레포를 만든다.
fn clean_repo() -> TempDir {
    let dir = repo("payment");
    assert!(
        lore_in(dir.path(), &["install", "--no-skills"])
            .status
            .success()
    );
    git(dir.path(), &["add", "-A"]);
    git(dir.path(), &["commit", "-q", "-m", "lore install"]);
    dir
}

fn status_of(report: &Value, id: &str) -> String {
    check_of(report, id)["status"].as_str().unwrap().to_string()
}

fn check_of<'a>(report: &'a Value, id: &str) -> &'a Value {
    report["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|check| check["id"] == id)
        .unwrap_or_else(|| panic!("{id} 검사가 없습니다: {report}"))
}

fn codes(report: &Value, id: &str) -> Vec<String> {
    check_of(report, id)["problems"]
        .as_array()
        .map(|problems| {
            problems
                .iter()
                .map(|p| p["code"].as_str().unwrap().to_string())
                .collect()
        })
        .unwrap_or_default()
}

fn edit(dir: &Path, path: &str, from: &str, to: &str) {
    let file = dir.join(path);
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains(from), "{path}에 {from:?}가 없습니다");
    std::fs::write(file, text.replace(from, to)).unwrap();
}

#[test]
fn 어긋난_곳이_없으면_모든_검사가_통과한다() {
    let dir = clean_repo();
    let output = lore_in(dir.path(), &["check"]);
    assert_eq!(output.status.code(), Some(0), "{}", stdout(&output));
    let report = json(&output);
    assert_eq!(report["ok"], true);
    for id in ["paths", "references", "frontmatter", "index", "guide"] {
        assert_eq!(status_of(&report, id), "pass", "{id}");
        assert_eq!(check_of(&report, id)["blocking"], true);
    }
    // 검사는 파일을 고치지 않는다.
    assert_eq!(git(dir.path(), &["status", "--porcelain"]), "");
}

#[test]
fn 없는_파일을_가리키는_경로는_경로_유효성_실패다() {
    let dir = clean_repo();
    edit(
        dir.path(),
        "docs/specs/payment-notification.md",
        "services/notification/**",
        "services/sms/**",
    );
    let output = lore_in(dir.path(), &["check"]);
    assert_eq!(output.status.code(), Some(1));
    let report = json(&output);
    assert_eq!(report["ok"], false);
    assert_eq!(status_of(&report, "paths"), "fail");
    assert_eq!(codes(&report, "paths"), ["no_matching_files"]);
    assert!(check_of(&report, "paths")["fix"].is_string());
    assert_eq!(status_of(&report, "references"), "pass");
    // 소스에 문제가 있으면 인덱스는 다시 만들 수 없어 건너뛴다.
    assert_eq!(status_of(&report, "index"), "skip");
    assert!(check_of(&report, "index")["reason"].is_string());
}

#[test]
fn 없는_아이디_참조는_참조_유효성_실패다() {
    let dir = clean_repo();
    edit(
        dir.path(),
        "docs/decisions/ADR-20261110-log-format.md",
        "specs: []",
        "specs: [payment-sms]",
    );
    let report = json(&lore_in(dir.path(), &["check"]));
    assert_eq!(status_of(&report, "references"), "fail");
    assert_eq!(codes(&report, "references"), ["unknown_reference"]);
    assert_eq!(status_of(&report, "paths"), "pass");
    assert_eq!(status_of(&report, "frontmatter"), "pass");
}

#[test]
fn 빠진_필수_항목은_머리말_형식_실패다() {
    let dir = clean_repo();
    edit(
        dir.path(),
        "docs/decisions/ADR-20261110-log-format.md",
        "status: proposed\n",
        "",
    );
    let report = json(&lore_in(dir.path(), &["check"]));
    assert_eq!(status_of(&report, "frontmatter"), "fail");
    assert_eq!(codes(&report, "frontmatter"), ["missing_field"]);
}

#[test]
fn 인덱스가_소스와_다르면_실패하고_고치지_않는다() {
    let dir = clean_repo();
    edit(
        dir.path(),
        "docs/specs/payment.md",
        "title: 결제\n",
        "title: 결제 진입점\n",
    );
    let before = std::fs::read_to_string(dir.path().join("docs/INDEX.md")).unwrap();
    let output = lore_in(dir.path(), &["check"]);
    assert_eq!(output.status.code(), Some(1));
    let report = json(&output);
    assert_eq!(status_of(&report, "index"), "fail");
    assert_eq!(codes(&report, "index"), ["index_outdated"]);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("docs/INDEX.md")).unwrap(),
        before
    );

    // 안내한 대로 lore build를 실행하면 통과한다.
    assert!(lore_in(dir.path(), &["build"]).status.success());
    assert_eq!(
        status_of(&json(&lore_in(dir.path(), &["check"])), "index"),
        "pass"
    );

    std::fs::remove_file(dir.path().join("docs/INDEX.md")).unwrap();
    let report = json(&lore_in(dir.path(), &["check"]));
    assert_eq!(codes(&report, "index"), ["index_missing"]);
}

#[test]
fn 안내문_구간이_없거나_다르면_실패한다() {
    let dir = repo("payment");
    let report = json(&lore_in(dir.path(), &["check"]));
    assert_eq!(codes(&report, "guide"), ["guide_missing"]);

    assert!(
        lore_in(dir.path(), &["install", "--no-skills"])
            .status
            .success()
    );
    assert_eq!(
        status_of(&json(&lore_in(dir.path(), &["check"])), "guide"),
        "pass"
    );

    edit(dir.path(), "AGENTS.md", "### 원칙", "### 우리 원칙");
    let report = json(&lore_in(dir.path(), &["check"]));
    assert_eq!(codes(&report, "guide"), ["guide_outdated"]);

    edit(dir.path(), "AGENTS.md", "<!-- lore:end -->\n", "");
    let report = json(&lore_in(dir.path(), &["check"]));
    assert_eq!(codes(&report, "guide"), ["broken_marker"]);
}

#[test]
fn 도구별_파일은_있을_때만_구간을_본다() {
    let dir = clean_repo();
    std::fs::write(dir.path().join("CLAUDE.md"), "# 규칙\n").unwrap();
    let output = lore_in(dir.path(), &["check"]);
    assert_eq!(output.status.code(), Some(1));
    let report = json(&output);
    let problems = check_of(&report, "guide")["problems"].as_array().unwrap();
    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0]["path"], "CLAUDE.md");
    assert_eq!(problems[0]["code"], "guide_missing");

    assert!(
        lore_in(dir.path(), &["install", "--no-skills"])
            .status
            .success()
    );
    assert_eq!(
        status_of(&json(&lore_in(dir.path(), &["check"])), "guide"),
        "pass"
    );
}

#[test]
fn 사람용_출력은_실패를_먼저_보여_주고_표준_출력으로_낸다() {
    let dir = clean_repo();
    std::fs::remove_file(dir.path().join("docs/INDEX.md")).unwrap();
    let output = lore_in(dir.path(), &["check", "--for-human"]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        stdout(&output),
        "lore check: 실패 1, 건너뜀 3, 통과 4

실패  인덱스 일치
  docs/INDEX.md 인덱스 파일이 없습니다.
  고치려면: lore build를 실행하고 바뀐 인덱스를 커밋하세요.

건너뜀  소스 노트 신선도
  --origins로 원본의 최신 판 목록을 주지 않았습니다.

건너뜀  스펙 동반 갱신
  --base로 비교 기준을 주지 않았습니다.

건너뜀  매핑 누락
  --base로 비교 기준을 주지 않았습니다.

통과  경로 유효성

통과  참조 유효성

통과  머리말 형식

통과  안내문 일치
"
    );
}

#[test]
fn 잘못된_고정_예제는_문제를_검사별로_나눈다() {
    let dir = repo("invalid");
    let output = lore_in(dir.path(), &["check"]);
    assert_eq!(output.status.code(), Some(1));
    let report = json(&output);
    // 소스 읽기의 문제 17개에, lore build라면 거기서 멈춰 보지 못했을 경로 문제 1개가 더해진다.
    for (id, count) in [("paths", 1), ("references", 6), ("frontmatter", 11)] {
        assert_eq!(status_of(&report, id), "fail", "{id}");
        assert_eq!(codes(&report, id).len(), count, "{id}");
    }
}

const THREAD: (&str, &str) = (
    "https://example.slack.com/archives/C1/p1",
    "2026-10-20T18:00:00+09:00",
);
const MEETING: (&str, &str) = (
    "https://drive.google.com/file/d/retry-meeting",
    "2026-11-03T10:20:00+09:00",
);

/// 원본의 최신 판 목록을 파일로 쓰고 `lore check --origins`를 실행한다.
fn check_with_origins(dir: &Path, origins: &[(&str, &str)]) -> (Option<i32>, Value) {
    let entries: Vec<Value> = origins
        .iter()
        .map(|(origin, version)| serde_json::json!({"origin": origin, "origin_version": version}))
        .collect();
    let file = dir.join(".lore-origins.json");
    std::fs::write(&file, Value::Array(entries).to_string()).unwrap();
    let output = lore_in(dir, &["check", "--origins", ".lore-origins.json"]);
    std::fs::remove_file(file).unwrap();
    (output.status.code(), json(&output))
}

#[test]
fn 원본_목록을_주지_않으면_신선도는_건너뛴다() {
    let dir = clean_repo();
    let output = lore_in(dir.path(), &["check"]);
    assert_eq!(output.status.code(), Some(0));
    let report = json(&output);
    assert_eq!(status_of(&report, "freshness"), "skip");
    assert!(check_of(&report, "freshness")["reason"].is_string());
}

#[test]
fn 원본의_판이_소스_노트와_같으면_신선도_통과다() {
    let dir = clean_repo();
    let (code, report) = check_with_origins(dir.path(), &[THREAD, MEETING]);
    assert_eq!(code, Some(0));
    assert_eq!(status_of(&report, "freshness"), "pass");
    assert!(check_of(&report, "freshness").get("unchecked").is_none());
}

#[test]
fn 원본의_판이_바뀌었으면_신선도_실패다() {
    let dir = clean_repo();
    let newer = (MEETING.0, "2026-11-12T09:00:00+09:00");
    let (code, report) = check_with_origins(dir.path(), &[THREAD, newer]);
    assert_eq!(code, Some(1));
    assert_eq!(status_of(&report, "freshness"), "fail");
    let problems = check_of(&report, "freshness")["problems"]
        .as_array()
        .unwrap();
    assert_eq!(problems.len(), 1);
    assert_eq!(
        problems[0]["path"],
        "docs/sources/SRC-20261103-retry-meeting.md"
    );
    assert_eq!(problems[0]["field"], "origin_version");
    assert_eq!(problems[0]["code"], "stale_source");
}

#[test]
fn 목록에_없는_원본은_건수만_알리고_넘어간다() {
    let dir = clean_repo();
    let (code, report) = check_with_origins(dir.path(), &[THREAD]);
    assert_eq!(code, Some(0));
    assert_eq!(status_of(&report, "freshness"), "pass");
    assert_eq!(check_of(&report, "freshness")["unchecked"], 1);
}

#[test]
fn 원본_목록의_형식이_틀리면_invalid_input이다() {
    let dir = clean_repo();
    std::fs::write(dir.path().join("o.json"), r#"{"origin": "a"}"#).unwrap();
    let output = lore_in(dir.path(), &["check", "--origins", "o.json"]);
    assert_eq!(output.status.code(), Some(1));
    let report = json(&output);
    assert_eq!(report["error"]["code"], "invalid_input");
    assert_eq!(report["error"]["problems"][0]["path"], "o.json");

    let (_, report) = check_with_origins(dir.path(), &[THREAD, (THREAD.0, "다른 판")]);
    assert_eq!(report["error"]["problems"][0]["code"], "duplicate_origin");
}

/// 고정 예제에서 `feature` 브랜치를 만들어, `main`을 비교 기준으로 쓸 수 있게 한다.
fn feature_repo() -> TempDir {
    let dir = clean_repo();
    git(dir.path(), &["checkout", "-q", "-b", "feature"]);
    dir
}

fn commit_all(dir: &Path, message: &str) {
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", message]);
}

/// `--base main`으로 검사한다. PR 설명이 있으면 파일로 써서 `--pr-body`로 넘긴다.
fn check_against_main(dir: &Path, pr_body: Option<&str>) -> (Option<i32>, Value) {
    let mut args = vec!["check", "--base", "main"];
    if let Some(body) = pr_body {
        // 워크플로처럼 레포 밖에 두어 바뀐 파일에 끼지 않게 한다.
        let file = dir.join(".git/lore-pr-body.md");
        std::fs::write(file, body).unwrap();
        args.extend(["--pr-body", ".git/lore-pr-body.md"]);
    }
    let output = lore_in(dir, &args);
    (output.status.code(), json(&output))
}

fn findings(report: &Value, id: &str) -> Vec<Value> {
    check_of(report, id)["findings"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

#[test]
fn 비교_기준이_없으면_판단이_드는_검사는_건너뛰고_막지_않는다() {
    let dir = clean_repo();
    let report = json(&lore_in(dir.path(), &["check"]));
    for id in ["spec_update", "unmapped"] {
        assert_eq!(status_of(&report, id), "skip", "{id}");
        assert_eq!(check_of(&report, id)["blocking"], false);
        assert!(check_of(&report, id)["reason"].is_string());
    }
}

#[test]
fn 코드만_바꾸면_스펙_동반_갱신_경고이고_병합을_막지_않는다() {
    let dir = feature_repo();
    edit(
        dir.path(),
        "services/payment/retry/policy.ts",
        "export",
        "// 재시도 간격 조정\nexport",
    );
    commit_all(dir.path(), "재시도 정책 수정");
    let (code, report) = check_against_main(dir.path(), None);
    assert_eq!(code, Some(0));
    assert_eq!(report["ok"], true);
    assert_eq!(status_of(&report, "spec_update"), "warn");
    assert!(check_of(&report, "spec_update")["fix"].is_string());
    assert_eq!(
        findings(&report, "spec_update"),
        [serde_json::json!({
            "spec": "payment-retry",
            "files": ["services/payment/retry/policy.ts"],
            "outcome": "warned",
        })]
    );
    // 기존 파일을 고친 것이라 매핑 누락은 아니다.
    assert_eq!(status_of(&report, "unmapped"), "pass");
}

#[test]
fn 스펙도_함께_바꾸면_스펙_동반_갱신은_통과다() {
    let dir = feature_repo();
    edit(
        dir.path(),
        "services/payment/retry/policy.ts",
        "export",
        "// 재시도 간격 조정\nexport",
    );
    edit(
        dir.path(),
        "docs/specs/payment-retry.md",
        "최대 3회까지",
        "간격을 늘려 가며 최대 3회까지",
    );
    // 커밋하지 않은 변경도 비교 범위에 든다.
    let (_, report) = check_against_main(dir.path(), None);
    assert_eq!(status_of(&report, "spec_update"), "pass");
    let found = findings(&report, "spec_update");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0]["outcome"], "updated");
}

#[test]
fn no_impact_줄을_준_스펙은_경고_없이_넘기고_사유를_남긴다() {
    let dir = feature_repo();
    edit(
        dir.path(),
        "services/payment/retry/policy.ts",
        "export",
        "// 주석만 고침\nexport",
    );
    edit(
        dir.path(),
        "services/notification/retry-alert.ts",
        "export",
        "// 주석만 고침\nexport",
    );
    commit_all(dir.path(), "주석 정리");
    let body = "주석을 정리합니다.\r\n\r\nlore: no-impact payment-retry 주석만 바꿈\r\n";
    let (code, report) = check_against_main(dir.path(), Some(body));
    assert_eq!(code, Some(0));
    assert_eq!(status_of(&report, "spec_update"), "warn");
    assert_eq!(
        findings(&report, "spec_update"),
        [
            serde_json::json!({
                "spec": "payment-notification",
                "files": ["services/notification/retry-alert.ts"],
                "outcome": "warned",
            }),
            serde_json::json!({
                "spec": "payment-retry",
                "files": ["services/payment/retry/policy.ts"],
                "outcome": "no_impact",
                "reason": "주석만 바꿈",
            }),
        ]
    );

    let body = "lore: no-impact payment-retry 주석만 바꿈\nlore: no-impact payment-notification 주석만 바꿈\n";
    let (_, report) = check_against_main(dir.path(), Some(body));
    assert_eq!(status_of(&report, "spec_update"), "pass");
}

#[test]
fn 비교_기준은_merge_base라서_기준_브랜치의_이후_커밋은_보지_않는다() {
    let dir = clean_repo();
    git(dir.path(), &["checkout", "-q", "-b", "feature"]);
    edit(
        dir.path(),
        "services/payment/retry/policy.ts",
        "export",
        "// 재시도 간격 조정\nexport",
    );
    commit_all(dir.path(), "재시도 정책 수정");
    git(dir.path(), &["checkout", "-q", "main"]);
    edit(
        dir.path(),
        "services/payment/core/charge.ts",
        "export",
        "// 다른 PR\nexport",
    );
    commit_all(dir.path(), "다른 PR");
    git(dir.path(), &["checkout", "-q", "feature"]);

    let (_, report) = check_against_main(dir.path(), None);
    let specs: Vec<Value> = findings(&report, "spec_update")
        .iter()
        .map(|finding| finding["spec"].clone())
        .collect();
    assert_eq!(specs, ["payment-retry"]);
}

#[test]
fn 지운_파일은_지우기_전_경로로_스펙을_찾는다() {
    let dir = feature_repo();
    git(
        dir.path(),
        &["rm", "-q", "services/payment/retry/policy.ts"],
    );
    commit_all(dir.path(), "정책 파일 삭제");
    let (_, report) = check_against_main(dir.path(), None);
    let found = findings(&report, "spec_update");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0]["spec"], "payment-retry");
    assert_eq!(found[0]["files"][0], "services/payment/retry/policy.ts");
}

#[test]
fn 스펙에_맞지_않는_새_파일은_매핑_누락_경고다() {
    let dir = feature_repo();
    let write = |path: &str| {
        let file = dir.path().join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, "export {};\n").unwrap();
    };
    write("services/billing/invoice.ts");
    write("services/payment/retry/backoff.ts");
    write("docs/plans/next.md");
    write(".claude/skills/lore-record/SKILL.md");
    commit_all(dir.path(), "새 파일");
    // git이 추적하지 않는 파일은 보지 않는다.
    write("services/billing/draft.ts");

    let (code, report) = check_against_main(dir.path(), None);
    assert_eq!(code, Some(0));
    assert_eq!(status_of(&report, "unmapped"), "warn");
    assert!(check_of(&report, "unmapped")["fix"].is_string());
    assert_eq!(
        findings(&report, "unmapped"),
        [serde_json::json!({
            "files": ["services/billing/invoice.ts"],
            "outcome": "warned",
        })]
    );
}

#[test]
fn 이름을_바꾼_파일은_새_경로로_매핑을_본다() {
    let dir = feature_repo();
    std::fs::create_dir_all(dir.path().join("services/payment/core/fees")).unwrap();
    git(
        dir.path(),
        &[
            "mv",
            "services/payment/retry/policy.ts",
            "services/payment/core/fees/policy.ts",
        ],
    );
    commit_all(dir.path(), "정책을 결제 처리로 옮김");
    let (_, report) = check_against_main(dir.path(), None);
    assert_eq!(status_of(&report, "unmapped"), "pass");
    // 옮겨 간 스펙과 원래 스펙에 모두 닿는다.
    let specs: Vec<Value> = findings(&report, "spec_update")
        .iter()
        .map(|finding| finding["spec"].clone())
        .collect();
    assert_eq!(specs, ["payment-core", "payment-retry"]);
}

#[test]
fn 설정의_무시_목록에_맞는_새_파일은_매핑_누락에서_뺀다() {
    let dir = feature_repo();
    let config = dir.path().join("lore.config.yaml");
    let text = std::fs::read_to_string(&config).unwrap();
    std::fs::write(
        &config,
        format!("{text}\ncheck:\n  ignore:\n    - 'scripts/**'\n"),
    )
    .unwrap();
    std::fs::create_dir_all(dir.path().join("scripts")).unwrap();
    std::fs::write(dir.path().join("scripts/seed.ts"), "export {};\n").unwrap();
    commit_all(dir.path(), "시드 스크립트");
    let (_, report) = check_against_main(dir.path(), None);
    assert_eq!(status_of(&report, "unmapped"), "pass");
    assert!(findings(&report, "unmapped").is_empty());
}

#[test]
fn 판단이_드는_검사의_사람용_출력은_스펙과_파일을_보여_준다() {
    let dir = feature_repo();
    edit(
        dir.path(),
        "services/payment/retry/policy.ts",
        "export",
        "// 재시도 간격 조정\nexport",
    );
    edit(
        dir.path(),
        "services/notification/retry-alert.ts",
        "export",
        "// 주석만 고침\nexport",
    );
    std::fs::create_dir_all(dir.path().join("services/billing")).unwrap();
    std::fs::write(
        dir.path().join("services/billing/invoice.ts"),
        "export {};\n",
    )
    .unwrap();
    commit_all(dir.path(), "여러 변경");
    std::fs::write(
        dir.path().join(".git/body.md"),
        "lore: no-impact payment-notification 주석만 바꿈\n",
    )
    .unwrap();
    let output = lore_in(
        dir.path(),
        &[
            "check",
            "--base",
            "main",
            "--pr-body",
            ".git/body.md",
            "--for-human",
        ],
    );
    assert_eq!(output.status.code(), Some(0));
    let text = stdout(&output);
    assert!(
        text.starts_with("lore check: 경고 2, 건너뜀 1, 통과 5\n"),
        "{text}"
    );
    assert!(
        text.contains(
            "경고  스펙 동반 갱신
  payment-notification 영향 없음으로 넘김: 주석만 바꿈
  payment-retry 스펙이 바뀌지 않았습니다. 바뀐 파일: services/payment/retry/policy.ts
  고치려면: "
        ),
        "{text}"
    );
    assert!(
        text.contains(
            "경고  매핑 누락
  services/billing/invoice.ts 어떤 스펙의 paths, tests에도 맞지 않습니다.
  고치려면: "
        ),
        "{text}"
    );
}

#[test]
fn 비교_기준을_찾지_못하면_git_오류다() {
    let dir = clean_repo();
    let output = lore_in(dir.path(), &["check", "--base", "no-such-branch"]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(json(&output)["error"]["code"], "git");
}

/// 파일마다 줄을 하나 더해 커밋한다.
fn touch_commit(dir: &Path, files: &[&str], message: &str) {
    for file in files {
        let path = dir.join(file);
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(path, format!("{text}// {message}\n")).unwrap();
    }
    commit_all(dir, message);
}

/// 나눌 후보와 합칠 후보가 모두 나오는 이력을 만든다.
fn history_repo() -> TempDir {
    let dir = clean_repo();
    let charge = "services/payment/core/charge.ts";
    let policy = "services/payment/retry/policy.ts";
    touch_commit(dir.path(), &[policy, charge], "1");
    touch_commit(
        dir.path(),
        &["services/payment/retry/queue.ts", charge],
        "2",
    );
    touch_commit(dir.path(), &[policy], "3");
    touch_commit(
        dir.path(),
        &[
            "services/payment/index.ts",
            charge,
            policy,
            "services/notification/retry-alert.ts",
        ],
        "4",
    );
    dir
}

#[test]
fn 보고는_report를_줄_때만_내고_병합을_막지_않는다() {
    let dir = history_repo();
    assert!(
        json(&lore_in(dir.path(), &["check"]))
            .get("report")
            .is_none()
    );

    let output = lore_in(dir.path(), &["check", "--report"]);
    assert_eq!(output.status.code(), Some(0));
    let report = &json(&output)["report"];
    assert_eq!(
        report["unsourced_decisions"],
        serde_json::json!({
            "count": 2,
            "total": 4,
            "ratio": 0.5,
            "decisions": ["ADR-20260918-retry-inline", "ADR-20261110-log-format"],
        })
    );
    // 고정 예제의 첫 커밋과 스펙에 닿지 않는 설치 커밋은 세지 않는다.
    assert_eq!(
        report["spec_size"],
        serde_json::json!({
            "commits": 4,
            "thresholds": {"split_percent": 50, "merge_percent": 80, "group_size": 4},
            "split": [
                {"spec": "payment-core", "commits": 3, "ratio": 0.75},
                {"spec": "payment-retry", "commits": 4, "ratio": 1.0},
            ],
            "merge": [
                {"specs": ["payment", "payment-notification"], "together": 1, "either": 1, "ratio": 1.0},
            ],
            "groups": [
                {"specs": ["payment", "payment-core", "payment-notification", "payment-retry"], "commits": 1},
            ],
        })
    );
}

#[test]
fn 보고의_사람용_출력은_검사_결과_뒤에_붙는다() {
    let dir = history_repo();
    let output = lore_in(dir.path(), &["check", "--report", "--for-human"]);
    let text = stdout(&output);
    assert!(
        text.ends_with(
            "통과  안내문 일치

보고
  근거 없는 결정: 2/4 (50%)
  스펙을 건드린 커밋 4건 기준
  나눌 스펙 후보 (커밋의 50% 이상):
    payment-core 3/4 (75%)
    payment-retry 4/4 (100%)
  합칠 스펙 후보 (둘 중 하나가 바뀐 커밋의 80% 이상에서 함께 바뀜):
    payment, payment-notification 1/1 (100%)
  합칠 스펙 후보 (한 커밋에서 4개 이상 함께 바뀜):
    payment, payment-core, payment-notification, payment-retry 1건
"
        ),
        "{text}"
    );
}

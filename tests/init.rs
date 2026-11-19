//! `lore init`이 기존 프로젝트 도입의 진행 상태를 내는지 고정 예제로 확인한다.

mod common;

use common::{json, lore_in, repo};

#[test]
fn 진행_상태와_스펙에_속한_코드_파일의_비율을_낸다() {
    let dir = repo("payment");
    let output = lore_in(dir.path(), &["init"]);
    assert_eq!(output.status.code(), Some(0));
    let report = json(&output);
    assert_eq!(report["ok"], true);
    assert_eq!(report["channels"].as_array().unwrap().len(), 3);
    assert_eq!(report["channels"][0]["id"], "github");
    assert_eq!(report["channels"][0]["tool"], "lore-connector-github");
    assert_eq!(report["sources"], 2);
    assert_eq!(report["specs"], 4);
    // 소스 폴더와 설정 파일은 빼고, 어느 스펙에도 없는 README.md만 남는다.
    assert_eq!(
        report["coverage"],
        serde_json::json!({
            "scope": "",
            "files": 8,
            "mapped": 7,
            "ratio": 0.87,
            "unmapped": ["README.md"],
        })
    );
    assert_eq!(report["next"], "draft");
}

#[test]
fn 범위를_주면_그_폴더의_파일만_센다() {
    let dir = repo("payment");
    let report = json(&lore_in(
        &dir.path().join("services"),
        &["init", "--scope", "payment"],
    ));
    assert_eq!(report["coverage"]["scope"], "services/payment");
    assert_eq!(report["coverage"]["files"], 6);
    assert_eq!(report["coverage"]["ratio"], 1.0);
    assert_eq!(report["next"], "done");

    // 스펙이 맡을 코드 파일이 없는 범위는 끝난 것이 아니다.
    let report = json(&lore_in(dir.path(), &["init", "--scope", "docs"]));
    assert_eq!(report["coverage"]["files"], 0);
    assert_eq!(report["next"], "no_files");

    let output = lore_in(dir.path(), &["init", "--scope", "nowhere"]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        json(&output)["error"]["problems"][0]["code"],
        "invalid_scope"
    );
}

#[test]
fn 채널과_소스_노트가_없으면_앞_단계부터_하라고_한다() {
    let dir = repo("payment");
    std::fs::write(dir.path().join("lore.config.yaml"), "version: 1\n").unwrap();
    assert_eq!(json(&lore_in(dir.path(), &["init"]))["next"], "channels");

    std::fs::write(
        dir.path().join("lore.config.yaml"),
        "version: 1\nchannels:\n  - id: git\n    tool: lore-connector-git\ncheck:\n  ignore: ['*.md']\n",
    )
    .unwrap();
    for file in std::fs::read_dir(dir.path().join("docs/sources")).unwrap() {
        std::fs::remove_file(file.unwrap().path()).unwrap();
    }
    let report = json(&lore_in(dir.path(), &["init"]));
    assert_eq!(report["next"], "collect");
    // check.ignore에 맞는 README.md는 스펙이 맡을 파일이 아니다.
    assert_eq!(report["coverage"]["unmapped"], serde_json::json!([]));
}

#[test]
fn 사람용_출력은_다음_단계와_속하지_않은_파일을_보여_준다() {
    let dir = repo("payment");
    let output = lore_in(dir.path(), &["init", "--for-human"]);
    assert_eq!(
        common::stdout(&output),
        "lore init: 다음은 3단계 스펙과 결정 초안 (lore-init-draft)
  채널 3개, 소스 노트 2개, 스펙 4개
  프로젝트 전체: 코드 파일 8개 중 7개가 스펙에 속함 (87%)
    README.md
"
    );
}

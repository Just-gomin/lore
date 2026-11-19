//! `lore-extractor-markdown` 실행 파일이 추출기 규약대로 입출력하는지 확인한다.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

fn run(input: &serde_json::Value) -> (bool, serde_json::Value) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_lore-extractor-markdown"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.to_string().as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    let json = serde_json::from_slice(&output.stdout).unwrap();
    (output.status.success(), json)
}

fn fixture(name: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
        .display()
        .to_string()
}

#[test]
fn 마크다운_파일_하나를_후보_하나로_낸다() {
    let (ok, json) = run(&serde_json::json!({
        "file": fixture("meeting.md"),
        "channel": "meeting",
        "origin": "https://drive.google.com/file/d/meeting",
        "kind": "discussion",
    }));
    assert!(ok, "{json}");
    let candidate = &json["candidates"][0];
    assert_eq!(json["ok"], true);
    assert_eq!(json["candidates"].as_array().unwrap().len(), 1);
    assert_eq!(candidate["title"], "정산 재시도 회의");
    assert_eq!(candidate["kind"], "discussion");
    assert_eq!(candidate["channel"], "meeting");
    assert_eq!(
        candidate["origin"],
        "https://drive.google.com/file/d/meeting"
    );
    assert_eq!(candidate["occurred_at"], "2026-11-03");
    assert_eq!(candidate["origin_version"].as_str().unwrap().len(), 64);
    assert!(
        candidate["text"]
            .as_str()
            .unwrap()
            .starts_with("# 정산 재시도 회의\n\n- 운영 담당")
    );
}

#[test]
fn 날짜가_없으면_입력으로_받는다() {
    let input = serde_json::json!({
        "file": fixture("memo.txt"),
        "channel": "memo",
        "origin": "https://drive.google.com/file/d/memo",
    });
    let (ok, json) = run(&input);
    assert!(!ok);
    assert_eq!(json["error"]["code"], "occurred_at_missing");

    let mut input = input;
    input["occurred_at"] = "2026-11-04".into();
    let (ok, json) = run(&input);
    assert!(ok, "{json}");
    assert_eq!(json["candidates"][0]["title"], "memo");
    assert_eq!(json["candidates"][0]["kind"], "reference");
}

#[test]
fn 입력과_파일의_오류를_코드로_낸다() {
    let (ok, json) = run(&serde_json::json!({"file": fixture("memo.txt"), "channel": "memo"}));
    assert!(!ok);
    assert_eq!(json["error"]["code"], "invalid_input");

    let (_, json) = run(&serde_json::json!({
        "file": fixture("memo.txt"), "channel": "memo", "origin": "x", "lang": "eng",
    }));
    assert_eq!(json["error"]["code"], "invalid_input");

    let (_, json) = run(&serde_json::json!({
        "file": fixture("none.md"), "channel": "memo", "origin": "x",
    }));
    assert_eq!(json["error"]["code"], "io");
}

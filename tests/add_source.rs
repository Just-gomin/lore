//! `lore add source`가 등급 규칙대로 소스 노트를 만들고 갱신하는 과정을 확인한다.
//!
//! 고정 예제 설정의 채널: `github`(일반), `kakaotalk`(민감), `hr`(제외).

mod common;

use std::path::Path;

use common::{json, lore_in, lore_with_input, repo, stdout};
use serde_json::{Value, json};

fn input(channel: &str, origin_version: &str) -> String {
    json!({
        "title": "재시도 횟수 상향 요청",
        "slug": "retry-count",
        "kind": "discussion",
        "channel": channel,
        "origin": "https://example.com/threads/retry-count",
        "origin_version": origin_version,
        "occurred_at": "2026-11-10",
        "body": "- 야간 정산 시간대에 3회 재시도로는 실패가 남는다고 알림\n- 5회까지 늘려 달라고 요청\n"
    })
    .to_string()
}

fn add(dir: &Path, input: &str, date: &str) -> Value {
    let output = lore_with_input(dir, &["add", "source", "--date", date], input);
    json(&output)
}

fn read(dir: &Path, path: &str) -> String {
    std::fs::read_to_string(dir.join(path)).unwrap()
}

const PATH: &str = "docs/sources/SRC-20261110-retry-count.md";

#[test]
fn 일반_채널은_본문까지_남긴다() {
    let dir = repo("payment");
    let report = add(
        dir.path(),
        &input("github", "2026-11-10T14:02:00+09:00"),
        "2026-11-11",
    );
    assert_eq!(
        report,
        json!({
            "ok": true, "id": "SRC-20261110-retry-count", "path": PATH,
            "sensitivity": "normal", "change": "created"
        })
    );
    assert_eq!(
        read(dir.path(), PATH),
        "---
id: SRC-20261110-retry-count
title: 재시도 횟수 상향 요청
kind: discussion # discussion | client-doc | evidence | reference
origin: https://example.com/threads/retry-count
origin_version: 2026-11-10T14:02:00+09:00
extracted_at: 2026-11-11
sensitivity: normal # normal | sensitive
---

- 야간 정산 시간대에 3회 재시도로는 실패가 남는다고 알림
- 5회까지 늘려 달라고 요청
"
    );
    assert!(lore_in(dir.path(), &["build"]).status.success());
}

#[test]
fn 민감_채널과_설정에_없는_채널은_머리말만_남긴다() {
    for channel in ["kakaotalk", "slack"] {
        let dir = repo("payment");
        let report = add(dir.path(), &input(channel, "v1"), "2026-11-11");
        assert_eq!(report["sensitivity"], "sensitive", "{channel}");
        let text = read(dir.path(), PATH);
        assert!(
            text.ends_with("sensitivity: sensitive # normal | sensitive\n---\n"),
            "{text}"
        );
        assert!(lore_in(dir.path(), &["build"]).status.success());
    }
}

#[test]
fn 제외_채널은_거부하고_파일을_만들지_않는다() {
    let dir = repo("payment");
    let output = lore_with_input(dir.path(), &["add", "source"], &input("hr", "v1"));
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        json(&output)["error"],
        json!({"code": "excluded_channel", "channel": "hr"})
    );
    assert!(!dir.path().join(PATH).exists());
}

#[test]
fn 같은_원본은_판이_같으면_그대로_두고_다르면_갱신한다() {
    let dir = repo("payment");
    add(dir.path(), &input("github", "v1"), "2026-11-11");

    let same = add(dir.path(), &input("github", "v1"), "2026-11-20");
    assert_eq!(same["change"], "unchanged");
    assert!(read(dir.path(), PATH).contains("extracted_at: 2026-11-11\n"));

    // 슬러그와 날짜가 달라도 같은 원본이면 처음의 ID와 파일을 쓴다.
    let mut newer: Value = serde_json::from_str(&input("github", "v2")).unwrap();
    newer["slug"] = json!("retry-count-2");
    newer["occurred_at"] = json!("2026-11-12");
    let updated = add(dir.path(), &newer.to_string(), "2026-11-20");
    assert_eq!(updated["change"], "updated");
    assert_eq!(updated["id"], "SRC-20261110-retry-count");
    let text = read(dir.path(), PATH);
    assert!(text.contains("origin_version: v2\n"));
    assert!(text.contains("extracted_at: 2026-11-20\n"));
}

#[test]
fn 등급이_민감으로_바뀌면_판이_같아도_본문을_지운다() {
    let dir = repo("payment");
    add(dir.path(), &input("github", "v1"), "2026-11-11");
    let report = add(dir.path(), &input("kakaotalk", "v1"), "2026-11-12");
    assert_eq!(report["change"], "updated");
    assert!(!read(dir.path(), PATH).contains("야간 정산"));
}

#[test]
fn 원본은_다른데_아이디가_겹치면_id_exists다() {
    let dir = repo("payment");
    add(dir.path(), &input("github", "v1"), "2026-11-11");
    let mut other: Value = serde_json::from_str(&input("github", "v1")).unwrap();
    other["origin"] = json!("https://example.com/threads/other");
    let output = lore_with_input(dir.path(), &["add", "source"], &other.to_string());
    assert_eq!(json(&output)["error"]["code"], "id_exists");
}

#[test]
fn from으로_현재_폴더_기준의_파일을_읽는다() {
    let dir = repo("payment");
    let sub = dir.path().join("services");
    std::fs::write(sub.join("input.json"), input("github", "v1")).unwrap();
    let output = lore_in(
        &sub,
        &["add", "source", "--from", "input.json", "--for-human"],
    );
    assert!(output.status.success());
    assert!(stdout(&output).starts_with("lore add source: SRC-20261110-retry-count (만듦)"));
}

#[test]
fn 잘못된_입력은_항목마다_문제를_낸다() {
    let dir = repo("payment");
    let output = lore_with_input(dir.path(), &["add", "source"], r#"{"title": "제목"}"#);
    assert_eq!(output.status.code(), Some(1));
    let report = json(&output);
    assert_eq!(report["error"]["code"], "invalid_input");
    assert_eq!(report["error"]["problems"][0]["path"], "-");
    assert_eq!(report["error"]["problems"].as_array().unwrap().len(), 6);
}

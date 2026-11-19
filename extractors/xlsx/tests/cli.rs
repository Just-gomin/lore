//! `lore-extractor-xlsx` 실행 파일이 고정 파일을 기대한 표로 바꾸는지 확인한다.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

fn run(input: &serde_json::Value) -> (Option<i32>, serde_json::Value) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_lore-extractor-xlsx"))
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
    (output.status.code(), json)
}

fn input(name: &str) -> serde_json::Value {
    let file = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    serde_json::json!({
        "file": file.display().to_string(),
        "channel": "meeting",
        "origin": format!("https://drive.google.com/file/d/{name}"),
        "kind": "client-doc",
    })
}

const TABLE: &str = "## 회의\n\n| 항목 | 담당 | 날짜 | 금액 | 두 배 |\n| --- | --- | --- | --- | --- |\n| 재시도 횟수 | 운영 | 2026-11-03 | 3 | 6 |\n| 간격 |  | 2026-11-04 10:30 | 1.5 | 3 |\n";

#[test]
fn 엑셀의_보이는_시트를_표로_내고_숨긴_시트는_수만_적는다() {
    let (code, json) = run(&input("meeting.xlsx"));
    assert_eq!(code, Some(0), "{json}");
    let candidate = &json["candidates"][0];
    assert_eq!(candidate["title"], "정산 회의 자료");
    assert_eq!(candidate["kind"], "client-doc");
    // 문서 속성의 수정 시각(2026-11-03T03:00:00Z)을 로컬 날짜로 바꾼다.
    assert!(
        ["2026-11-02", "2026-11-03"].contains(&candidate["occurred_at"].as_str().unwrap()),
        "{candidate}"
    );
    let text = candidate["text"].as_str().unwrap();
    // 가운데 빈 행은 빠지고, 병합 셀은 왼쪽 위에만, 수식은 저장된 값, 오류 값은 그대로다.
    assert_eq!(
        text,
        format!(
            "{TABLE}| 비고 | a\\|b | 줄<br>바꿈 |  | #DIV/0! |\n\n(숨긴 시트 1개는 넣지 않았습니다.)"
        )
    );
    assert!(!text.contains("비밀"));
}

#[test]
fn ods도_같은_규칙으로_읽는다() {
    let (code, json) = run(&input("meeting.ods"));
    assert_eq!(code, Some(0), "{json}");
    let candidate = &json["candidates"][0];
    assert_eq!(candidate["title"], "정산 회의 자료");
    // 시간대가 없는 dc:date는 적힌 날짜 그대로다.
    assert_eq!(candidate["occurred_at"], "2026-11-03");
    assert_eq!(
        candidate["text"],
        format!("{TABLE}| 비고 | a\\|b | 줄<br>바꿈 |  |  |\n\n(숨긴 시트 1개는 넣지 않았습니다.)")
    );
}

#[test]
fn 같은_파일은_출력이_같고_행_상한을_바꿀_수_있다() {
    let first = run(&input("meeting.xlsx")).1;
    assert_eq!(run(&input("meeting.xlsx")).1, first);

    let mut limited = input("meeting.xlsx");
    limited["max_rows"] = 1.into();
    let (_, json) = run(&limited);
    let text = json["candidates"][0]["text"].as_str().unwrap();
    assert!(text.contains("(남은 행 2개는 넣지 않았습니다.)"), "{text}");
    assert!(!text.contains("간격"));

    limited["max_rows"] = 0.into();
    let (code, json) = run(&limited);
    assert_eq!(code, Some(2));
    assert_eq!(json["error"]["code"], "invalid_input");
}

#[test]
fn 스프레드시트가_아니면_읽지_못한다() {
    let mut not_sheet = input("meeting.xlsx");
    not_sheet["file"] = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/make.py")
        .display()
        .to_string()
        .into();
    let (code, json) = run(&not_sheet);
    assert_eq!(code, Some(1));
    assert_eq!(json["error"]["code"], "unreadable_file");
}

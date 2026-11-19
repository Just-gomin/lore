//! `lore-extractor-image` 실행 파일을 가짜 tesseract와 실제 tesseract로 확인한다.

use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn fixture(name: &str) -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
        .display()
        .to_string()
}

fn input(name: &str) -> serde_json::Value {
    serde_json::json!({
        "file": fixture(name),
        "channel": "meeting",
        "origin": format!("https://drive.google.com/file/d/{name}"),
    })
}

/// `tesseract` 자리에 쓸 실행 파일. `body`는 셸 스크립트 본문이다.
fn fake(dir: &Path, body: &str) -> PathBuf {
    let path = dir.join("tesseract");
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn run(input: &serde_json::Value, tesseract: Option<&Path>) -> (Option<i32>, serde_json::Value) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_lore-extractor-image"));
    if let Some(tesseract) = tesseract {
        command.env("LORE_TESSERACT", tesseract);
    }
    let mut child = command
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

/// 4.x처럼 판을 표준 오류로 내고, 받은 인자를 인식한 글로 낸다.
const ECHO: &str = r#"if [ "$1" = "--version" ]; then echo "tesseract 4.1.1" >&2; echo " leptonica-1.79.0" >&2; exit 0; fi
printf 'args: %s %s %s %s\n\f' "$(basename "$1")" "$2" "$3" "$4""#;

#[test]
fn 인식한_글과_도구_판을_내고_exif_촬영_시각을_날짜로_쓴다() {
    let dir = tempfile::tempdir().unwrap();
    let tesseract = fake(dir.path(), ECHO);
    let (code, json) = run(&input("photo.jpg"), Some(&tesseract));
    assert_eq!(code, Some(0), "{json}");
    let candidate = &json["candidates"][0];
    assert_eq!(candidate["title"], "photo");
    assert_eq!(candidate["occurred_at"], "2026-11-03");
    assert_eq!(
        candidate["text"],
        "(글자 인식: tesseract 4.1.1, 언어 kor+eng. 인식 오류가 있을 수 있습니다.)\n\nargs: photo.jpg stdout -l kor+eng"
    );

    let mut english = input("datetime-only.jpg");
    english["lang"] = "eng".into();
    let (_, json) = run(&english, Some(&tesseract));
    let candidate = &json["candidates"][0];
    // DateTimeOriginal이 없으면 DateTime을 쓴다.
    assert_eq!(candidate["occurred_at"], "2026-11-04");
    assert!(candidate["text"].as_str().unwrap().ends_with("-l eng"));
}

#[test]
fn exif가_없으면_날짜를_입력으로_받는다() {
    let dir = tempfile::tempdir().unwrap();
    let tesseract = fake(dir.path(), ECHO);
    let (code, json) = run(&input("text.png"), Some(&tesseract));
    assert_eq!(code, Some(1));
    assert_eq!(json["error"]["code"], "occurred_at_missing");

    let mut dated = input("text.png");
    dated["occurred_at"] = "2026-11-03".into();
    let (code, json) = run(&dated, Some(&tesseract));
    assert_eq!(code, Some(0), "{json}");
}

#[test]
fn 도구가_없거나_실패하면_설치_방법을_알린다() {
    let dir = tempfile::tempdir().unwrap();
    let (code, json) = run(&input("photo.jpg"), Some(&dir.path().join("none")));
    let message = json["error"]["message"].as_str().unwrap();
    assert!(message.contains("none"), "{message}");
    assert_eq!(code, Some(1));
    assert_eq!(json["error"]["code"], "tool_missing");
    assert!(
        json["error"]["message"]
            .as_str()
            .unwrap()
            .contains("brew install tesseract")
    );

    // 실제 tesseract는 언어 하나를 읽지 못하면 그 말을 남기고 나머지 언어로 읽은 뒤 0으로 끝난다.
    let failing = fake(
        dir.path(),
        r#"if [ "$1" = "--version" ]; then echo "tesseract 5.3.4"; exit 0; fi
echo "Failed loading language 'kor'" >&2; echo "garbage"; exit 0"#,
    );
    let (code, json) = run(&input("photo.jpg"), Some(&failing));
    assert_eq!(code, Some(1));
    assert_eq!(json["error"]["code"], "tool_failed");
    let message = json["error"]["message"].as_str().unwrap();
    assert!(
        message.contains("Failed loading language 'kor'"),
        "{message}"
    );
    assert!(message.contains("tesseract-ocr-kor"), "{message}");

    let crashed = fake(dir.path(), r#"echo "segfault" >&2; exit 1"#);
    let (_, json) = run(&input("photo.jpg"), Some(&crashed));
    assert_eq!(json["error"]["code"], "tool_failed");

    // 실행할 수 없는 파일(폴더)도 찾지 못한 것으로 알린다.
    let (_, json) = run(&input("photo.jpg"), Some(dir.path()));
    assert_eq!(json["error"]["code"], "tool_missing");
}

#[test]
fn 판을_읽지_못하면_이름만_쓰고_빼기로_시작하는_경로는_옵션으로_넘기지_않는다() {
    let dir = tempfile::tempdir().unwrap();
    let tesseract = fake(
        dir.path(),
        r#"if [ "$1" = "--version" ]; then echo "libleptonica.so: not found" >&2; exit 127; fi
printf 'file: %s\n' "$1""#,
    );
    std::fs::copy(fixture("photo.jpg"), dir.path().join("--list-langs")).unwrap();
    let mut dashed = input("photo.jpg");
    dashed["file"] = "--list-langs".into();
    let mut command = Command::new(env!("CARGO_BIN_EXE_lore-extractor-image"));
    let mut child = command
        .current_dir(dir.path())
        .env("LORE_TESSERACT", &tesseract)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(dashed.to_string().as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let text = json["candidates"][0]["text"].as_str().unwrap();
    assert!(text.starts_with("(글자 인식: tesseract, 언어"), "{text}");
    assert!(text.ends_with("file: ./--list-langs"), "{text}");
}

#[test]
fn 이미지가_아니거나_언어가_이상하면_부르지_않는다() {
    let dir = tempfile::tempdir().unwrap();
    let tesseract = fake(dir.path(), "echo called; exit 1");
    let mut not_image = input("photo.jpg");
    not_image["file"] = fixture("make.py").into();
    let (_, json) = run(&not_image, Some(&tesseract));
    assert_eq!(json["error"]["code"], "unreadable_file");

    let mut bad_lang = input("photo.jpg");
    bad_lang["lang"] = "eng --psm 3".into();
    let (code, json) = run(&bad_lang, Some(&tesseract));
    assert_eq!(code, Some(2));
    assert_eq!(json["error"]["code"], "invalid_input");
}

/// 실제 tesseract로 고정 이미지의 영어 글자를 읽는다. CI는 tesseract를 설치하고 돌린다.
/// tesseract가 없는 로컬 환경에서는 건너뛰되, CI(`CI` 환경 변수)에서는 실패한다.
#[test]
fn 실제_tesseract가_고정_이미지의_글자를_읽는다() {
    if Command::new("tesseract").arg("--version").output().is_err() {
        assert!(
            std::env::var_os("CI").is_none(),
            "CI에는 tesseract가 있어야 합니다. sudo apt-get install -y tesseract-ocr"
        );
        eprintln!("tesseract가 없어 건너뜁니다. brew install tesseract로 설치하면 돕니다.");
        return;
    }
    let mut english = input("photo.jpg");
    english["lang"] = "eng".into();
    let (code, json) = run(&english, Some(Path::new("tesseract")));
    assert_eq!(code, Some(0), "{json}");
    let text = json["candidates"][0]["text"].as_str().unwrap();
    assert!(text.contains("RETRY"), "{text}");
    assert!(text.contains("TIMES"), "{text}");
}

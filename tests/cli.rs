//! 바이너리를 실제로 실행해 출력 계약을 확인한다.

use std::process::{Command, Output};

fn lore(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_lore"))
        .args(args)
        .output()
        .expect("lore 바이너리를 실행할 수 있어야 한다")
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("표준 출력은 UTF-8이어야 한다")
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).expect("표준 오류는 UTF-8이어야 한다")
}

#[test]
fn 버전은_기본이_json이다() {
    let output = lore(&["--version"]);
    assert!(output.status.success());
    assert_eq!(
        stdout(&output).trim(),
        format!(r#"{{"ok":true,"version":"{}"}}"#, env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn for_human이면_버전을_텍스트로_낸다() {
    let output = lore(&["--version", "--for-human"]);
    assert!(output.status.success());
    assert_eq!(
        stdout(&output).trim(),
        format!("lore {}", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn 도움말에_모든_명령이_나온다() {
    let output = lore(&["help", "--for-human"]);
    assert!(output.status.success());
    let text = stdout(&output);
    for name in [
        "install",
        "uninstall",
        "init",
        "add",
        "why",
        "impact",
        "build",
        "check",
        "sync",
    ] {
        assert!(text.contains(name), "도움말에 {name}이 없다");
    }
}

#[test]
fn for_human이면_오류를_표준_오류로_낸다() {
    let output = lore(&["explain", "--for-human"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(stdout(&output).is_empty());
    assert!(stderr(&output).contains("explain"));
}

#[test]
fn 모르는_명령은_unknown_command다() {
    let output = lore(&["explain"]);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(
        stdout(&output).trim(),
        r#"{"ok":false,"error":{"code":"unknown_command","command":"explain"}}"#
    );
}

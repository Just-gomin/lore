//! 출력 형식. 기본은 JSON이고 `--for-human`일 때만 사람용 텍스트를 낸다.

use serde::Serialize;

use crate::cli::Command;
use crate::error::LoreError;

/// 사용법이 잘못됐거나 아직 구현되지 않은 명령일 때의 종료 코드.
pub const EXIT_USAGE: u8 = 2;

/// 명령 하나를 실행한 결과.
#[derive(Debug, PartialEq, Eq)]
pub struct Outcome {
    pub stdout: String,
    pub stderr: String,
    pub code: u8,
}

impl Outcome {
    fn ok(stdout: String) -> Outcome {
        Outcome {
            stdout,
            stderr: String::new(),
            code: 0,
        }
    }

    /// 오류도 JSON일 때는 표준 출력으로 낸다. 에이전트가 한 곳만 읽으면 되게 하기 위해서다.
    fn fail(for_human: bool, json: String, text: String) -> Outcome {
        Outcome::fail_with(EXIT_USAGE, for_human, json, text)
    }

    fn fail_with(code: u8, for_human: bool, json: String, text: String) -> Outcome {
        if for_human {
            Outcome {
                stdout: String::new(),
                stderr: text,
                code,
            }
        } else {
            Outcome {
                stdout: json,
                stderr: String::new(),
                code,
            }
        }
    }
}

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Serialize)]
struct VersionJson {
    ok: bool,
    version: &'static str,
}

#[derive(Serialize)]
struct HelpJson {
    ok: bool,
    version: &'static str,
    commands: Vec<CommandJson>,
}

#[derive(Serialize)]
struct CommandJson {
    name: &'static str,
    summary: &'static str,
}

#[derive(Serialize)]
struct FailureJson<E> {
    ok: bool,
    error: E,
}

#[derive(Serialize)]
struct SuccessJson<'a, T> {
    ok: bool,
    #[serde(flatten)]
    report: &'a T,
}

/// 오류 객체. `code`가 맨 앞에 오고 나머지 항목이 뒤따른다.
#[derive(Serialize)]
#[serde(tag = "code", rename_all = "snake_case")]
enum ErrorJson<'a> {
    NotImplemented { command: &'a str },
    UnknownCommand { command: &'a str },
    InvalidArguments { message: &'a str },
}

/// 출력용 구조체를 한 줄 JSON으로 바꾼다.
fn to_json<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).expect("출력 구조체는 항상 JSON으로 바꿀 수 있다")
}

fn failure(error: ErrorJson<'_>) -> String {
    to_json(&FailureJson { ok: false, error })
}

/// 명령이 성공했을 때. JSON은 `ok` 뒤에 결과의 항목을 그대로 펼친다.
pub fn success<T: Serialize>(
    report: &T,
    for_human: bool,
    text: impl FnOnce(&T) -> String,
) -> Outcome {
    if for_human {
        Outcome::ok(text(report))
    } else {
        Outcome::ok(to_json(&SuccessJson { ok: true, report }))
    }
}

/// 명령이 실행 중에 실패했을 때.
pub fn error(error: &LoreError, for_human: bool) -> Outcome {
    Outcome::fail_with(
        error.exit_code(),
        for_human,
        to_json(&FailureJson { ok: false, error }),
        format!("lore: {error}"),
    )
}

pub fn version(for_human: bool) -> Outcome {
    if for_human {
        Outcome::ok(format!("lore {VERSION}"))
    } else {
        Outcome::ok(to_json(&VersionJson {
            ok: true,
            version: VERSION,
        }))
    }
}

pub fn help(for_human: bool) -> Outcome {
    if for_human {
        let width = Command::ALL
            .iter()
            .map(|command| command.name().len())
            .max()
            .unwrap_or(0);
        let mut text = String::from("사용법: lore <명령> [옵션] [--for-human]\n\n명령\n");
        for command in Command::ALL {
            text.push_str(&format!(
                "  {:width$}  {}\n",
                command.name(),
                command.summary()
            ));
        }
        text.push_str("\n출력은 기본이 JSON이고, --for-human을 붙이면 사람용 텍스트입니다.");
        Outcome::ok(text)
    } else {
        let commands = Command::ALL
            .into_iter()
            .map(|command| CommandJson {
                name: command.name(),
                summary: command.summary(),
            })
            .collect();
        Outcome::ok(to_json(&HelpJson {
            ok: true,
            version: VERSION,
            commands,
        }))
    }
}

pub fn not_implemented(command: Command, for_human: bool) -> Outcome {
    Outcome::fail(
        for_human,
        failure(ErrorJson::NotImplemented {
            command: command.name(),
        }),
        format!("lore {}: 아직 구현되지 않았습니다.", command.name()),
    )
}

pub fn unknown_command(name: &str, for_human: bool) -> Outcome {
    Outcome::fail(
        for_human,
        failure(ErrorJson::UnknownCommand { command: name }),
        format!(
            "lore: '{name}'은(는) 없는 명령입니다. 'lore help --for-human'으로 목록을 확인하세요."
        ),
    )
}

pub fn invalid_arguments(message: &str, for_human: bool) -> Outcome {
    Outcome::fail(
        for_human,
        failure(ErrorJson::InvalidArguments { message }),
        format!("lore: {message}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 오류_json은_code가_맨_앞에_온다() {
        assert_eq!(
            failure(ErrorJson::UnknownCommand { command: "explain" }),
            r#"{"ok":false,"error":{"code":"unknown_command","command":"explain"}}"#
        );
    }

    #[test]
    fn 따옴표와_제어_문자를_이스케이프한다() {
        assert_eq!(
            failure(ErrorJson::InvalidArguments {
                message: "a\"b\\c\nd\u{1}"
            }),
            r#"{"ok":false,"error":{"code":"invalid_arguments","message":"a\"b\\c\nd\u0001"}}"#
        );
    }

    #[test]
    fn 한글은_그대로_둔다() {
        assert_eq!(
            failure(ErrorJson::NotImplemented { command: "결정" }),
            r#"{"ok":false,"error":{"code":"not_implemented","command":"결정"}}"#
        );
    }
}

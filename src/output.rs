//! 출력 형식. 기본은 JSON이고 `--for-human`일 때만 사람용 텍스트를 낸다.

use crate::cli::Command;

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
        if for_human {
            Outcome {
                stdout: String::new(),
                stderr: text,
                code: EXIT_USAGE,
            }
        } else {
            Outcome {
                stdout: json,
                stderr: String::new(),
                code: EXIT_USAGE,
            }
        }
    }
}

const VERSION: &str = env!("CARGO_PKG_VERSION");

pub fn version(for_human: bool) -> Outcome {
    if for_human {
        Outcome::ok(format!("lore {VERSION}"))
    } else {
        Outcome::ok(format!(r#"{{"ok":true,"version":{}}}"#, quote(VERSION)))
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
        let commands: Vec<String> = Command::ALL
            .iter()
            .map(|command| {
                format!(
                    r#"{{"name":{},"summary":{}}}"#,
                    quote(command.name()),
                    quote(command.summary())
                )
            })
            .collect();
        Outcome::ok(format!(
            r#"{{"ok":true,"version":{},"commands":[{}]}}"#,
            quote(VERSION),
            commands.join(",")
        ))
    }
}

pub fn not_implemented(command: Command, for_human: bool) -> Outcome {
    Outcome::fail(
        for_human,
        format!(
            r#"{{"ok":false,"error":{{"code":"not_implemented","command":{}}}}}"#,
            quote(command.name())
        ),
        format!("lore {}: 아직 구현되지 않았습니다.", command.name()),
    )
}

pub fn unknown_command(name: &str, for_human: bool) -> Outcome {
    Outcome::fail(
        for_human,
        format!(
            r#"{{"ok":false,"error":{{"code":"unknown_command","command":{}}}}}"#,
            quote(name)
        ),
        format!(
            "lore: '{name}'은(는) 없는 명령입니다. 'lore help --for-human'으로 목록을 확인하세요."
        ),
    )
}

/// 문자열을 JSON 문자열 리터럴로 바꾼다.
fn quote(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if (ch as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", ch as u32)),
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 따옴표와_제어_문자를_이스케이프한다() {
        assert_eq!(quote("a\"b\\c\nd\u{1}"), r#""a\"b\\c\nd\u0001""#);
    }

    #[test]
    fn 한글은_그대로_둔다() {
        assert_eq!(quote("결정"), "\"결정\"");
    }
}

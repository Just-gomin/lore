//! 추출기가 함께 쓰는 입력 읽기와 출력. 기획서 6장의 추출기 규약을 따릅니다.
//!
//! 추출기는 표준 입력으로 JSON 하나를 받아, 파일 하나를 후보 하나로 바꿔 표준 출력으로 냅니다.

use std::io::Read;
use std::path::PathBuf;
use std::process::ExitCode;

use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::{Candidate, ConnectorError, candidates_json, error_json};

/// 모든 추출기가 받는 입력.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Input {
    pub file: PathBuf,
    pub channel: String,
    pub origin: String,
    pub kind: String,
    pub title: Option<String>,
    /// `YYYY-MM-DD`
    pub occurred_at: Option<String>,
}

/// 추출기마다 더 받는 항목. 추출기가 읽은 항목은 꺼내 가고, 남은 항목은 모르는 항목으로 오류가 됩니다.
#[derive(Debug, Default)]
pub struct Extra(Map<String, Value>);

impl Extra {
    /// 문자열 항목을 꺼냅니다.
    pub fn string(&mut self, key: &str) -> Result<Option<String>, ConnectorError> {
        match self.0.remove(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(value)) => Ok(Some(value)),
            Some(_) => Err(invalid(format!("{key}는 문자열이어야 합니다."))),
        }
    }

    /// 0보다 큰 정수 항목을 꺼냅니다.
    pub fn count(&mut self, key: &str) -> Result<Option<usize>, ConnectorError> {
        match self.0.remove(key) {
            None | Some(Value::Null) => Ok(None),
            Some(value) => value
                .as_u64()
                .filter(|count| *count > 0)
                .map(|count| Some(count as usize))
                .ok_or_else(|| invalid(format!("{key}는 0보다 큰 정수여야 합니다."))),
        }
    }

    fn finish(self) -> Result<(), ConnectorError> {
        match self.0.keys().next() {
            None => Ok(()),
            Some(key) => Err(invalid(format!("모르는 항목 '{key}'가 있습니다."))),
        }
    }
}

/// 추출기가 파일에서 읽은 것. 제목과 날짜는 입력에 없을 때만 씁니다.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Extracted {
    pub title: Option<String>,
    /// `YYYY-MM-DD`
    pub occurred_at: Option<String>,
    pub text: String,
}

fn invalid(message: impl Into<String>) -> ConnectorError {
    ConnectorError::new("invalid_input", message)
}

/// 입력 JSON을 공통 항목과 추출기마다의 항목으로 나눕니다.
pub fn parse_input(text: &str) -> Result<(Input, Extra), ConnectorError> {
    let value: Value = serde_json::from_str(text)
        .map_err(|error| invalid(format!("입력 JSON을 읽지 못했습니다: {error}")))?;
    let Value::Object(mut map) = value else {
        return Err(invalid("입력은 JSON 객체여야 합니다."));
    };
    let mut take = |key: &str, required: bool| -> Result<Option<String>, ConnectorError> {
        match map.remove(key) {
            None | Some(Value::Null) if required => Err(invalid(format!("{key}가 필요합니다."))),
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(value)) if required && value.trim().is_empty() => {
                Err(invalid(format!("{key}가 비어 있습니다.")))
            }
            Some(Value::String(value)) => Ok(Some(value)),
            Some(_) => Err(invalid(format!("{key}는 문자열이어야 합니다."))),
        }
    };
    let file = take("file", true)?.unwrap_or_default();
    let channel = take("channel", true)?.unwrap_or_default();
    let origin = take("origin", true)?.unwrap_or_default();
    let kind = take("kind", false)?.unwrap_or_else(|| "reference".to_string());
    let title = take("title", false)?;
    let occurred_at = take("occurred_at", false)?;
    if let Some(date) = &occurred_at
        && !is_date(date)
    {
        return Err(invalid(format!(
            "occurred_at '{date}'는 YYYY-MM-DD여야 합니다."
        )));
    }
    Ok((
        Input {
            file: PathBuf::from(file),
            channel,
            origin,
            kind,
            title,
            occurred_at,
        },
        Extra(map),
    ))
}

/// `YYYY-MM-DD` 꼴인지 봅니다. 달은 1\~12, 날은 1\~31만 봅니다(코어의 날짜 검사와 같음).
pub fn is_date(text: &str) -> bool {
    let parts: Vec<&str> = text.split('-').collect();
    let [year, month, day] = parts.as_slice() else {
        return false;
    };
    let digits =
        |part: &str, len: usize| part.len() == len && part.bytes().all(|b| b.is_ascii_digit());
    if !(digits(year, 4) && digits(month, 2) && digits(day, 2)) {
        return false;
    }
    let month: u32 = month.parse().unwrap_or(0);
    let day: u32 = day.parse().unwrap_or(0);
    (1..=12).contains(&month) && (1..=31).contains(&day)
}

/// 파일에 적힌 날짜나 시각을 `YYYY-MM-DD`로 바꿉니다. 시간대가 있는 값(RFC 3339)은 실행 환경의 로컬 날짜로,
/// 시간대가 없는 값은 적힌 날짜 그대로 씁니다. 날짜로 시작하지 않으면 `None`입니다.
pub fn local_date(value: &str) -> Option<String> {
    let value = value.trim();
    if let Ok(time) = chrono::DateTime::parse_from_rfc3339(value) {
        return Some(
            time.with_timezone(&chrono::Local)
                .format("%Y-%m-%d")
                .to_string(),
        );
    }
    let date = value.get(..10)?;
    let rest = &value[10..];
    let ends = rest.is_empty() || rest.starts_with(['T', ' ']);
    (ends && is_date(date)).then(|| date.to_string())
}

/// 파일 내용의 sha256을 소문자 16진수로 씁니다. `shasum -a 256`과 같은 값입니다.
pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// 입력과 파일 내용, 추출한 것으로 후보를 만듭니다. 날짜를 찾지 못했으면 `occurred_at_missing`입니다.
pub fn candidate(
    input: Input,
    bytes: &[u8],
    extracted: Extracted,
) -> Result<Candidate, ConnectorError> {
    let title = input
        .title
        .filter(|title| !title.trim().is_empty())
        .or(extracted.title.filter(|title| !title.trim().is_empty()))
        .unwrap_or_else(|| {
            input
                .file
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or_default()
        });
    let occurred_at = input.occurred_at.or(extracted.occurred_at).ok_or_else(|| {
        ConnectorError::new(
            "occurred_at_missing",
            format!(
                "{}에서 날짜를 찾지 못했습니다. 입력에 occurred_at(YYYY-MM-DD)을 넣어 다시 부르세요.",
                input.file.display()
            ),
        )
    })?;
    Ok(Candidate {
        title,
        kind: input.kind,
        channel: input.channel,
        origin: input.origin,
        origin_version: sha256_hex(bytes),
        occurred_at,
        text: extracted.text,
    })
}

/// 표준 입력을 읽어 `extract`로 파일을 바꾸고, 결과를 표준 출력에 씁니다.
///
/// `extract`는 입력, 추출기마다의 항목, 파일 내용을 받습니다. 추출기마다의 항목은 다 꺼내 가야 합니다.
pub fn run(
    extract: impl FnOnce(&Input, &mut Extra, &[u8]) -> Result<Extracted, ConnectorError>,
) -> ExitCode {
    let mut text = String::new();
    let result = std::io::stdin()
        .read_to_string(&mut text)
        .map_err(|error| ConnectorError::new("io", format!("표준 입력을 읽지 못했습니다: {error}")))
        .and_then(|_| extract_from(&text, extract));
    match result {
        Ok(candidate) => {
            println!("{}", candidates_json(&[candidate]));
            ExitCode::SUCCESS
        }
        Err(error) => {
            println!("{}", error_json(&error));
            // 커넥터처럼 입력이 잘못됐으면 2, 나머지는 1입니다.
            if error.code == "invalid_input" {
                ExitCode::from(2)
            } else {
                ExitCode::FAILURE
            }
        }
    }
}

/// [`run`]의 본체. 입력 글을 받아 후보를 돌려줍니다.
pub fn extract_from(
    text: &str,
    extract: impl FnOnce(&Input, &mut Extra, &[u8]) -> Result<Extracted, ConnectorError>,
) -> Result<Candidate, ConnectorError> {
    let (input, mut extra) = parse_input(text)?;
    let bytes = std::fs::read(&input.file).map_err(|error| {
        ConnectorError::new(
            "io",
            format!("{}를 읽지 못했습니다: {error}", input.file.display()),
        )
    })?;
    let extracted = extract(&input, &mut extra, &bytes)?;
    extra.finish()?;
    candidate(input, &bytes, extracted)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 꼭_있어야_할_항목과_모르는_항목을_오류로_낸다() {
        let error = parse_input(r#"{"file": "a.md", "channel": "memo"}"#).unwrap_err();
        assert_eq!(error.code, "invalid_input");
        assert!(error.message.contains("origin"));

        let (_, extra) =
            parse_input(r#"{"file": "a.md", "channel": "memo", "origin": "x", "lang": "eng"}"#)
                .unwrap();
        assert_eq!(extra.finish().unwrap_err().code, "invalid_input");

        let error = parse_input(
            r#"{"file": "a.md", "channel": "memo", "origin": "x", "occurred_at": "2026-13-01"}"#,
        )
        .unwrap_err();
        assert!(error.message.contains("occurred_at"));
        assert_eq!(parse_input("[]").unwrap_err().code, "invalid_input");
    }

    #[test]
    fn 입력의_제목과_날짜가_파일에서_찾은_것보다_앞선다() {
        let (input, _) = parse_input(
            r#"{"file": "dir/회의.md", "channel": "memo", "origin": "x", "title": "입력 제목"}"#,
        )
        .unwrap();
        let found = Extracted {
            title: Some("파일 제목".to_string()),
            occurred_at: Some("2026-11-01".to_string()),
            text: "본문".to_string(),
        };
        let made = candidate(input.clone(), b"abc", found).unwrap();
        assert_eq!(made.title, "입력 제목");
        assert_eq!(made.occurred_at, "2026-11-01");
        assert_eq!(made.kind, "reference");
        assert_eq!(
            made.origin_version,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );

        let input = Input {
            title: None,
            ..input
        };
        let missing = candidate(input.clone(), b"", Extracted::default());
        assert_eq!(missing.unwrap_err().code, "occurred_at_missing");
        let input = Input {
            occurred_at: Some("2026-11-02".to_string()),
            ..input
        };
        assert_eq!(candidate_title(input.clone()), "회의");
        let input = Input {
            title: Some("  ".to_string()),
            ..input
        };
        assert_eq!(candidate_title(input), "회의");
    }

    #[test]
    fn 시간대가_없는_시각은_적힌_날짜를_쓴다() {
        assert_eq!(local_date("2026-11-03").as_deref(), Some("2026-11-03"));
        assert_eq!(
            local_date("2026-11-03 10:00:00").as_deref(),
            Some("2026-11-03")
        );
        assert_eq!(
            local_date("2026-11-03T10:20:30.123").as_deref(),
            Some("2026-11-03")
        );
        assert!(local_date("2026-11-03T01:02:03Z").is_some());
        assert_eq!(local_date("2026-11-033"), None);
        assert_eq!(local_date("모름"), None);
    }

    fn candidate_title(input: Input) -> String {
        candidate(input, b"", Extracted::default()).unwrap().title
    }
}

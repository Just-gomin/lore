//! `lore-extractor-markdown`: 마크다운과 텍스트 파일을 소스 노트 후보로 바꾸는 추출기.
//!
//! 본문을 그대로 원문으로 냅니다. 머리말이 있으면 떼어 내고 그 `title`과 `date`를 씁니다.
//! 입력과 출력은 기획서 6장의 추출기 규약을 따릅니다.

use std::process::ExitCode;

use lore_connector_common::ConnectorError;
use lore_connector_common::extractor::{self, Extracted, is_date};
use serde_yaml_ng::Value;

fn main() -> ExitCode {
    extractor::run(|_, _, bytes| extract(bytes))
}

fn extract(bytes: &[u8]) -> Result<Extracted, ConnectorError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| ConnectorError::new("unreadable_file", "UTF-8 글이 아닙니다."))?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let (front, body) = split_front_matter(text);
    let field = |key: &str| {
        front
            .as_ref()
            .and_then(|front| front.get(key))
            .and_then(Value::as_str)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    };
    let title = field("title").or_else(|| first_heading(body));
    let occurred_at = field("date").filter(|date| is_date(date));
    Ok(Extracted {
        title,
        occurred_at,
        text: body.trim().to_string(),
    })
}

/// 머리말(`---`로 둘러싼 YAML)과 본문을 나눕니다. 머리말이 없거나 YAML로 읽지 못하면 전체가 본문입니다.
fn split_front_matter(text: &str) -> (Option<Value>, &str) {
    let normalized = text
        .strip_prefix("---\n")
        .or_else(|| text.strip_prefix("---\r\n"));
    let Some(rest) = normalized else {
        return (None, text);
    };
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        if line.trim_end() == "---" {
            let yaml = &rest[..offset];
            let body = &rest[offset + line.len()..];
            return match serde_yaml_ng::from_str::<Value>(yaml) {
                Ok(value) if value.is_mapping() => (Some(value), body),
                _ => (None, text),
            };
        }
        offset += line.len();
    }
    (None, text)
}

/// 첫 `# ` 제목. 코드 블록 안은 보지 않습니다.
fn first_heading(body: &str) -> Option<String> {
    let mut fenced = false;
    for line in body.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fenced = !fenced;
            continue;
        }
        if !fenced && let Some(title) = line.strip_prefix("# ") {
            let title = title.trim().trim_end_matches('#').trim();
            if !title.is_empty() {
                return Some(title.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 머리말의_제목과_날짜를_쓰고_본문만_낸다() {
        let found =
            extract("---\ntitle: 회의\ndate: 2026-11-03\n---\n\n# 다른 제목\n\n내용\n".as_bytes())
                .unwrap();
        assert_eq!(found.title.as_deref(), Some("회의"));
        assert_eq!(found.occurred_at.as_deref(), Some("2026-11-03"));
        assert_eq!(found.text, "# 다른 제목\n\n내용");
    }

    #[test]
    fn 머리말이_없으면_첫_제목을_쓰고_날짜는_없다() {
        let found = extract("```\n# 코드\n```\n\n# 정산 회의 #\n본문".as_bytes()).unwrap();
        assert_eq!(found.title.as_deref(), Some("정산 회의"));
        assert_eq!(found.occurred_at, None);
        assert!(found.text.starts_with("```"));
    }

    #[test]
    fn 날짜_꼴이_아니거나_머리말이_깨지면_쓰지_않는다() {
        let found = extract("---\ndate: 11월 3일\n---\n본문".as_bytes()).unwrap();
        assert_eq!(found.occurred_at, None);
        let found = extract("---\n: [\n---\n본문".as_bytes()).unwrap();
        assert!(found.text.starts_with("---"));
    }

    #[test]
    fn utf8이_아니면_읽지_못한다() {
        assert_eq!(
            extract(&[0xff, 0xfe, 0x00]).unwrap_err().code,
            "unreadable_file"
        );
    }
}

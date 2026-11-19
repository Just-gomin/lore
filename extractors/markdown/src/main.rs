//! `lore-extractor-markdown`: 마크다운과 텍스트 파일을 소스 노트 후보로 바꾸는 추출기.
//!
//! 본문을 그대로 원문으로 냅니다. 머리말이 있으면 떼어 내고 그 `title`과 `date`를 씁니다.
//! 입력과 출력은 기획서 6장의 추출기 규약을 따릅니다.

use std::process::ExitCode;

use lore_connector_common::ConnectorError;
use lore_connector_common::extractor::{self, Extracted, local_date};
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
    let occurred_at = field("date").and_then(|date| local_date(&date));
    Ok(Extracted {
        title,
        occurred_at,
        text: trim_blank_lines(body).to_string(),
    })
}

/// 앞뒤의 빈 줄만 뺍니다. 첫 줄의 들여쓰기(들여 쓴 코드 블록)는 남깁니다.
fn trim_blank_lines(text: &str) -> &str {
    let start = text
        .split_inclusive('\n')
        .take_while(|line| line.trim().is_empty())
        .map(str::len)
        .sum();
    text[start..].trim_end()
}

/// 머리말(`---`로 둘러싼 YAML)과 본문을 나눕니다. 머리말이 없거나 YAML로 읽지 못하면 전체가 본문입니다.
/// 비었거나 매핑이 아닌 머리말도 떼어 내되, 항목은 매핑에서만 읽습니다.
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
                Ok(value) => (value.is_mapping().then_some(value), body),
                Err(_) => (None, text),
            };
        }
        offset += line.len();
    }
    (None, text)
}

/// 첫 `#` 제목(ATX 1단계). 코드 블록 안은 보지 않습니다.
fn first_heading(body: &str) -> Option<String> {
    // 열린 코드 블록의 울타리 글자와 길이
    let mut fence: Option<(char, usize)> = None;
    for line in body.lines() {
        let indent = line.len() - line.trim_start_matches(' ').len();
        if indent > 3 {
            continue;
        }
        let trimmed = &line[indent..];
        let marker = trimmed.chars().next().filter(|c| *c == '`' || *c == '~');
        if let Some(marker) = marker {
            let run = trimmed.chars().take_while(|c| *c == marker).count();
            if run >= 3 {
                match fence {
                    None => fence = Some((marker, run)),
                    Some((open, length))
                        if open == marker && run >= length && trimmed[run..].trim().is_empty() =>
                    {
                        fence = None
                    }
                    Some(_) => {}
                }
                continue;
            }
        }
        if fence.is_some() {
            continue;
        }
        let Some(rest) = trimmed.strip_prefix('#') else {
            continue;
        };
        if !rest.is_empty() && !rest.starts_with([' ', '\t']) {
            continue;
        }
        let mut title = rest.trim();
        // 끝의 `#` 묶음은 앞에 공백이 있을 때만 닫는 표시입니다(`C#`은 그대로).
        if let Some((head, tail)) = title.rsplit_once([' ', '\t'])
            && !tail.is_empty()
            && tail.chars().all(|c| c == '#')
        {
            title = head.trim_end();
        } else if title.chars().all(|c| c == '#') {
            title = "";
        }
        if !title.is_empty() {
            return Some(title.to_string());
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
    fn 코드_블록의_울타리는_같은_글자와_길이로_닫는다() {
        let found = extract("````\n```\n# inside\n````\n# Real".as_bytes()).unwrap();
        assert_eq!(found.title.as_deref(), Some("Real"));
        let found = extract("~~~\n```py\n~~~\n# Real".as_bytes()).unwrap();
        assert_eq!(found.title.as_deref(), Some("Real"));
    }

    #[test]
    fn 제목의_샵과_들여쓰기를_규칙대로_읽는다() {
        let title = |text: &str| extract(text.as_bytes()).unwrap().title;
        assert_eq!(title("# Learn C#").as_deref(), Some("Learn C#"));
        assert_eq!(title("# 제목 ##").as_deref(), Some("제목"));
        assert_eq!(title("   # 들여씀").as_deref(), Some("들여씀"));
        assert_eq!(title("#\t탭").as_deref(), Some("탭"));
        assert_eq!(title("    # 코드").as_deref(), None);
        assert_eq!(title("#해시태그").as_deref(), None);
    }

    #[test]
    fn 빈_머리말도_떼어_내고_시각이_든_날짜는_날짜만_쓴다() {
        let found = extract("---\n---\n# H\nbody".as_bytes()).unwrap();
        assert_eq!(found.text, "# H\nbody");
        let found = extract("---\ndate: 2026-11-03 10:00:00\n---\n\n    코드".as_bytes()).unwrap();
        assert_eq!(found.occurred_at.as_deref(), Some("2026-11-03"));
        assert_eq!(found.text, "    코드");
    }

    #[test]
    fn utf8이_아니면_읽지_못한다() {
        assert_eq!(
            extract(&[0xff, 0xfe, 0x00]).unwrap_err().code,
            "unreadable_file"
        );
    }
}

//! `lore add source`: 입력 JSON 한 건을 받아 민감도 등급을 적용하고 소스 노트를 만들거나 갱신한다.
//!
//! 정제는 lore 밖에서 끝난다. 여기서는 형식을 검사하고 등급 규칙대로 남길 것을 고른다.

use serde::Serialize;
use serde_json::{Map, Value};

use crate::add::{scalar, today};
use crate::config::{Config, Sensitivity};
use crate::error::{LoreError, Problem};
use crate::source::{self, Kind, Record, is_iso_date, is_slug};

const FIELDS: [&str; 8] = [
    "title",
    "slug",
    "kind",
    "channel",
    "origin",
    "origin_version",
    "occurred_at",
    "body",
];
const KINDS: [&str; 4] = ["discussion", "client-doc", "evidence", "reference"];

/// 형식 검사를 마친 입력.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceInput {
    pub title: String,
    pub slug: String,
    pub kind: String,
    pub channel: String,
    pub origin: String,
    pub origin_version: String,
    pub occurred_at: String,
    /// 민감 채널은 내용을 읽지 않고 넘길 수 있으므로 빠져도 된다.
    pub body: String,
}

/// 소스 노트 파일에 일어난 일.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Change {
    Created,
    Updated,
    Unchanged,
}

impl Change {
    fn label(self) -> &'static str {
        match self {
            Change::Created => "만듦",
            Change::Updated => "바꿈",
            Change::Unchanged => "그대로",
        }
    }
}

/// 추가 결과. JSON 출력이 된다.
#[derive(Debug, Serialize)]
pub struct AddSourceReport {
    pub id: String,
    pub path: String,
    pub sensitivity: Sensitivity,
    pub change: Change,
}

/// 입력 JSON을 읽어 항목을 검사한다. `name`은 문제에 적을 입력의 이름이다.
pub fn parse_input(name: &str, text: &str) -> Result<SourceInput, LoreError> {
    let invalid = |problems| LoreError::InvalidInput { problems };
    let problem =
        |field: Option<&str>, code, message: String| Problem::new(name, field, code, message);
    let value: Value = serde_json::from_str(text)
        .map_err(|error| invalid(vec![problem(None, "invalid_json", error.to_string())]))?;
    let Some(map) = value.as_object() else {
        return Err(invalid(vec![problem(
            None,
            "invalid_json",
            "입력은 JSON 객체 하나여야 합니다.".to_string(),
        )]));
    };

    let mut problems = Vec::new();
    for key in map.keys() {
        if !FIELDS.contains(&key.as_str()) {
            problems.push(problem(
                Some(key),
                "unknown_field",
                format!("{key}는 받지 않는 항목입니다."),
            ));
        }
    }
    let mut field = |key: &'static str, required: bool| -> String {
        match map.get(key) {
            None | Some(Value::Null) if required => {
                problems.push(problem(
                    Some(key),
                    "missing_field",
                    format!("필수 항목 {key}가 없습니다."),
                ));
                String::new()
            }
            None | Some(Value::Null) => String::new(),
            Some(Value::String(text)) => text.clone(),
            Some(_) => {
                problems.push(problem(
                    Some(key),
                    "invalid_type",
                    format!("{key}는 문자열이어야 합니다."),
                ));
                String::new()
            }
        }
    };
    let input = SourceInput {
        title: field("title", true).trim().to_string(),
        slug: field("slug", true),
        kind: field("kind", true),
        channel: field("channel", true),
        origin: field("origin", true),
        origin_version: field("origin_version", true),
        occurred_at: field("occurred_at", true),
        body: field("body", false),
    };
    check_values(name, map, &input, &mut problems);

    if problems.is_empty() {
        Ok(input)
    } else {
        problems.sort();
        Err(invalid(problems))
    }
}

/// 값이 있는 항목의 형식을 본다. 빠진 항목은 이미 보고했다.
fn check_values(
    name: &str,
    map: &Map<String, Value>,
    input: &SourceInput,
    problems: &mut Vec<Problem>,
) {
    let present = |key: &str| map.get(key).is_some_and(Value::is_string);
    let mut bad = |key: &str, message: String| {
        problems.push(Problem::new(name, Some(key), "invalid_value", message));
    };
    if present("slug") && !is_slug(&input.slug) {
        bad(
            "slug",
            "slug는 영문 소문자, 숫자, -로 된 슬러그여야 합니다.".to_string(),
        );
    }
    if present("kind") && !KINDS.contains(&input.kind.as_str()) {
        bad(
            "kind",
            format!("kind는 {} 중 하나여야 합니다.", KINDS.join(", ")),
        );
    }
    if present("occurred_at") && !is_iso_date(&input.occurred_at) {
        bad(
            "occurred_at",
            "occurred_at은 YYYY-MM-DD여야 합니다.".to_string(),
        );
    }
    for key in ["title", "channel", "origin", "origin_version"] {
        if present(key) && map[key].as_str().is_some_and(|text| text.trim().is_empty()) {
            bad(key, format!("{key}가 비어 있습니다."));
        }
    }
}

/// 등급을 적용해 소스 노트를 만들거나 갱신한다. `date`는 `extracted_at`이며 없으면 오늘이다.
pub fn add_source(
    config: &Config,
    input: &SourceInput,
    date: Option<&str>,
) -> Result<AddSourceReport, LoreError> {
    let sensitivity = config.sensitivity(&input.channel);
    if sensitivity == Sensitivity::Excluded {
        return Err(LoreError::ExcludedChannel {
            channel: input.channel.clone(),
        });
    }

    // 레포의 다른 문서에 있는 문제는 소스 노트 추가를 막지 않는다.
    let (corpus, _) = source::read_lenient(config);
    let existing = corpus.of_kind(Kind::Source).find(|doc| match &doc.record {
        Record::Source(note) => note.origin == input.origin,
        _ => false,
    });

    let (id, path) = match existing {
        Some(doc) => (doc.id.clone(), doc.path.clone()),
        None => {
            let id = format!("SRC-{}-{}", input.occurred_at.replace('-', ""), input.slug);
            let path = format!("{}/{}/{id}.md", config.docs, Kind::Source.folder());
            if corpus.get(&id).is_some() || config.root.join(&path).exists() {
                return Err(LoreError::IdExists { id, path });
            }
            (id, path)
        }
    };

    if let Some(Record::Source(note)) = existing.map(|doc| &doc.record)
        && note.origin_version == input.origin_version
        && note.sensitivity == sensitivity.name()
    {
        return Ok(AddSourceReport {
            id,
            path,
            sensitivity,
            change: Change::Unchanged,
        });
    }

    let extracted_at = date.map_or_else(today, str::to_string);
    let text = render(&id, input, &extracted_at, sensitivity);
    let mut problems = Vec::new();
    source::parse_document(
        &path,
        &text,
        source::sha256_hex(text.as_bytes()),
        Kind::Source,
        &mut problems,
    );
    if !problems.is_empty() {
        problems.sort();
        return Err(LoreError::InvalidSources { problems });
    }

    let file = config.root.join(&path);
    let io_error = |error: std::io::Error| LoreError::Io {
        path: path.clone(),
        message: error.to_string(),
    };
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(io_error)?;
    }
    std::fs::write(&file, &text).map_err(io_error)?;

    Ok(AddSourceReport {
        id,
        path,
        sensitivity,
        change: if existing.is_some() {
            Change::Updated
        } else {
            Change::Created
        },
    })
}

/// 머리말은 기획서 2장의 소스 노트 템플릿과 같다. 민감 등급은 본문을 남기지 않는다.
fn render(id: &str, input: &SourceInput, extracted_at: &str, sensitivity: Sensitivity) -> String {
    let mut text = format!(
        "---\n\
         id: {id}\n\
         title: {}\n\
         kind: {} # discussion | client-doc | evidence | reference\n\
         origin: {}\n\
         origin_version: {}\n\
         extracted_at: {extracted_at}\n\
         sensitivity: {} # normal | sensitive\n\
         ---\n",
        scalar(&input.title),
        input.kind,
        scalar(&input.origin),
        scalar(&input.origin_version),
        sensitivity.name(),
    );
    let body = input.body.trim();
    if sensitivity == Sensitivity::Normal && !body.is_empty() {
        text.push('\n');
        text.push_str(body);
        text.push('\n');
    }
    text
}

/// 사람용 추가 결과.
pub fn render_report(report: &AddSourceReport) -> String {
    let mut text = format!(
        "lore add source: {} ({})\n  문서  {}\n  등급  {}",
        report.id,
        report.change.label(),
        report.path,
        report.sensitivity.name()
    );
    if report.change != Change::Unchanged {
        text.push_str("\n\nlore build로 인덱스를 다시 만드세요.");
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 빠지거나_틀린_항목을_모두_알려_준다() {
        let text = r#"{"title": "회의", "slug": "Retry", "kind": "memo", "occurred_at": "2026-11-3", "extra": 1}"#;
        let Err(LoreError::InvalidInput { problems }) = parse_input("-", text) else {
            panic!("InvalidInput이 아니다");
        };
        let found: Vec<(&str, &str)> = problems
            .iter()
            .map(|p| (p.field.as_deref().unwrap_or(""), p.code))
            .collect();
        assert_eq!(
            found,
            [
                ("channel", "missing_field"),
                ("extra", "unknown_field"),
                ("kind", "invalid_value"),
                ("occurred_at", "invalid_value"),
                ("origin", "missing_field"),
                ("origin_version", "missing_field"),
                ("slug", "invalid_value"),
            ]
        );
    }

    #[test]
    fn 본문은_빠져도_된다() {
        let text = r#"{"title": "계약 검토", "slug": "contract", "kind": "client-doc", "channel": "contracts",
            "origin": "https://drive.google.com/x", "origin_version": "v3", "occurred_at": "2026-11-03"}"#;
        assert_eq!(parse_input("-", text).unwrap().body, "");
    }
}

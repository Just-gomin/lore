//! `lore add decision`, `lore add incident`: 내장 템플릿으로 기록 파일을 만들고 ID를 발급한다.
//!
//! 본문은 템플릿의 제목만 둔다. 본문을 채우는 것은 에이전트다.

use serde::Serialize;
use serde_yaml_ng::Value;

use crate::config::Config;
use crate::error::LoreError;
use crate::source::{self, Kind};

const DECISION_BODY: &str = include_str!("../templates/decision.md");
const INCIDENT_BODY: &str = include_str!("../templates/incident.md");

/// 만들 기록. 인자 해석이 끝난 값이다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewRecord {
    /// [`Kind::Decision`] 또는 [`Kind::Incident`]
    pub kind: Kind,
    pub slug: String,
    pub title: String,
    pub specs: Vec<String>,
    pub sources: Vec<String>,
    /// `YYYY-MM-DD`. 없으면 오늘이다.
    pub date: Option<String>,
    /// 결정만.
    pub supersedes: Option<String>,
    /// 결정만. `proposed` 또는 `accepted`.
    pub status: String,
    /// 결정만. `stated` 또는 `inferred`.
    pub confidence: String,
    /// 사고만.
    pub decisions: Vec<String>,
}

/// 추가 결과. JSON 출력이 된다.
#[derive(Debug, Serialize)]
pub struct AddReport {
    pub id: String,
    pub kind: &'static str,
    pub path: String,
    /// 대체되는 결정이 가리키던 스펙. 본문에 고칠 문장이 있을 수 있다.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub specs_to_review: Vec<String>,
}

/// 실행 환경의 로컬 시간대로 본 오늘. `YYYY-MM-DD`.
pub fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

pub fn add(config: &Config, record: &NewRecord) -> Result<AddReport, LoreError> {
    let date = record.date.clone().unwrap_or_else(today);
    let prefix = match record.kind {
        Kind::Decision => "ADR",
        _ => "INC",
    };
    let id = format!("{prefix}-{}-{}", date.replace('-', ""), record.slug);
    let path = format!("{}/{}/{id}.md", config.docs, record.kind.folder());

    // 레포의 다른 문서에 있는 문제는 새 기록을 막지 않는다.
    let (corpus, _) = source::read_lenient(config);
    if corpus.get(&id).is_some() || config.root.join(&path).exists() {
        return Err(LoreError::IdExists { id, path });
    }

    let text = render(record, &id);
    let mut problems = Vec::new();
    let doc = source::parse_document(
        &path,
        &text,
        source::sha256_hex(text.as_bytes()),
        record.kind,
        &mut problems,
    );
    if let Some(doc) = &doc {
        problems.extend(source::check_new(&corpus, doc));
    }
    if !problems.is_empty() {
        problems.sort();
        problems.dedup();
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

    let specs_to_review = record
        .supersedes
        .as_deref()
        .and_then(|old| corpus.get(old))
        .and_then(|old| old.decision())
        .map(|old| old.specs.clone())
        .unwrap_or_default();
    Ok(AddReport {
        id,
        kind: record.kind.node_kind(),
        path,
        specs_to_review,
    })
}

/// 머리말과 템플릿 본문. 머리말의 모양은 블로그 레포의 손으로 쓴 템플릿과 같다.
fn render(record: &NewRecord, id: &str) -> String {
    let mut text = format!("---\nid: {id}\ntitle: {}\n", scalar(&record.title));
    let body = match record.kind {
        Kind::Decision => {
            text.push_str(&format!(
                "status: {} # proposed | accepted\n",
                record.status
            ));
            if let Some(old) = &record.supersedes {
                text.push_str(&format!("supersedes: {}\n", scalar(old)));
            }
            text.push_str(&format!("specs: {}\n", list(&record.specs)));
            text.push_str(&format!("sources: {}\n", list(&record.sources)));
            text.push_str(&format!(
                "confidence: {} # stated | inferred\n",
                record.confidence
            ));
            DECISION_BODY
        }
        _ => {
            text.push_str(&format!(
                "specs: {} # 사고가 난 기능\n",
                list(&record.specs)
            ));
            text.push_str(&format!(
                "decisions: {} # 이 사고로 내린 결정\n",
                list(&record.decisions)
            ));
            text.push_str(&format!("sources: {}\n", list(&record.sources)));
            INCIDENT_BODY
        }
    };
    text.push_str("---\n\n");
    text.push_str(body);
    text
}

/// YAML에 그대로 적어도 같은 문자열로 읽히면 그대로, 아니면 따옴표로 감싼다.
fn scalar(text: &str) -> String {
    let plain = format!("k: {text}\n");
    if reads_back(&plain, |value| value.as_str() == Some(text)) {
        text.to_string()
    } else {
        quoted(text)
    }
}

/// `[a, b]` 꼴의 목록. 항목마다 [`scalar`]와 같은 기준으로 따옴표를 정한다.
fn list(items: &[String]) -> String {
    let items: Vec<String> = items
        .iter()
        .map(|item| {
            let plain = format!("k: [{item}]\n");
            let single = |value: &Value| {
                value
                    .as_sequence()
                    .is_some_and(|seq| seq.len() == 1 && seq[0].as_str() == Some(item))
            };
            if reads_back(&plain, single) {
                item.clone()
            } else {
                quoted(item)
            }
        })
        .collect();
    format!("[{}]", items.join(", "))
}

fn reads_back(yaml: &str, check: impl Fn(&Value) -> bool) -> bool {
    serde_yaml_ng::from_str::<Value>(yaml)
        .ok()
        .and_then(|value| value.get("k").map(&check))
        .unwrap_or(false)
}

/// JSON 문자열은 YAML의 큰따옴표 문자열로도 같게 읽힌다.
fn quoted(text: &str) -> String {
    serde_json::to_string(text).expect("문자열은 항상 JSON으로 바꿀 수 있다")
}

/// 사람용 추가 결과.
pub fn render_report(report: &AddReport) -> String {
    let mut text = format!("lore add: {}\n  문서  {}", report.id, report.path);
    if !report.specs_to_review.is_empty() {
        text.push_str(&format!(
            "\n  본문을 확인할 스펙  {}",
            report.specs_to_review.join(", ")
        ));
    }
    text.push_str("\n\n본문을 채운 뒤 lore build로 인덱스를 다시 만드세요.");
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 읽기에_문제없는_글자는_따옴표_없이_쓴다() {
        assert_eq!(
            scalar("재시도를 최대 5회로 늘린다"),
            "재시도를 최대 5회로 늘린다"
        );
        assert_eq!(scalar("a: b"), "\"a: b\"");
        assert_eq!(scalar("#1 원인"), "\"#1 원인\"");
        assert_eq!(scalar("true"), "\"true\"");
    }

    #[test]
    fn 목록_항목도_필요할_때만_따옴표로_감싼다() {
        assert_eq!(list(&[]), "[]");
        assert_eq!(
            list(&[
                "SRC-20261103-retry-meeting".to_string(),
                "https://github.com/org/repo/pull/231".to_string()
            ]),
            "[SRC-20261103-retry-meeting, https://github.com/org/repo/pull/231]"
        );
        assert_eq!(list(&["a,b".to_string()]), "[\"a,b\"]");
    }
}

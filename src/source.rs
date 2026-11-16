//! 소스 폴더의 마크다운을 찾아 머리말을 종류별로 읽는다.
//!
//! 문제는 파일마다 멈추지 않고 모두 모아 한 번에 돌려준다.

use std::collections::BTreeMap;
use std::path::Path;

use serde_yaml_ng::Value;
use sha2::{Digest, Sha256};

use crate::config::Config;
use crate::error::{LoreError, Problem};
use crate::frontmatter::{self, Fields};

/// 소스 문서의 종류. 폴더 이름으로 정해진다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    Spec,
    Decision,
    Incident,
    Source,
}

impl Kind {
    pub const ALL: [Kind; 4] = [Kind::Spec, Kind::Decision, Kind::Incident, Kind::Source];

    /// 소스 폴더 안의 하위 폴더 이름.
    pub fn folder(self) -> &'static str {
        match self {
            Kind::Spec => "specs",
            Kind::Decision => "decisions",
            Kind::Incident => "incidents",
            Kind::Source => "sources",
        }
    }

    /// 디비와 JSON 출력에 쓰는 노드 종류 이름.
    pub fn node_kind(self) -> &'static str {
        match self {
            Kind::Spec => "feature",
            Kind::Decision => "decision",
            Kind::Incident => "incident",
            Kind::Source => "source",
        }
    }

    /// ID 앞에 붙는 종류 표시. 스펙은 자유로운 슬러그를 쓴다.
    fn id_prefix(self) -> Option<&'static str> {
        match self {
            Kind::Spec => None,
            Kind::Decision => Some("ADR"),
            Kind::Incident => Some("INC"),
            Kind::Source => Some("SRC"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spec {
    pub paths: Vec<String>,
    pub tests: Vec<String>,
    pub depends_on: Vec<String>,
    pub parent: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    /// `proposed` 또는 `accepted`. 대체 여부는 여기에 없고 대체함 엣지에서 계산한다.
    pub status: String,
    pub supersedes: Option<String>,
    pub specs: Vec<String>,
    /// 소스 노트 ID 또는 외부 링크.
    pub sources: Vec<String>,
    pub confidence: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Incident {
    pub specs: Vec<String>,
    pub decisions: Vec<String>,
    pub sources: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceNote {
    pub kind: String,
    pub origin: String,
    pub origin_version: String,
    pub extracted_at: String,
    pub sensitivity: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Record {
    Spec(Spec),
    Decision(Decision),
    Incident(Incident),
    Source(SourceNote),
}

/// 읽어 낸 소스 문서 하나.
#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    pub id: String,
    pub title: String,
    pub kind: Kind,
    /// 프로젝트 홈 기준 경로. `/`로 구분한다.
    pub path: String,
    /// 머리말을 뺀 본문.
    pub body: String,
    /// 파일 전체의 SHA-256.
    pub hash: String,
    /// 머리말 전체. 키 순서로 정렬된다.
    pub frontmatter: serde_json::Value,
    pub record: Record,
}

impl Document {
    /// 결정, 사고, 소스 노트의 ID에 든 날짜(`YYYY-MM-DD`). 스펙은 없다.
    pub fn date(&self) -> Option<String> {
        id_date(&self.id)
    }

    pub fn spec(&self) -> Option<&Spec> {
        match &self.record {
            Record::Spec(spec) => Some(spec),
            _ => None,
        }
    }

    pub fn decision(&self) -> Option<&Decision> {
        match &self.record {
            Record::Decision(decision) => Some(decision),
            _ => None,
        }
    }
}

/// 소스 전체. 문서는 ID 순으로 정렬되어 있다.
#[derive(Debug, Clone, PartialEq)]
pub struct Corpus {
    pub documents: Vec<Document>,
}

impl Corpus {
    pub fn get(&self, id: &str) -> Option<&Document> {
        self.documents
            .binary_search_by(|doc| doc.id.as_str().cmp(id))
            .ok()
            .map(|index| &self.documents[index])
    }

    pub fn of_kind(&self, kind: Kind) -> impl Iterator<Item = &Document> {
        self.documents.iter().filter(move |doc| doc.kind == kind)
    }
}

/// 외부 링크인지 본다. 소스 노트 ID가 아닌 `sources` 항목은 링크여야 한다.
pub fn is_link(reference: &str) -> bool {
    reference.contains("://")
}

/// 소스 폴더를 모두 읽는다. 문제가 하나라도 있으면 모든 문제를 돌려준다.
pub fn read(config: &Config) -> Result<Corpus, LoreError> {
    let mut problems = Vec::new();
    let mut documents = Vec::new();

    for kind in Kind::ALL {
        let folder = format!("{}/{}", config.docs, kind.folder());
        let mut files = Vec::new();
        collect_markdown(&config.root, &folder, &mut files, &mut problems);
        files.sort();
        for path in files {
            if let Some(doc) = read_document(&config.root, &path, kind, &mut problems) {
                documents.push(doc);
            }
        }
    }

    documents.sort_by(|a, b| a.id.cmp(&b.id).then_with(|| a.path.cmp(&b.path)));
    check_duplicates(&documents, &mut problems);
    documents.dedup_by(|b, a| a.id == b.id);

    let corpus = Corpus { documents };
    check_references(&corpus, &mut problems);

    if problems.is_empty() {
        Ok(corpus)
    } else {
        problems.sort();
        problems.dedup();
        Err(LoreError::InvalidSources { problems })
    }
}

/// `folder` 아래의 `.md` 파일을 찾는다. `_`나 `.`로 시작하는 항목은 건너뛴다.
fn collect_markdown(root: &Path, folder: &str, out: &mut Vec<String>, problems: &mut Vec<Problem>) {
    let entries = match std::fs::read_dir(root.join(folder)) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
        Err(error) => {
            problems.push(Problem::new(folder, None, "unreadable", error.to_string()));
            return;
        }
    };

    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                problems.push(Problem::new(folder, None, "unreadable", error.to_string()));
                continue;
            }
        };
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('_') || name.starts_with('.') {
            continue;
        }
        let path = format!("{folder}/{name}");
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        if is_dir {
            collect_markdown(root, &path, out, problems);
        } else if name.ends_with(".md") {
            out.push(path);
        }
    }
}

fn read_document(
    root: &Path,
    path: &str,
    kind: Kind,
    problems: &mut Vec<Problem>,
) -> Option<Document> {
    let bytes = match std::fs::read(root.join(path)) {
        Ok(bytes) => bytes,
        Err(error) => {
            problems.push(Problem::new(path, None, "unreadable", error.to_string()));
            return None;
        }
    };
    let hash = sha256_hex(&bytes);
    let text = match String::from_utf8(bytes) {
        Ok(text) => text,
        Err(_) => {
            problems.push(Problem::new(
                path,
                None,
                "unreadable",
                "UTF-8 텍스트가 아닙니다.",
            ));
            return None;
        }
    };

    let Some((yaml, body)) = frontmatter::split(&text) else {
        problems.push(Problem::new(
            path,
            None,
            "missing_frontmatter",
            "파일 맨 앞에 --- 로 감싼 머리말이 없습니다.",
        ));
        return None;
    };
    let value: Value = match serde_yaml_ng::from_str(yaml) {
        Ok(value) => value,
        Err(error) => {
            problems.push(Problem::new(path, None, "invalid_yaml", error.to_string()));
            return None;
        }
    };
    let Some(map) = value.as_mapping() else {
        problems.push(Problem::new(
            path,
            None,
            "invalid_yaml",
            "머리말은 키와 값의 묶음이어야 합니다.",
        ));
        return None;
    };

    let mut fields = Fields::new(map, path, problems);
    let id = fields.string("id", true);
    let title = fields.string("title", true);
    let record = match kind {
        Kind::Spec => Record::Spec(Spec {
            paths: fields.list("paths", true),
            tests: fields.list("tests", false),
            depends_on: fields.list("depends_on", false),
            parent: fields.string("parent", false),
        }),
        Kind::Decision => Record::Decision(Decision {
            status: fields
                .choice("status", &["proposed", "accepted"], true)
                .unwrap_or_default(),
            supersedes: fields.string("supersedes", false),
            specs: fields.list("specs", false),
            sources: fields.list("sources", false),
            confidence: fields
                .choice("confidence", &["stated", "inferred"], true)
                .unwrap_or_default(),
        }),
        Kind::Incident => Record::Incident(Incident {
            specs: fields.list("specs", false),
            decisions: fields.list("decisions", false),
            sources: fields.list("sources", false),
        }),
        Kind::Source => Record::Source(SourceNote {
            kind: fields
                .choice(
                    "kind",
                    &["discussion", "client-doc", "evidence", "reference"],
                    true,
                )
                .unwrap_or_default(),
            origin: fields.string("origin", true).unwrap_or_default(),
            origin_version: fields.string("origin_version", true).unwrap_or_default(),
            extracted_at: fields.string("extracted_at", true).unwrap_or_default(),
            sensitivity: fields
                .choice("sensitivity", &["normal", "sensitive"], true)
                .unwrap_or_default(),
        }),
    };
    fields.finish();

    if let Record::Source(note) = &record
        && !note.extracted_at.is_empty()
        && !is_iso_date(&note.extracted_at)
    {
        problems.push(Problem::new(
            path,
            Some("extracted_at"),
            "invalid_value",
            format!(
                "extracted_at은 YYYY-MM-DD여야 합니다. '{}'는 받지 않습니다.",
                note.extracted_at
            ),
        ));
    }

    let id = id?;
    check_id(&id, kind, path, problems);

    Some(Document {
        id,
        title: title.unwrap_or_default(),
        kind,
        path: path.to_string(),
        body: body.trim_start_matches(['\r', '\n']).to_string(),
        hash,
        frontmatter: serde_json::to_value(&value).unwrap_or(serde_json::Value::Null),
        record,
    })
}

/// ID 형식과 파일 이름을 검사한다.
fn check_id(id: &str, kind: Kind, path: &str, problems: &mut Vec<Problem>) {
    let valid = match kind.id_prefix() {
        None => is_slug(id),
        Some(prefix) => id
            .strip_prefix(prefix)
            .and_then(|rest| rest.strip_prefix('-'))
            .and_then(|rest| rest.split_once('-'))
            .is_some_and(|(date, slug)| is_compact_date(date) && is_slug(slug)),
    };
    if !valid {
        let expected = match kind.id_prefix() {
            None => "영문 소문자, 숫자, -로 된 슬러그".to_string(),
            Some(prefix) => format!("{prefix}-YYYYMMDD-슬러그"),
        };
        problems.push(Problem::new(
            path,
            Some("id"),
            "invalid_id",
            format!("ID '{id}'는 {expected} 형식이어야 합니다."),
        ));
    }

    let stem = path
        .rsplit('/')
        .next()
        .and_then(|name| name.strip_suffix(".md"))
        .unwrap_or_default();
    if stem != id {
        problems.push(Problem::new(
            path,
            Some("id"),
            "id_mismatch",
            format!("ID '{id}'와 파일 이름 '{stem}'이 다릅니다. 파일 이름은 ID와 같아야 합니다."),
        ));
    }
}

fn check_duplicates(documents: &[Document], problems: &mut Vec<Problem>) {
    let mut seen: BTreeMap<&str, &str> = BTreeMap::new();
    for doc in documents {
        if let Some(first) = seen.get(doc.id.as_str()) {
            problems.push(Problem::new(
                &doc.path,
                Some("id"),
                "duplicate_id",
                format!("ID '{}'를 {first}에서 이미 씁니다.", doc.id),
            ));
        } else {
            seen.insert(&doc.id, &doc.path);
        }
    }
}

/// 다른 문서를 가리키는 항목이 실제 문서를 가리키는지 본다.
fn check_references(corpus: &Corpus, problems: &mut Vec<Problem>) {
    let ids: BTreeMap<&str, Kind> = corpus
        .documents
        .iter()
        .map(|doc| (doc.id.as_str(), doc.kind))
        .collect();

    let mut check = |doc: &Document, field: &str, target: &str, expected: Kind| {
        let message = if target == doc.id {
            Some(format!("{field}가 자기 자신을 가리킵니다."))
        } else {
            match ids.get(target) {
                Some(kind) if *kind == expected => None,
                Some(_) => Some(format!(
                    "'{target}'는 {} 문서가 아닙니다.",
                    expected.node_kind()
                )),
                None => Some(format!(
                    "'{target}'라는 {} 문서가 없습니다.",
                    expected.node_kind()
                )),
            }
        };
        if let Some(message) = message {
            let code = if ids.contains_key(target) {
                "invalid_reference"
            } else {
                "unknown_reference"
            };
            problems.push(Problem::new(&doc.path, Some(field), code, message));
        }
    };

    let check_sources = |doc: &Document, sources: &[String], problems: &mut Vec<Problem>| {
        for source in sources {
            if is_link(source) {
                continue;
            }
            if source.starts_with("SRC-") {
                match ids.get(source.as_str()) {
                    Some(Kind::Source) => {}
                    _ => problems.push(Problem::new(
                        &doc.path,
                        Some("sources"),
                        "unknown_reference",
                        format!("'{source}'라는 source 문서가 없습니다."),
                    )),
                }
            } else {
                problems.push(Problem::new(
                    &doc.path,
                    Some("sources"),
                    "invalid_reference",
                    format!("'{source}'는 소스 노트 ID(SRC-...)나 링크(https://...)여야 합니다."),
                ));
            }
        }
    };

    let mut source_problems = Vec::new();
    for doc in &corpus.documents {
        match &doc.record {
            Record::Spec(spec) => {
                for target in &spec.depends_on {
                    check(doc, "depends_on", target, Kind::Spec);
                }
                if let Some(parent) = &spec.parent {
                    check(doc, "parent", parent, Kind::Spec);
                }
            }
            Record::Decision(decision) => {
                for target in &decision.specs {
                    check(doc, "specs", target, Kind::Spec);
                }
                if let Some(target) = &decision.supersedes {
                    check(doc, "supersedes", target, Kind::Decision);
                }
                check_sources(doc, &decision.sources, &mut source_problems);
            }
            Record::Incident(incident) => {
                for target in &incident.specs {
                    check(doc, "specs", target, Kind::Spec);
                }
                for target in &incident.decisions {
                    check(doc, "decisions", target, Kind::Decision);
                }
                check_sources(doc, &incident.sources, &mut source_problems);
            }
            Record::Source(_) => {}
        }
    }
    problems.append(&mut source_problems);
}

/// `ADR-20261105-retry-queue`에서 `2026-11-05`를 꺼낸다.
pub fn id_date(id: &str) -> Option<String> {
    let mut parts = id.splitn(3, '-');
    let prefix = parts.next()?;
    let date = parts.next()?;
    parts.next()?;
    if !matches!(prefix, "ADR" | "INC" | "SRC") || !is_compact_date(date) {
        return None;
    }
    Some(format!("{}-{}-{}", &date[..4], &date[4..6], &date[6..8]))
}

fn is_slug(text: &str) -> bool {
    !text.is_empty()
        && !text.starts_with('-')
        && !text.ends_with('-')
        && text
            .chars()
            .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-')
}

/// `YYYYMMDD`. 달과 날의 범위만 본다.
fn is_compact_date(text: &str) -> bool {
    text.len() == 8
        && text.chars().all(|ch| ch.is_ascii_digit())
        && matches!(text[4..6].parse::<u32>(), Ok(1..=12))
        && matches!(text[6..8].parse::<u32>(), Ok(1..=31))
}

/// `YYYY-MM-DD`.
fn is_iso_date(text: &str) -> bool {
    let parts: Vec<&str> = text.split('-').collect();
    parts.len() == 3
        && parts[0].len() == 4
        && parts[1].len() == 2
        && parts[2].len() == 2
        && is_compact_date(&parts.concat())
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest.iter() {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 아이디에서_날짜를_꺼낸다() {
        assert_eq!(
            id_date("ADR-20261105-retry-queue").as_deref(),
            Some("2026-11-05")
        );
        assert_eq!(id_date("payment-retry"), None);
        assert_eq!(id_date("ADR-20261305-bad"), None);
    }

    #[test]
    fn 날짜와_슬러그_형식을_검사한다() {
        assert!(is_iso_date("2026-11-04"));
        assert!(!is_iso_date("2026-1-04"));
        assert!(is_slug("payment-retry"));
        assert!(!is_slug("Payment"));
        assert!(!is_slug("-a"));
    }
}

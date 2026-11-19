//! `lore why`: ID, 파일 경로, 심볼에 연결된 결정과 사고를 조회한다.
//!
//! 디비만 읽는다. 디비가 작업 트리보다 오래됐으면 먼저 다시 만든다([`query::open_target`]).
//! 결정과 사고는 스펙에만 연결되므로 파일과 심볼은 스펙을 거쳐 답한다. 그 스펙의 기록 중
//! 본문에 대상이 언급된 것은 묶음 안에서 위로 올린다. 언급은 사람이 선언한 연결이 아니라 추론이다.

use std::collections::BTreeSet;
use std::path::Path;

use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;

use crate::config::Config;
use crate::db::database_error;
use crate::error::LoreError;
use crate::query;
use crate::source::{id_date, is_link};

/// 얼마나 깊이 내려가 보여 줄지.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Depth {
    /// 한 줄 목록.
    Index,
    /// 문서 본문까지.
    Doc,
    /// 연결된 소스 노트까지.
    Source,
}

impl Depth {
    pub const NAMES: [&'static str; 3] = ["index", "doc", "source"];

    pub fn from_name(name: &str) -> Option<Depth> {
        match name {
            "index" => Some(Depth::Index),
            "doc" => Some(Depth::Doc),
            "source" => Some(Depth::Source),
            _ => None,
        }
    }
}

/// 조회 결과. JSON 출력이 된다.
#[derive(Debug, Serialize)]
pub struct WhyReport {
    pub target: Target,
    pub depth: Depth,
    /// 조회 전에 디비를 다시 만들었는지.
    pub rebuilt: bool,
    /// 대상이 기능이나 파일일 때 그 기능들. 기록이면 비어 있다.
    pub features: Vec<Feature>,
    /// 대상이 결정, 사고, 소스 노트일 때 그 기록 하나. 기능이나 파일이면 비어 있다.
    pub records: Vec<Record>,
}

#[derive(Debug, Serialize)]
pub struct Target {
    /// 사용자가 넘긴 그대로의 값.
    pub input: String,
    /// 해석한 노드 ID.
    pub id: String,
    /// 노드 종류: `feature`, `decision`, `incident`, `source`, `file`, `symbol`
    pub kind: String,
    /// 언급으로 보는 이름. 파일이면 경로, 심볼이면 이름, 심볼 ID, 담은 파일의 경로. 찾은 기록이 없어도 채운다.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub mentions: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct Feature {
    pub id: String,
    pub title: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// 시간순. 대체된 결정도 함께 든다. 파일이나 심볼로 물으면 본문에 언급된 유효한 기록이 먼저다.
    pub decisions: Vec<Record>,
    /// 시간순. 파일이나 심볼로 물으면 본문에 언급된 기록이 먼저다.
    pub incidents: Vec<Record>,
}

#[derive(Debug, Serialize)]
pub struct Record {
    pub id: String,
    /// `decision`, `incident`, `source`
    pub kind: String,
    /// ID에 든 날짜. `YYYY-MM-DD`.
    pub date: Option<String>,
    pub title: String,
    pub path: String,
    /// 결정의 머리말 `status`. 대체 여부는 `superseded_by`로 본다.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supersedes: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub superseded_by: Option<String>,
    /// 이 기록이 가리키는 기능.
    pub features: Vec<String>,
    /// 결정이면 이 결정을 낳은 사고, 사고면 이 사고가 낳은 결정.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub related: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// `--depth source`일 때 근거가 된 소스 노트.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sources: Option<Vec<Record>>,
    /// `--depth source`일 때 머리말 `sources`의 외부 링크.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub links: Option<Vec<String>>,
    /// 본문에 언급된 대상의 이름. 이 연결의 근거는 추론이다.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub mentioned: Vec<String>,
}

impl Record {
    fn is_superseded(&self) -> bool {
        self.superseded_by.is_some()
    }

    /// 사람용 상태 표시. 결정에만 있다.
    fn status_label(&self) -> Option<&'static str> {
        if self.kind != "decision" {
            return None;
        }
        Some(if self.is_superseded() {
            "[대체됨]"
        } else if self.status.as_deref() == Some("proposed") {
            "[제안]"
        } else {
            "[유효]"
        })
    }
}

/// 대상을 조회한다. 디비가 없거나 작업 트리와 다르면 먼저 다시 만든다.
pub fn why(config: &Config, cwd: &Path, input: &str, depth: Depth) -> Result<WhyReport, LoreError> {
    let (conn, rebuilt, id, kind) = query::open_target(config, cwd, input)?;

    let reader = Reader { conn: &conn, depth };
    let mut mentions = Vec::new();
    let (features, records) = match kind.as_str() {
        "file" => {
            mentions.push(id.clone());
            (reader.features_of_file(&id, &mentions)?, Vec::new())
        }
        // 심볼은 속함 엣지로 담은 파일에 닿고, 거기서 파일처럼 스펙에 닿는다.
        "symbol" => {
            let file = reader.targets_of(&id, "belongs_to")?.into_iter().next();
            let name = file
                .as_deref()
                .and_then(|file| id.strip_prefix(file))
                .and_then(|rest| rest.strip_prefix('#'))
                .unwrap_or(&id);
            // `default`는 흔한 낱말이라 이름으로는 찾지 않는다.
            if name != "default" {
                mentions.push(name.to_string());
            }
            mentions.push(id.clone());
            match file {
                Some(file) => {
                    mentions.push(file.clone());
                    (reader.features_of_file(&file, &mentions)?, Vec::new())
                }
                None => (Vec::new(), Vec::new()),
            }
        }
        "feature" => (vec![reader.feature(&id, &[])?], Vec::new()),
        _ => (Vec::new(), vec![reader.record(&id)?]),
    };

    Ok(WhyReport {
        target: Target {
            input: input.to_string(),
            id,
            kind,
            mentions,
        },
        depth,
        rebuilt,
        features,
        records,
    })
}

struct Reader<'a> {
    conn: &'a Connection,
    depth: Depth,
}

impl Reader<'_> {
    /// `sql`의 첫 열을 문자열 목록으로 읽는다.
    fn ids(&self, sql: &str, id: &str) -> Result<Vec<String>, LoreError> {
        let mut stmt = self.conn.prepare_cached(sql).map_err(database_error)?;
        let rows = stmt
            .query_map([id], |row| row.get::<_, String>(0))
            .map_err(database_error)?;
        rows.collect::<Result<_, _>>().map_err(database_error)
    }

    fn sources_of(&self, id: &str, edge: &str) -> Result<Vec<String>, LoreError> {
        self.ids(
            &format!("SELECT src FROM edges WHERE dst = ?1 AND kind = '{edge}' ORDER BY src"),
            id,
        )
    }

    fn targets_of(&self, id: &str, edge: &str) -> Result<Vec<String>, LoreError> {
        self.ids(
            &format!("SELECT dst FROM edges WHERE src = ?1 AND kind = '{edge}' ORDER BY dst"),
            id,
        )
    }

    /// 노드의 제목, 경로, 머리말.
    fn node(&self, id: &str) -> Result<(String, String, String, Option<String>), LoreError> {
        self.conn
            .query_row(
                "SELECT kind, ifnull(title, ''), source_path, frontmatter FROM nodes WHERE id = ?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .map_err(database_error)
    }

    fn body(&self, id: &str) -> Result<Option<String>, LoreError> {
        if self.depth == Depth::Index {
            return Ok(None);
        }
        self.conn
            .query_row("SELECT body FROM content WHERE id = ?1", [id], |row| {
                row.get(0)
            })
            .optional()
            .map_err(database_error)
    }

    /// 파일이 구현하거나 검증하는 기능.
    fn features_of_file(&self, file: &str, mentions: &[String]) -> Result<Vec<Feature>, LoreError> {
        let ids: BTreeSet<String> = self
            .targets_of(file, "implements")?
            .into_iter()
            .chain(self.targets_of(file, "verifies")?)
            .collect();
        ids.iter().map(|id| self.feature(id, mentions)).collect()
    }

    /// 기록 본문에 든 `mentions`. 깊이와 상관없이 본문을 읽는다.
    fn mentioned_in(&self, id: &str, mentions: &[String]) -> Result<Vec<String>, LoreError> {
        if mentions.is_empty() {
            return Ok(Vec::new());
        }
        let body: Option<String> = self
            .conn
            .query_row("SELECT body FROM content WHERE id = ?1", [id], |row| {
                row.get(0)
            })
            .optional()
            .map_err(database_error)?;
        let body = body.unwrap_or_default();
        Ok(mentions
            .iter()
            .filter(|name| mentions_word(&body, name))
            .cloned()
            .collect())
    }

    fn feature(&self, id: &str, mentions: &[String]) -> Result<Feature, LoreError> {
        let (_, title, path, _) = self.node(id)?;

        // 대체된 결정이 이 기능을 가리키지 않더라도 이력으로 함께 보여 준다.
        let mut decisions: BTreeSet<String> = self.sources_of(id, "decides")?.into_iter().collect();
        let mut queue: Vec<String> = decisions.iter().cloned().collect();
        while let Some(decision) = queue.pop() {
            for old in self.targets_of(&decision, "supersedes")? {
                if decisions.insert(old.clone()) {
                    queue.push(old);
                }
            }
        }
        let incidents = self.sources_of(id, "occurred_in")?;

        let mut decisions = self.records(decisions.iter())?;
        let mut incidents = self.records(incidents.iter())?;
        for record in decisions.iter_mut().chain(incidents.iter_mut()) {
            record.mentioned = self.mentioned_in(&record.id, mentions)?;
        }
        // 언급된 기록을 위로 올린다. 대체된 결정은 표시만 하고 올리지 않는다. 정렬은 안정적이라 시간순이 남는다.
        for records in [&mut decisions, &mut incidents] {
            records.sort_by_key(|record| record.mentioned.is_empty() || record.is_superseded());
        }

        Ok(Feature {
            id: id.to_string(),
            title,
            path,
            body: self.body(id)?,
            decisions,
            incidents,
        })
    }

    /// 기록 여럿을 날짜, ID 순으로 읽는다.
    fn records<'a>(&self, ids: impl Iterator<Item = &'a String>) -> Result<Vec<Record>, LoreError> {
        let mut records = ids
            .map(|id| self.record(id))
            .collect::<Result<Vec<_>, _>>()?;
        records.sort_by(|a, b| (&a.date, &a.id).cmp(&(&b.date, &b.id)));
        Ok(records)
    }

    fn record(&self, id: &str) -> Result<Record, LoreError> {
        let (kind, title, path, frontmatter) = self.node(id)?;
        let frontmatter: serde_json::Value = frontmatter
            .as_deref()
            .and_then(|text| serde_json::from_str(text).ok())
            .unwrap_or_default();
        let status = frontmatter
            .get("status")
            .and_then(|value| value.as_str())
            .map(str::to_string);

        let (features, related) = match kind.as_str() {
            "decision" => (
                self.targets_of(id, "decides")?,
                self.sources_of(id, "triggers")?,
            ),
            "incident" => (
                self.targets_of(id, "occurred_in")?,
                self.targets_of(id, "triggers")?,
            ),
            // 소스 노트는 근거가 되는 결정과 사고를 관련 기록으로 둔다.
            _ => (Vec::new(), self.targets_of(id, "supports")?),
        };

        let (sources, links) = if self.depth == Depth::Source && kind != "source" {
            let notes = Reader {
                conn: self.conn,
                depth: Depth::Doc,
            }
            .records(self.sources_of(id, "supports")?.iter())?;
            let links = frontmatter
                .get("sources")
                .and_then(|value| value.as_array())
                .into_iter()
                .flatten()
                .filter_map(|value| value.as_str())
                .filter(|value| is_link(value))
                .map(str::to_string)
                .collect();
            (Some(notes), Some(links))
        } else {
            (None, None)
        };

        Ok(Record {
            id: id.to_string(),
            date: id_date(id),
            title,
            path,
            status: if kind == "decision" { status } else { None },
            supersedes: self.targets_of(id, "supersedes")?.into_iter().next(),
            superseded_by: self.sources_of(id, "supersedes")?.into_iter().next(),
            kind,
            features,
            related,
            body: self.body(id)?,
            sources,
            links,
            mentioned: Vec::new(),
        })
    }
}

/// `text`에 `name`이 단어로 들어 있는지. 대소문자를 구분하고, 앞뒤 글자가 영문자, 숫자, `_`, `$`, `/`이면
/// 낱말의 일부로 본다. `/`를 넣은 것은 `src/x.ts`가 `packages/a/src/x.ts` 안에서 맞지 않게 하기 위해서다.
fn mentions_word(text: &str, name: &str) -> bool {
    if name.is_empty() {
        return false;
    }
    let part = |c: char| c.is_ascii_alphanumeric() || matches!(c, '_' | '$' | '/');
    text.match_indices(name).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let after = text[at + name.len()..].chars().next();
        !before.is_some_and(part) && !after.is_some_and(part)
    })
}

/// 사람용 조회 결과. 시나리오 문서의 형식을 따른다.
pub fn render(report: &WhyReport) -> String {
    let mut blocks = Vec::new();
    for feature in &report.features {
        blocks.push(render_feature(feature));
    }
    for record in &report.records {
        blocks.push(render_record(record));
    }
    if blocks.is_empty() {
        let what = if report.target.kind == "symbol" {
            "이 심볼을 담은 파일"
        } else {
            "이 파일"
        };
        blocks.push(format!(
            "{} 에 연결된 스펙이 없습니다. {what}을 맡을 스펙의 paths에 더하면 결정과 사고가 이어집니다.",
            report.target.id
        ));
    }
    if report.depth == Depth::Index
        && let Some(next) = next_step(report)
    {
        blocks.push(format!("자세히: lore why {next} --depth doc"));
    }
    blocks.join("\n\n")
}

fn render_feature(feature: &Feature) -> String {
    let mut text = format!("기능  {}  {}", feature.id, feature.title);
    if let Some(body) = &feature.body {
        text.push_str(&format!("\n\n{}", indent(body)));
    }
    if feature.decisions.is_empty() && feature.incidents.is_empty() {
        text.push_str("\n\n기록된 결정과 사고가 없습니다.");
    }
    for (label, records) in [("결정", &feature.decisions), ("사고", &feature.incidents)] {
        if records.is_empty() {
            continue;
        }
        text.push_str(&format!("\n\n{label}\n{}", lines(records)));
        for record in records {
            text.push_str(&render_detail(record));
        }
    }
    text
}

/// 기록 목록을 열을 맞춰 한 줄씩 쓴다. 상태가 있으면 제목 뒤에 붙인다.
fn lines(records: &[Record]) -> String {
    let id_width = records.iter().map(|r| r.id.len()).max().unwrap_or(0);
    let title_width = records
        .iter()
        .map(|r| display_width(&r.title))
        .max()
        .unwrap_or(0);
    let status_width = records
        .iter()
        .filter_map(Record::status_label)
        .map(display_width)
        .max()
        .unwrap_or(0);
    records
        .iter()
        .map(|record| {
            let date = record.date.as_deref().unwrap_or("          ");
            let mut line = format!("  {date}  {:id_width$}  {}", record.id, record.title);
            // 상태 칸을 묶음에서 가장 넓은 상태에 맞춰, 언급 표시가 줄마다 같은 자리에 오게 한다.
            let mut tail = String::new();
            if status_width > 0 {
                let status = record.status_label().unwrap_or("");
                tail.push_str(&format!(
                    "  {status}{}",
                    " ".repeat(status_width - display_width(status))
                ));
            }
            if !record.mentioned.is_empty() {
                tail.push_str("  언급됨(추론)");
            }
            if !tail.trim().is_empty() {
                let pad = title_width - display_width(&record.title);
                line.push_str(&format!("{}{}", " ".repeat(pad), tail.trim_end()));
            }
            line
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// `--depth doc` 이상에서 기록마다 붙이는 본문과 근거.
fn render_detail(record: &Record) -> String {
    let mut text = String::new();
    if let Some(body) = &record.body {
        text.push_str(&format!(
            "\n\n── {}  {}\n\n{}",
            record.id,
            record.title,
            indent(body)
        ));
    }
    text.push_str(&render_sources(record));
    text
}

/// `--depth source`일 때 기록 아래에 붙이는 소스 노트와 외부 링크.
fn render_sources(record: &Record) -> String {
    let mut text = String::new();
    for note in record.sources.iter().flatten() {
        text.push_str(&format!("\n\n  근거  {}  {}", note.id, note.title));
        if let Some(body) = &note.body {
            text.push_str(&format!("\n\n{}", indent(&indent(body))));
        }
    }
    for link in record.links.iter().flatten() {
        text.push_str(&format!("\n\n  근거  {link}"));
    }
    text
}

fn render_record(record: &Record) -> String {
    let kind = match record.kind.as_str() {
        "decision" => "결정",
        "incident" => "사고",
        _ => "소스 노트",
    };
    let mut text = format!("{kind}  {}  {}", record.id, record.title);
    if let Some(label) = record.status_label() {
        text.push_str(&format!("  {label}"));
    }
    let mut facts = Vec::new();
    if let Some(date) = &record.date {
        facts.push(("날짜", date.clone()));
    }
    if !record.features.is_empty() {
        facts.push(("기능", record.features.join(", ")));
    }
    if let Some(old) = &record.supersedes {
        facts.push(("대체함", old.clone()));
    }
    if let Some(new) = &record.superseded_by {
        facts.push(("대체됨", new.clone()));
    }
    if !record.related.is_empty() {
        let label = match record.kind.as_str() {
            "decision" => "계기",
            "incident" => "낳은 결정",
            _ => "근거가 됨",
        };
        facts.push((label, record.related.join(", ")));
    }
    facts.push(("문서", record.path.clone()));
    let width = facts
        .iter()
        .map(|(label, _)| display_width(label))
        .max()
        .unwrap_or(0);
    for (label, value) in facts {
        let pad = width - display_width(label);
        text.push_str(&format!("\n  {label}{}  {value}", " ".repeat(pad)));
    }
    if let Some(body) = &record.body {
        text.push_str(&format!("\n\n{}", indent(body)));
    }
    text.push_str(&render_sources(record));
    text
}

/// 다음에 열어 볼 만한 기록. 가장 최근의 유효한 결정, 없으면 가장 최근의 사고다.
fn next_step(report: &WhyReport) -> Option<&str> {
    if !report.records.is_empty() {
        return report.records.first().map(|record| record.id.as_str());
    }
    fn latest(records: Vec<&Record>) -> Option<&str> {
        records
            .into_iter()
            .max_by(|a, b| (&a.date, &a.id).cmp(&(&b.date, &b.id)))
            .map(|record| record.id.as_str())
    }
    let features = &report.features;
    // 파일이나 심볼로 물었으면 위로 올린(언급된 유효한) 기록이 먼저다.
    let lifted = features
        .iter()
        .flat_map(|f| f.decisions.iter().chain(&f.incidents))
        .find(|r| !r.mentioned.is_empty() && !r.is_superseded());
    if let Some(record) = lifted {
        return Some(record.id.as_str());
    }
    latest(
        features
            .iter()
            .flat_map(|f| &f.decisions)
            .filter(|r| !r.is_superseded())
            .collect(),
    )
    .or_else(|| latest(features.iter().flat_map(|f| &f.incidents).collect()))
}

fn indent(text: &str) -> String {
    text.trim_end()
        .lines()
        .map(|line| {
            if line.is_empty() {
                String::new()
            } else {
                format!("  {line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// 고정폭 글꼴에서 차지하는 칸 수. 한글과 한자 같은 넓은 글자는 두 칸이다.
pub(crate) fn display_width(text: &str) -> usize {
    text.chars()
        .map(|ch| match ch as u32 {
            0x1100..=0x115F
            | 0x2E80..=0x303E
            | 0x3041..=0x33FF
            | 0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xA960..=0xA97F
            | 0xAC00..=0xD7A3
            | 0xF900..=0xFAFF
            | 0xFE30..=0xFE4F
            | 0xFF00..=0xFF60
            | 0xFFE0..=0xFFE6 => 2,
            _ => 1,
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 한글은_두_칸으로_센다() {
        assert_eq!(display_width("abc"), 3);
        assert_eq!(display_width("결제 재시도"), 11);
    }

    #[test]
    fn 깊이_이름을_오간다() {
        for name in Depth::NAMES {
            let depth = Depth::from_name(name).unwrap();
            assert_eq!(serde_json::to_value(depth).unwrap(), name);
        }
        assert_eq!(Depth::from_name("all"), None);
    }

    #[test]
    fn 이름은_단어_경계로만_언급으로_본다() {
        assert!(mentions_word("`f`를 부릅니다", "f"));
        assert!(mentions_word("f(x)", "f"));
        assert!(mentions_word("끝에 f", "f"));
        assert!(!mentions_word("fx와 if", "f"));
        assert!(!mentions_word("$f, f_1", "f"));
        assert!(mentions_word("a/b.ts에서", "a/b.ts"));
        assert!(!mentions_word("x/a/b.ts", "a/b.ts"));
        assert!(!mentions_word("ID와 Id", "id"));
        assert!(!mentions_word("무엇이든", ""));
    }
}

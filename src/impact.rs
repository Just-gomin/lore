//! `lore impact`: 대상을 바꾸면 영향이 닿는 범위를 다섯 묶음으로 보여 준다.
//!
//! 홉은 기능 사이를 건너는 횟수다. 대상에서 출발해 의존함, 속함 엣지를 거꾸로 건너
//! "대상에 기대는 기능"을 찾고, 닿은 기능마다 파일, 테스트, 유효한 결정, 사고를 모은다.

use std::collections::BTreeMap;
use std::path::Path;

use rusqlite::Connection;
use serde::Serialize;

use crate::config::Config;
use crate::db::database_error;
use crate::error::LoreError;
use crate::graph::{EdgeKind, ORIGIN_FRONTMATTER};
use crate::query;

/// 기능 사이에서 건널 수 있는 엣지. `--edges`의 기본값이기도 하다.
pub const TRAVERSABLE: [EdgeKind; 2] = [EdgeKind::DependsOn, EdgeKind::BelongsTo];

/// 영향 범위. JSON 출력이 된다.
#[derive(Debug, Serialize)]
pub struct ImpactReport {
    pub target: Target,
    pub hop: u32,
    pub edges: Vec<EdgeKind>,
    /// 조회 전에 디비를 다시 만들었는지.
    pub rebuilt: bool,
    pub features: Vec<Item>,
    pub files: Vec<Item>,
    pub tests: Vec<Item>,
    pub decisions: Vec<Item>,
    pub incidents: Vec<Item>,
}

#[derive(Debug, Serialize)]
pub struct Target {
    pub input: String,
    pub id: String,
    pub kind: String,
}

/// 영향받는 노드 하나와 거기까지 온 길.
#[derive(Debug, Clone, Serialize)]
pub struct Item {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// 기능 사이를 건넌 횟수. 0은 대상이 직접 닿는 기능이다.
    pub hop: u32,
    /// 대상에서 이 노드까지 건넌 엣지. 엣지는 디비에 적힌 방향 그대로다.
    pub path: Vec<Step>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Step {
    pub src: String,
    pub dst: String,
    pub kind: EdgeKind,
    /// 엣지를 만든 근거. 2단계에서는 모두 `frontmatter`(선언)다.
    pub origin: String,
}

impl Item {
    /// 더 짧은 길이 앞선다. 길이가 같으면 경로를 글자 순으로 비교해 결과를 하나로 정한다.
    fn key(&self) -> (u32, usize, &Vec<Step>) {
        (self.hop, self.path.len(), &self.path)
    }
}

/// 같은 노드에 여러 길로 닿으면 가장 짧은 길만 남긴다.
fn keep_shortest(found: &mut BTreeMap<String, Item>, item: Item) {
    match found.get(&item.id) {
        Some(old) if old.key() <= item.key() => {}
        _ => {
            found.insert(item.id.clone(), item);
        }
    }
}

pub fn impact(
    config: &Config,
    cwd: &Path,
    input: &str,
    hop: u32,
    edges: &[EdgeKind],
) -> Result<ImpactReport, LoreError> {
    let (conn, rebuilt, id, kind) = query::open_target(config, cwd, input)?;
    let mut edges = edges.to_vec();
    edges.sort();
    edges.dedup();

    let mut features = BTreeMap::new();
    for seed in seeds(&conn, &id, &kind)? {
        for item in reach(&conn, &seed, hop, &edges)? {
            keep_shortest(&mut features, item);
        }
    }

    let attach = |edge: EdgeKind| -> Result<Vec<Item>, LoreError> {
        let mut found = BTreeMap::new();
        for feature in features.values() {
            for step in incoming(&conn, &feature.id, edge)? {
                let mut path = feature.path.clone();
                let src = step.src.clone();
                path.push(step);
                keep_shortest(
                    &mut found,
                    Item {
                        id: src,
                        title: None,
                        hop: feature.hop,
                        path,
                    },
                );
            }
        }
        Ok(sorted(found))
    };
    let files = attach(EdgeKind::Implements)?;
    let tests = attach(EdgeKind::Verifies)?;
    let mut decisions = attach(EdgeKind::Decides)?;
    let superseded = superseded(&conn)?;
    decisions.retain(|item| !superseded.contains(&item.id));
    let mut incidents = attach(EdgeKind::OccurredIn)?;

    let mut features = sorted(features);
    for item in features
        .iter_mut()
        .chain(&mut decisions)
        .chain(&mut incidents)
    {
        item.title = title(&conn, &item.id)?;
    }

    Ok(ImpactReport {
        target: Target {
            input: input.to_string(),
            id,
            kind,
        },
        hop,
        edges,
        rebuilt,
        features,
        files,
        tests,
        decisions,
        incidents,
    })
}

fn sorted(found: BTreeMap<String, Item>) -> Vec<Item> {
    let mut items: Vec<Item> = found.into_values().collect();
    items.sort_by(|a, b| (a.hop, &a.id).cmp(&(b.hop, &b.id)));
    items
}

fn step(row: &rusqlite::Row<'_>) -> rusqlite::Result<Step> {
    let kind: String = row.get(2)?;
    Ok(Step {
        src: row.get(0)?,
        dst: row.get(1)?,
        kind: EdgeKind::from_name(&kind).unwrap_or(EdgeKind::DependsOn),
        origin: row.get(3)?,
    })
}

/// `dst`로 들어오는 `kind` 엣지.
fn incoming(conn: &Connection, dst: &str, kind: EdgeKind) -> Result<Vec<Step>, LoreError> {
    let mut stmt = conn
        .prepare_cached(
            "SELECT src, dst, kind, origin FROM edges WHERE dst = ?1 AND kind = ?2 ORDER BY src",
        )
        .map_err(database_error)?;
    let rows = stmt
        .query_map([dst, kind.name()], step)
        .map_err(database_error)?;
    rows.collect::<Result<_, _>>().map_err(database_error)
}

/// `src`에서 나가는 엣지 중 `kinds`에 든 것.
fn outgoing(conn: &Connection, src: &str, kinds: &[EdgeKind]) -> Result<Vec<Step>, LoreError> {
    let mut steps = Vec::new();
    for kind in kinds {
        let mut stmt = conn
            .prepare_cached(
                "SELECT src, dst, kind, origin FROM edges WHERE src = ?1 AND kind = ?2 ORDER BY dst",
            )
            .map_err(database_error)?;
        let rows = stmt
            .query_map([src, kind.name()], step)
            .map_err(database_error)?;
        steps.extend(
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(database_error)?,
        );
    }
    Ok(steps)
}

/// 대상이 직접 닿는 기능. 홉 0이다.
fn seeds(conn: &Connection, id: &str, kind: &str) -> Result<Vec<Item>, LoreError> {
    let item = |path: Vec<Step>| {
        let id = path.last().map(|step| step.dst.clone()).unwrap_or_default();
        Item {
            id,
            title: None,
            hop: 0,
            path,
        }
    };
    let direct = |kinds: &[EdgeKind]| -> Result<Vec<Item>, LoreError> {
        Ok(outgoing(conn, id, kinds)?
            .into_iter()
            .map(|step| item(vec![step]))
            .collect())
    };
    match kind {
        "feature" => Ok(vec![Item {
            id: id.to_string(),
            title: None,
            hop: 0,
            path: Vec::new(),
        }]),
        "file" => direct(&[EdgeKind::Implements, EdgeKind::Verifies]),
        "decision" => direct(&[EdgeKind::Decides]),
        "incident" => direct(&[EdgeKind::OccurredIn]),
        // 소스 노트는 근거가 되는 결정과 사고를 거쳐 기능에 닿는다.
        _ => {
            let mut items = Vec::new();
            for support in outgoing(conn, id, &[EdgeKind::Supports])? {
                for next in outgoing(
                    conn,
                    &support.dst,
                    &[EdgeKind::Decides, EdgeKind::OccurredIn],
                )? {
                    items.push(item(vec![support.clone(), next]));
                }
            }
            Ok(items)
        }
    }
}

/// `seed`에서 엣지를 거꾸로 `hop`번까지 건너 닿는 기능. 재귀 쿼리로 찾는다.
///
/// 경로는 `src`, `dst`, `kind`, `origin`을 구분 문자로 이어 붙인 글자로 들고 다니다가 마지막에 푼다.
/// 이미 지난 기능은 다시 건너지 않아 순환이 있어도 끝난다.
fn reach(
    conn: &Connection,
    seed: &Item,
    hop: u32,
    edges: &[EdgeKind],
) -> Result<Vec<Item>, LoreError> {
    if edges.is_empty() {
        return Ok(vec![seed.clone()]);
    }
    let kinds = edges
        .iter()
        .map(|kind| format!("'{}'", kind.name()))
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "WITH RECURSIVE reach(id, hop, path, seen) AS (
             SELECT ?1, 0, '', char(29) || ?1 || char(29)
             UNION ALL
             SELECT e.src, r.hop + 1,
                    r.path || e.src || char(31) || e.dst || char(31) || e.kind || char(31) || e.origin || char(30),
                    r.seen || e.src || char(29)
             FROM reach r JOIN edges e ON e.dst = r.id
             WHERE r.hop < ?2 AND e.kind IN ({kinds})
               AND instr(r.seen, char(29) || e.src || char(29)) = 0
         )
         SELECT id, hop, path FROM reach ORDER BY hop, id, path"
    );
    let mut stmt = conn.prepare_cached(&sql).map_err(database_error)?;
    let rows = stmt
        .query_map(rusqlite::params![seed.id, hop], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, u32>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(database_error)?;

    let mut found = BTreeMap::new();
    for row in rows {
        let (id, hop, encoded) = row.map_err(database_error)?;
        let mut path = seed.path.clone();
        path.extend(decode(&encoded));
        keep_shortest(
            &mut found,
            Item {
                id,
                title: None,
                hop,
                path,
            },
        );
    }
    Ok(found.into_values().collect())
}

fn decode(encoded: &str) -> Vec<Step> {
    encoded
        .split('\u{1e}')
        .filter(|part| !part.is_empty())
        .filter_map(|part| {
            let fields: Vec<&str> = part.split('\u{1f}').collect();
            let [src, dst, kind, origin] = fields.as_slice() else {
                return None;
            };
            Some(Step {
                src: src.to_string(),
                dst: dst.to_string(),
                kind: EdgeKind::from_name(kind)?,
                origin: origin.to_string(),
            })
        })
        .collect()
}

/// 다른 결정이 대체한 결정의 ID.
fn superseded(conn: &Connection) -> Result<Vec<String>, LoreError> {
    let mut stmt = conn
        .prepare_cached("SELECT DISTINCT dst FROM edges WHERE kind = 'supersedes'")
        .map_err(database_error)?;
    let rows = stmt
        .query_map([], |row| row.get(0))
        .map_err(database_error)?;
    rows.collect::<Result<_, _>>().map_err(database_error)
}

fn title(conn: &Connection, id: &str) -> Result<Option<String>, LoreError> {
    conn.query_row("SELECT title FROM nodes WHERE id = ?1", [id], |row| {
        row.get(0)
    })
    .map_err(database_error)
}

/// 사람용 영향 범위. 시나리오 문서의 다섯 묶음을 따른다.
pub fn render(report: &ImpactReport) -> String {
    let mut text = format!(
        "대상  {}  ({})",
        report.target.id,
        kind_label(&report.target.kind)
    );

    text.push_str(&format!("\n\n영향받는 기능  {}개", report.features.len()));
    let width = id_width(&report.features);
    for item in &report.features {
        let route = if item.hop == 0 {
            "직접".to_string()
        } else {
            format!("{}홉  {}", item.hop, describe(&feature_steps(item)))
        };
        text.push_str(&format!("\n  {:width$}  {route}", item.id));
    }

    for (label, items) in [
        ("영향받는 파일", &report.files),
        ("돌려야 할 테스트", &report.tests),
        ("지켜야 할 결정", &report.decisions),
        ("관련 사고", &report.incidents),
    ] {
        text.push_str(&format!("\n\n{label}  {}개", items.len()));
        let width = id_width(items);
        for item in items {
            let feature = item
                .path
                .last()
                .map(|step| step.dst.as_str())
                .unwrap_or_default();
            let mut line = format!("\n  {:width$}  ", item.id);
            if let Some(title) = &item.title {
                line.push_str(&format!("{title}  "));
            }
            line.push_str(&format!("({feature})"));
            text.push_str(&line);
        }
    }
    text
}

/// 기능 사이를 건넌 엣지만. 대상에서 첫 기능까지 온 엣지는 빼고 보여 준다.
fn feature_steps(item: &Item) -> Vec<&Step> {
    item.path
        .iter()
        .filter(|step| TRAVERSABLE.contains(&step.kind))
        .collect()
}

/// `note 의존함 common (선언)`처럼 엣지를 사람이 읽는 말로 잇는다.
fn describe(steps: &[&Step]) -> String {
    steps
        .iter()
        .map(|step| {
            format!(
                "{} {} {} ({})",
                step.src,
                step.kind.label(),
                step.dst,
                origin_label(&step.origin)
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn origin_label(origin: &str) -> &str {
    if origin == ORIGIN_FRONTMATTER {
        "선언"
    } else {
        "추론"
    }
}

fn kind_label(kind: &str) -> &str {
    match kind {
        "feature" => "기능",
        "file" => "파일",
        "decision" => "결정",
        "incident" => "사고",
        _ => "소스 노트",
    }
}

fn id_width(items: &[Item]) -> usize {
    items.iter().map(|item| item.id.len()).max().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 이어_붙인_경로를_엣지로_푼다() {
        let encoded = "note\u{1f}common\u{1f}depends_on\u{1f}frontmatter\u{1e}";
        assert_eq!(
            decode(encoded),
            [Step {
                src: "note".to_string(),
                dst: "common".to_string(),
                kind: EdgeKind::DependsOn,
                origin: "frontmatter".to_string(),
            }]
        );
        assert_eq!(decode(""), []);
    }
}

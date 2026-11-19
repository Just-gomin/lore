//! `lore impact`: 대상을 바꾸면 영향이 닿는 범위를 다섯 묶음으로 보여 준다.
//!
//! 홉은 기능 사이를 건너는 횟수다. 대상에서 출발해 의존함, 속함 엣지를 거꾸로 건너
//! "대상에 기대는 기능"을 찾고, 닿은 기능마다 파일, 테스트, 유효한 결정, 사고를 모은다.
//!
//! 정적 분석이 만든 의존(추론)도 기능 하나를 건너는 길로 센다. 파일 X를 기능 B의 파일이
//! 스펙에 속하지 않은 파일을 몇 번 거치든 import하면, B는 X가 속한 기능에서 한 홉이다.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::Path;

use rusqlite::Connection;
use serde::Serialize;

use crate::config::Config;
use crate::db::database_error;
use crate::error::LoreError;
use crate::graph::{EdgeKind, ORIGIN_ANALYSIS, ORIGIN_FRONTMATTER};
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
    /// 길의 근거. 정적 분석의 의존을 한 번이라도 건넜으면 `inferred`, 아니면 `declared`다.
    pub evidence: &'static str,
    /// 대상에서 이 노드까지 건넌 엣지. 엣지는 디비에 적힌 방향 그대로다.
    pub path: Vec<Step>,
    /// 길의 앞에서 대상이 직접 닿는 기능까지 온 엣지 수. 사람용 출력이 건너뛴다.
    #[serde(skip)]
    lead: usize,
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
    fn new(id: String, hop: u32, path: Vec<Step>, lead: usize) -> Item {
        let inferred = path
            .iter()
            .any(|step| step.kind == EdgeKind::DependsOn && step.origin != ORIGIN_FRONTMATTER);
        Item {
            id,
            title: None,
            hop,
            evidence: if inferred { "inferred" } else { "declared" },
            path,
            lead,
        }
    }

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
    let (conn, rebuilt, id, kind) = query::open_target(config, cwd, input, true)?;
    let mut edges = edges.to_vec();
    edges.sort();
    edges.dedup();
    // 분석 엣지는 모두 의존함이라 `--edges`에서 의존함을 빼면 함께 빠진다.
    let analysis = edges.contains(&EdgeKind::DependsOn);

    let seeds = seeds(&conn, &id, &kind)?;
    let seed_ids: BTreeSet<String> = seeds.iter().map(|item| item.id.clone()).collect();
    let mut features: BTreeMap<(String, &'static str), Item> = BTreeMap::new();
    let mut files = BTreeMap::new();

    // 홉 0의 펼칠 거리. 파일과 심볼은 분석 엣지를 대상 자신에서 출발한다.
    let mut frontier = Vec::new();
    let from_target = matches!(kind.as_str(), "file" | "symbol");
    for seed in &seeds {
        let lead = seed.path.len();
        let seed = Item::new(seed.id.clone(), 0, seed.path.clone(), lead);
        keep_shortest_feature(&mut features, seed.clone());
        let sources = if from_target {
            Vec::new()
        } else {
            feature_files(&conn, &seed)?
        };
        frontier.push(Expansion {
            feature: Some(seed),
            sources,
        });
    }
    if from_target {
        frontier.push(Expansion {
            feature: None,
            sources: vec![(id.clone(), Vec::new())],
        });
    }

    for level in 0..hop {
        let mut reached: BTreeMap<(String, &'static str), Item> = BTreeMap::new();
        for expansion in &frontier {
            if let Some(feature) = &expansion.feature {
                for step in declared(&conn, &feature.id, &edges)? {
                    let mut path = feature.path.clone();
                    let src = step.src.clone();
                    path.push(step);
                    keep_shortest_feature(
                        &mut reached,
                        Item::new(src, level + 1, path, feature.lead),
                    );
                }
            }
            if analysis && !expansion.sources.is_empty() {
                let lead = expansion.feature.as_ref().map_or(0, |feature| feature.lead);
                let walked = walk(&conn, &expansion.sources, &seed_ids)?;
                for (feature, path) in walked.features {
                    keep_shortest_feature(&mut reached, Item::new(feature, level + 1, path, lead));
                }
                for (file, path, owned) in walked.files {
                    let hop = if owned { level + 1 } else { level };
                    keep_shortest(&mut files, Item::new(file, hop, path, lead));
                }
            }
        }
        frontier = Vec::new();
        for (key, item) in reached {
            // 대상이 직접 닿는 기능에는 다른 길로 돌아오지 않는다.
            if features.contains_key(&key) || seed_ids.contains(&key.0) {
                continue;
            }
            features.insert(key, item.clone());
            let sources = feature_files(&conn, &item)?;
            frontier.push(Expansion {
                feature: Some(item),
                sources,
            });
        }
    }

    let attach = |edge: EdgeKind, found: &mut BTreeMap<String, Item>| -> Result<(), LoreError> {
        for feature in features.values() {
            for step in incoming(&conn, &feature.id, edge)? {
                let mut path = feature.path.clone();
                let src = step.src.clone();
                path.push(step);
                keep_shortest(found, Item::new(src, feature.hop, path, feature.lead));
            }
        }
        Ok(())
    };
    // 영향받는 파일에는 분석 엣지로 대상에 닿은 파일도 들어 있다.
    attach(EdgeKind::Implements, &mut files)?;
    let files = sorted(files);
    let collect = |edge: EdgeKind| -> Result<Vec<Item>, LoreError> {
        let mut found = BTreeMap::new();
        attach(edge, &mut found)?;
        Ok(sorted(found))
    };
    let tests = collect(EdgeKind::Verifies)?;
    let mut decisions = collect(EdgeKind::Decides)?;
    let superseded = superseded(&conn)?;
    decisions.retain(|item| !superseded.contains(&item.id));
    let mut incidents = collect(EdgeKind::OccurredIn)?;

    let mut features: Vec<Item> = features.into_values().collect();
    features.sort_by(|a, b| (a.hop, &a.id, a.evidence).cmp(&(b.hop, &b.id, b.evidence)));
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
        let lead = path.len();
        Item::new(id, 0, path, lead)
    };
    let direct = |kinds: &[EdgeKind]| -> Result<Vec<Item>, LoreError> {
        Ok(outgoing(conn, id, kinds)?
            .into_iter()
            .map(|step| item(vec![step]))
            .collect())
    };
    match kind {
        "feature" => Ok(vec![Item::new(id.to_string(), 0, Vec::new(), 0)]),
        "file" => direct(&[EdgeKind::Implements, EdgeKind::Verifies]),
        // 심볼은 담은 파일을 거쳐 기능에 닿는다.
        "symbol" => {
            let mut items = Vec::new();
            for belongs in outgoing(conn, id, &[EdgeKind::BelongsTo])? {
                for next in outgoing(
                    conn,
                    &belongs.dst,
                    &[EdgeKind::Implements, EdgeKind::Verifies],
                )? {
                    items.push(item(vec![belongs.clone(), next]));
                }
            }
            Ok(items)
        }
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

/// 한 기능(또는 대상)에서 다음 홉으로 펼칠 거리.
struct Expansion {
    /// 선언 엣지를 따라갈 기능. 스펙에 속하지 않은 파일이 대상이면 없다.
    feature: Option<Item>,
    /// 분석 엣지를 거꾸로 따라갈 출발 노드와 거기까지 온 길.
    sources: Vec<(String, Vec<Step>)>,
}

/// 같은 기능이라도 근거가 다르면 따로 남긴다. 근거마다 가장 짧은 길 하나다.
fn keep_shortest_feature(found: &mut BTreeMap<(String, &'static str), Item>, item: Item) {
    let key = (item.id.clone(), item.evidence);
    match found.get(&key) {
        Some(old) if old.key() <= item.key() => {}
        _ => {
            found.insert(key, item);
        }
    }
}

/// `feature`로 선언 엣지를 거꾸로 건너 한 홉에 닿는 기능.
fn declared(conn: &Connection, feature: &str, edges: &[EdgeKind]) -> Result<Vec<Step>, LoreError> {
    let mut steps = Vec::new();
    for kind in edges {
        steps.extend(
            incoming(conn, feature, *kind)?
                .into_iter()
                .filter(|step| step.origin == ORIGIN_FRONTMATTER && step.src != feature),
        );
    }
    Ok(steps)
}

/// 기능의 파일들. 분석 엣지를 따라갈 출발점이 된다.
fn feature_files(conn: &Connection, feature: &Item) -> Result<Vec<(String, Vec<Step>)>, LoreError> {
    let mut sources = Vec::new();
    for kind in [EdgeKind::Implements, EdgeKind::Verifies] {
        for step in incoming(conn, &feature.id, kind)? {
            let mut path = feature.path.clone();
            let src = step.src.clone();
            path.push(step);
            sources.push((src, path));
        }
    }
    Ok(sources)
}

/// 분석 엣지로 닿은 것들.
struct Walked {
    /// 닿은 기능과 길. 길의 끝은 그 기능의 파일이 기능을 구현하거나 검증하는 엣지다.
    features: Vec<(String, Vec<Step>)>,
    /// 닿은 파일, 길, 스펙에 속했는지.
    files: Vec<(String, Vec<Step>, bool)>,
}

/// 출발 노드들에 의존하는 파일과 심볼을 분석 엣지로 거꾸로 따라간다.
///
/// 스펙에 속한 파일에 닿으면 그 기능을 내고 거기서 멈춘다. 스펙에 속하지 않은 파일과 심볼은 계속 건넌다.
/// 파일에서는 그 파일의 심볼에 의존하는 것도 따라간다. 심볼 의존은 re-export를 거쳐 정의에 닿아 있어서,
/// 폴더의 `index.ts`를 거쳐 가져간 파일도 찾는다.
/// `skip`의 기능(대상이 직접 닿는 기능)은 내지 않는다. 같은 노드는 한 번만 지나 순환이 있어도 끝난다.
fn walk(
    conn: &Connection,
    sources: &[(String, Vec<Step>)],
    skip: &BTreeSet<String>,
) -> Result<Walked, LoreError> {
    let mut walked = Walked {
        features: Vec::new(),
        files: Vec::new(),
    };
    let mut seen: BTreeSet<String> = sources.iter().map(|(id, _)| id.clone()).collect();
    let mut queue: VecDeque<(String, Vec<Step>)> = sources.iter().cloned().collect();
    while let Some((node, path)) = queue.pop_front() {
        for belongs in incoming(conn, &node, EdgeKind::BelongsTo)?
            .into_iter()
            .filter(|step| step.origin == ORIGIN_ANALYSIS)
        {
            if seen.insert(belongs.src.clone()) {
                let mut path = path.clone();
                let symbol = belongs.src.clone();
                path.push(belongs);
                queue.push_back((symbol, path));
            }
        }
        let steps = incoming(conn, &node, EdgeKind::DependsOn)?;
        for step in steps
            .into_iter()
            .filter(|step| step.origin == ORIGIN_ANALYSIS)
        {
            if !seen.insert(step.src.clone()) {
                continue;
            }
            let src = step.src.clone();
            let mut path = path.clone();
            path.push(step);
            // 심볼은 담은 파일로 기능을 찾는다.
            let mut to_file = path.clone();
            let file = match outgoing(conn, &src, &[EdgeKind::BelongsTo])?
                .into_iter()
                .find(|step| step.origin == ORIGIN_ANALYSIS)
            {
                Some(belongs) => {
                    let file = belongs.dst.clone();
                    to_file.push(belongs);
                    file
                }
                None => src.clone(),
            };
            let owners = outgoing(conn, &file, &[EdgeKind::Implements, EdgeKind::Verifies])?;
            walked
                .files
                .push((file, to_file.clone(), !owners.is_empty()));
            if owners.is_empty() {
                queue.push_back((src, path));
                continue;
            }
            for owner in owners {
                if skip.contains(&owner.dst) {
                    continue;
                }
                let mut path = to_file.clone();
                let feature = owner.dst.clone();
                path.push(owner);
                walked.features.push((feature, path));
            }
        }
    }
    Ok(walked)
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
            format!(
                "{}홉  {}  {}",
                item.hop,
                evidence_label(item.evidence),
                describe(&item.path[item.lead..])
            )
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
            let mut line = format!("\n  {:width$}  ", item.id);
            if let Some(title) = &item.title {
                line.push_str(&format!("{title}  "));
            }
            // 분석 엣지로 닿은 파일은 무엇에 의존하는지, 나머지는 어느 기능에 닿았는지 보인다.
            // 심볼로 닿았으면 그 파일의 심볼이 의존한 것을 보인다.
            let reached_by = item.path.iter().rev().find(|step| {
                step.kind == EdgeKind::DependsOn
                    && (step.src == item.id
                        || step
                            .src
                            .strip_prefix(item.id.as_str())
                            .is_some_and(|rest| rest.starts_with('#')))
            });
            match reached_by {
                Some(step) => {
                    line.push_str(&format!("({} {})", step.kind.label(), step.dst));
                }
                _ => {
                    let feature = item
                        .path
                        .last()
                        .map(|step| step.dst.as_str())
                        .unwrap_or_default();
                    line.push_str(&format!("({feature})"));
                }
            }
            text.push_str(&line);
        }
    }
    text
}

/// `note 의존함 common (선언)`처럼 엣지를 사람이 읽는 말로 잇는다.
fn describe(steps: &[Step]) -> String {
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

fn evidence_label(evidence: &str) -> &str {
    if evidence == "declared" {
        "선언"
    } else {
        "추론"
    }
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
        "symbol" => "심볼",
        _ => "소스 노트",
    }
}

fn id_width(items: &[Item]) -> usize {
    items.iter().map(|item| item.id.len()).max().unwrap_or(0)
}

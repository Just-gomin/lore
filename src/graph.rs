//! 소스와 파일 목록으로 노드와 엣지를 만든다. 디비에 쓰기 전의 순수한 계산이다.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use globset::{GlobBuilder, GlobMatcher};
use serde::Serialize;

use crate::error::Problem;
use crate::source::{Corpus, Document, Record, is_link, sha256_hex};

/// 엣지의 종류. 이름은 디비와 JSON에 그대로 쓴다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    /// 결정 → 기능
    Decides,
    /// 결정 → 결정
    Supersedes,
    /// 사고 → 결정
    Triggers,
    /// 사고 → 기능
    OccurredIn,
    /// 파일 → 기능
    Implements,
    /// 파일 → 기능
    Verifies,
    /// 기능 → 기능
    DependsOn,
    /// 소스 노트 → 결정, 사고
    Supports,
    /// 기능 → 상위 기능
    BelongsTo,
}

impl EdgeKind {
    pub const ALL: [EdgeKind; 9] = [
        EdgeKind::Decides,
        EdgeKind::Supersedes,
        EdgeKind::Triggers,
        EdgeKind::OccurredIn,
        EdgeKind::Implements,
        EdgeKind::Verifies,
        EdgeKind::DependsOn,
        EdgeKind::Supports,
        EdgeKind::BelongsTo,
    ];

    pub fn name(self) -> &'static str {
        match self {
            EdgeKind::Decides => "decides",
            EdgeKind::Supersedes => "supersedes",
            EdgeKind::Triggers => "triggers",
            EdgeKind::OccurredIn => "occurred_in",
            EdgeKind::Implements => "implements",
            EdgeKind::Verifies => "verifies",
            EdgeKind::DependsOn => "depends_on",
            EdgeKind::Supports => "supports",
            EdgeKind::BelongsTo => "belongs_to",
        }
    }

    /// 사람용 출력에 쓰는 이름. 기획서 3장의 이름과 같다.
    pub fn label(self) -> &'static str {
        match self {
            EdgeKind::Decides => "결정함",
            EdgeKind::Supersedes => "대체함",
            EdgeKind::Triggers => "유발함",
            EdgeKind::OccurredIn => "발생함",
            EdgeKind::Implements => "구현함",
            EdgeKind::Verifies => "검증함",
            EdgeKind::DependsOn => "의존함",
            EdgeKind::Supports => "근거가 됨",
            EdgeKind::BelongsTo => "속함",
        }
    }

    pub fn from_name(name: &str) -> Option<EdgeKind> {
        EdgeKind::ALL.into_iter().find(|kind| kind.name() == name)
    }
}

/// 엣지를 만든 근거. 2단계에서는 모두 머리말이다.
pub const ORIGIN_FRONTMATTER: &str = "frontmatter";

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Node {
    pub id: String,
    /// `feature`, `decision`, `incident`, `source`, `file`
    pub kind: &'static str,
    pub title: Option<String>,
    pub summary: Option<String>,
    pub status: Option<String>,
    pub source_path: String,
    pub content_hash: String,
    /// 머리말 전체를 담은 JSON. 파일 노드는 없다.
    pub frontmatter: Option<String>,
    /// 전문 검색에 넣을 본문. 파일 노드는 없다.
    pub body: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Edge {
    pub src: String,
    pub dst: String,
    pub kind: EdgeKind,
    pub origin: &'static str,
}

/// 노드와 엣지. 둘 다 정렬되어 있어 같은 입력에서는 같은 순서가 나온다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Graph {
    pub nodes: Vec<Node>,
    pub edges: Vec<Edge>,
}

impl Graph {
    pub fn count_nodes(&self) -> BTreeMap<&'static str, usize> {
        let mut counts = BTreeMap::new();
        for node in &self.nodes {
            *counts.entry(node.kind).or_default() += 1;
        }
        counts
    }

    pub fn count_edges(&self) -> BTreeMap<&'static str, usize> {
        let mut counts = BTreeMap::new();
        for edge in &self.edges {
            *counts.entry(edge.kind.name()).or_default() += 1;
        }
        counts
    }
}

/// 소스와 git 추적 파일로 그래프를 만든다.
///
/// 스펙의 `paths`, `tests` 글롭이 추적 파일 하나에도 맞지 않으면 문제로 돌려준다.
pub fn build(corpus: &Corpus, root: &Path, files: &[String]) -> Result<Graph, Vec<Problem>> {
    let mut problems = Vec::new();
    let mut nodes = Vec::new();
    let mut edges = BTreeSet::new();
    let mut touched_files = BTreeSet::new();

    let mut edge = |src: &str, dst: &str, kind: EdgeKind| {
        edges.insert(Edge {
            src: src.to_string(),
            dst: dst.to_string(),
            kind,
            origin: ORIGIN_FRONTMATTER,
        });
    };

    for doc in &corpus.documents {
        nodes.push(document_node(doc));
        match &doc.record {
            Record::Spec(spec) => {
                let implemented = match_globs(doc, "paths", &spec.paths, files, &mut problems);
                let verified = match_globs(doc, "tests", &spec.tests, files, &mut problems);
                for file in &verified {
                    edge(file, &doc.id, EdgeKind::Verifies);
                }
                // 테스트로 맞은 파일은 같은 스펙을 구현하는 파일로 보지 않는다.
                for file in implemented.difference(&verified) {
                    edge(file, &doc.id, EdgeKind::Implements);
                }
                touched_files.extend(implemented);
                touched_files.extend(verified);
                for target in &spec.depends_on {
                    edge(&doc.id, target, EdgeKind::DependsOn);
                }
                if let Some(parent) = &spec.parent {
                    edge(&doc.id, parent, EdgeKind::BelongsTo);
                }
            }
            Record::Decision(decision) => {
                for spec in &decision.specs {
                    edge(&doc.id, spec, EdgeKind::Decides);
                }
                if let Some(old) = &decision.supersedes {
                    edge(&doc.id, old, EdgeKind::Supersedes);
                }
                for source in decision.sources.iter().filter(|s| !is_link(s)) {
                    edge(source, &doc.id, EdgeKind::Supports);
                }
            }
            Record::Incident(incident) => {
                for spec in &incident.specs {
                    edge(&doc.id, spec, EdgeKind::OccurredIn);
                }
                for decision in &incident.decisions {
                    edge(&doc.id, decision, EdgeKind::Triggers);
                }
                for source in incident.sources.iter().filter(|s| !is_link(s)) {
                    edge(source, &doc.id, EdgeKind::Supports);
                }
            }
            Record::Source(_) => {}
        }
    }

    for file in &touched_files {
        let hash = std::fs::read(root.join(file))
            .map(|bytes| sha256_hex(&bytes))
            .unwrap_or_default();
        nodes.push(Node {
            id: file.clone(),
            kind: "file",
            title: None,
            summary: None,
            status: None,
            source_path: file.clone(),
            content_hash: hash,
            frontmatter: None,
            body: None,
        });
    }

    if !problems.is_empty() {
        problems.sort();
        return Err(problems);
    }
    nodes.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(Graph {
        nodes,
        edges: edges.into_iter().collect(),
    })
}

fn document_node(doc: &Document) -> Node {
    let status = match &doc.record {
        Record::Decision(decision) => Some(decision.status.clone()),
        _ => None,
    };
    Node {
        id: doc.id.clone(),
        kind: doc.kind.node_kind(),
        title: Some(doc.title.clone()),
        summary: summary(&doc.body),
        status,
        source_path: doc.path.clone(),
        content_hash: doc.hash.clone(),
        frontmatter: Some(doc.frontmatter.to_string()),
        body: Some(doc.body.clone()),
    }
}

/// 본문에서 제목이 아닌 첫 문단을 한 줄로 만든다.
pub fn summary(body: &str) -> Option<String> {
    let mut lines = Vec::new();
    for line in body.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            if !lines.is_empty() {
                break;
            }
            continue;
        }
        if line.starts_with("```") {
            break;
        }
        lines.push(line.trim_start_matches("- ").trim());
    }
    let text = lines.join(" ");
    (!text.is_empty()).then_some(text)
}

/// 스펙의 `paths`, `tests`와 설정의 글롭을 읽는다. `*`는 폴더 경계를 넘지 않는다.
pub fn matcher(pattern: &str) -> Result<GlobMatcher, globset::Error> {
    let pattern = pattern.trim_start_matches("./");
    Ok(GlobBuilder::new(pattern)
        .literal_separator(true)
        .build()?
        .compile_matcher())
}

fn match_globs(
    doc: &Document,
    field: &str,
    patterns: &[String],
    files: &[String],
    problems: &mut Vec<Problem>,
) -> BTreeSet<String> {
    let mut matched = BTreeSet::new();
    for pattern in patterns {
        let glob = match matcher(pattern) {
            Ok(glob) => glob,
            Err(error) => {
                problems.push(Problem::new(
                    &doc.path,
                    Some(field),
                    "invalid_glob",
                    format!("'{pattern}'는 글롭으로 읽을 수 없습니다: {error}"),
                ));
                continue;
            }
        };
        let hits: Vec<&String> = files.iter().filter(|file| glob.is_match(file)).collect();
        if hits.is_empty() {
            problems.push(Problem::new(
                &doc.path,
                Some(field),
                "no_matching_files",
                format!("'{pattern}'에 맞는 파일이 git이 추적하는 파일 중에 없습니다."),
            ));
        }
        matched.extend(hits.into_iter().cloned());
    }
    matched
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 별표_하나는_폴더_경계를_넘지_않는다() {
        let glob = matcher("services/*.ts").unwrap();
        assert!(glob.is_match("services/a.ts"));
        assert!(!glob.is_match("services/payment/a.ts"));
        let glob = matcher("services/**").unwrap();
        assert!(glob.is_match("services/payment/retry/queue.ts"));
    }

    #[test]
    fn 요약은_제목이_아닌_첫_문단이다() {
        let body = "## 지금 동작\n\n- 첫 줄\n이어지는 줄\n\n둘째 문단\n";
        assert_eq!(summary(body).as_deref(), Some("첫 줄 이어지는 줄"));
        assert_eq!(summary("## 배경\n\n## 결정\n"), None);
    }

    #[test]
    fn 엣지_이름을_오간다() {
        for kind in EdgeKind::ALL {
            assert_eq!(EdgeKind::from_name(kind.name()), Some(kind));
        }
    }
}

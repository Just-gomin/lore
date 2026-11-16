//! `docs/INDEX.md`: 모든 기록의 한 줄 목록을 만든다.
//!
//! 스펙마다 묶고, 묶음 안에서는 스펙 자신을 먼저 두고 나머지를 ID 순으로 둔다.
//! 같은 소스에서는 같은 글자가 나온다. 날짜나 커밋처럼 소스 밖의 값은 넣지 않는다.

use std::collections::{BTreeMap, BTreeSet};

use crate::source::{Corpus, Document, Kind, Record};

/// 소스 폴더 안의 인덱스 파일 이름.
pub const FILE_NAME: &str = "INDEX.md";

/// 스펙에 연결되지 않은 기록을 모은 묶음의 제목.
const UNLINKED: &str = "스펙에 연결되지 않은 기록";

/// 인덱스 본문을 만든다. `docs`는 소스 폴더이며 링크는 그 폴더 기준 경로다.
pub fn render(corpus: &Corpus, docs: &str) -> String {
    let groups = group_by_spec(corpus);
    let superseded_by = superseded_by(corpus);
    let prefix = format!("{docs}/");

    let mut text = String::from(
        "# INDEX\n\n\
         모든 기록의 한 줄 목록입니다. `lore build`가 만들므로 손으로 고치지 않습니다. \
         ID 순으로 정렬하고 스펙별로 묶습니다.\n",
    );
    let line = |doc: &Document| {
        let link = doc.path.strip_prefix(&prefix).unwrap_or(&doc.path);
        let mut line = format!(
            "- `{}` · {} · [{}]({link})",
            doc.id,
            doc.title,
            label(doc.kind)
        );
        if let Some(new) = superseded_by.get(doc.id.as_str()) {
            line.push_str(&format!(" · [대체됨: {new}]"));
        }
        line
    };

    for (spec, members) in &groups.by_spec {
        text.push_str(&format!("\n## {spec}\n\n"));
        if let Some(doc) = corpus.get(spec) {
            text.push_str(&line(doc));
            text.push('\n');
        }
        for id in members {
            if let Some(doc) = corpus.get(id) {
                text.push_str(&line(doc));
                text.push('\n');
            }
        }
    }
    if !groups.unlinked.is_empty() {
        text.push_str(&format!("\n## {UNLINKED}\n\n"));
        for id in &groups.unlinked {
            if let Some(doc) = corpus.get(id) {
                text.push_str(&line(doc));
                text.push('\n');
            }
        }
    }
    text
}

fn label(kind: Kind) -> &'static str {
    match kind {
        Kind::Spec => "스펙",
        Kind::Decision => "결정",
        Kind::Incident => "사고",
        Kind::Source => "소스 노트",
    }
}

struct Groups<'a> {
    /// 스펙 ID → 그 스펙에 속한 기록 ID. 스펙 자신은 넣지 않는다.
    by_spec: BTreeMap<&'a str, BTreeSet<&'a str>>,
    /// 어느 스펙에도 닿지 않는 결정, 사고, 소스 노트.
    unlinked: BTreeSet<&'a str>,
}

/// 결정과 사고는 머리말의 `specs`로, 소스 노트는 자신이 근거가 되는 결정과 사고의 스펙으로 묶는다.
fn group_by_spec(corpus: &Corpus) -> Groups<'_> {
    let mut by_spec: BTreeMap<&str, BTreeSet<&str>> = corpus
        .of_kind(Kind::Spec)
        .map(|doc| (doc.id.as_str(), BTreeSet::new()))
        .collect();
    let mut source_specs: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();

    for doc in &corpus.documents {
        let (specs, sources) = match &doc.record {
            Record::Decision(decision) => (&decision.specs, &decision.sources),
            Record::Incident(incident) => (&incident.specs, &incident.sources),
            Record::Spec(_) | Record::Source(_) => continue,
        };
        for spec in specs {
            by_spec.entry(spec).or_default().insert(&doc.id);
        }
        for source in sources {
            source_specs
                .entry(source)
                .or_default()
                .extend(specs.iter().map(String::as_str));
        }
    }
    for doc in corpus.of_kind(Kind::Source) {
        for spec in source_specs.get(doc.id.as_str()).into_iter().flatten() {
            by_spec.entry(spec).or_default().insert(&doc.id);
        }
    }

    let linked: BTreeSet<&str> = by_spec.values().flatten().copied().collect();
    let unlinked = corpus
        .documents
        .iter()
        .filter(|doc| doc.kind != Kind::Spec && !linked.contains(doc.id.as_str()))
        .map(|doc| doc.id.as_str())
        .collect();
    Groups { by_spec, unlinked }
}

/// 대체된 결정 ID → 그 결정을 대체한 결정 ID.
fn superseded_by(corpus: &Corpus) -> BTreeMap<&str, &str> {
    corpus
        .of_kind(Kind::Decision)
        .filter_map(|doc| {
            let old = doc.decision()?.supersedes.as_deref()?;
            Some((old, doc.id.as_str()))
        })
        .collect()
}

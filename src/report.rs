//! `lore check --report`: 병합을 막지 않는 보고. 근거 없는 결정의 비율과 나눌 스펙, 합칠 스펙의 후보.
//!
//! 스펙 크기의 신호는 기획서 2장의 경험칙에서 시작한다. 쓰는 값은 출력에 함께 낸다.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

use crate::check::SpecGlobs;
use crate::config::Config;
use crate::error::LoreError;
use crate::{git, source};

/// 커밋의 이 비율 이상이 건드리는 스펙은 나눌 후보다.
const SPLIT_PERCENT: usize = 50;
/// 둘 중 하나가 바뀐 커밋의 이 비율 이상에서 함께 바뀐 두 스펙은 합칠 후보다.
const MERGE_PERCENT: usize = 80;
/// 한 커밋에서 이 개수 이상 함께 바뀐 스펙 묶음은 합칠 후보다.
const GROUP_SIZE: usize = 4;

#[derive(Debug, Serialize)]
pub struct Report {
    pub unsourced_decisions: Unsourced,
    pub spec_size: SpecSize,
}

/// `sources`가 비어 있는 결정 기록.
#[derive(Debug, Serialize)]
pub struct Unsourced {
    pub count: usize,
    pub total: usize,
    /// 결정 기록이 없으면 없다.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ratio: Option<f64>,
    pub decisions: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct SpecSize {
    /// 이력에서 스펙을 하나 이상 건드린 커밋의 수. 비율의 분모다.
    pub commits: usize,
    pub thresholds: Thresholds,
    pub split: Vec<Split>,
    pub merge: Vec<Merge>,
    pub groups: Vec<Group>,
}

#[derive(Debug, Serialize)]
pub struct Thresholds {
    pub split_percent: usize,
    pub merge_percent: usize,
    pub group_size: usize,
}

/// 나눌 후보. 너무 많은 커밋이 건드리는 스펙.
#[derive(Debug, Serialize)]
pub struct Split {
    pub spec: String,
    pub commits: usize,
    pub ratio: f64,
}

/// 합칠 후보. 거의 늘 함께 바뀌는 두 스펙.
#[derive(Debug, Serialize)]
pub struct Merge {
    pub specs: [String; 2],
    /// 둘이 함께 바뀐 커밋의 수.
    pub together: usize,
    /// 둘 중 하나라도 바뀐 커밋의 수.
    pub either: usize,
    pub ratio: f64,
}

/// 합칠 후보. 한 커밋에서 함께 바뀐 스펙 묶음.
#[derive(Debug, Serialize)]
pub struct Group {
    pub specs: Vec<String>,
    pub commits: usize,
}

pub fn report(config: &Config, corpus: &source::Corpus) -> Result<Report, LoreError> {
    let specs = SpecGlobs::all(corpus);
    let touched: Vec<BTreeSet<&str>> = git::history(&config.root)?
        .iter()
        .map(|files| {
            specs
                .iter()
                .filter(|spec| files.iter().any(|file| spec.matches(file)))
                .map(|spec| spec.doc.id.as_str())
                .collect::<BTreeSet<_>>()
        })
        .filter(|ids| !ids.is_empty())
        .collect();
    Ok(Report {
        unsourced_decisions: unsourced(corpus),
        spec_size: spec_size(&touched),
    })
}

fn unsourced(corpus: &source::Corpus) -> Unsourced {
    let mut total = 0;
    let mut decisions = Vec::new();
    for doc in corpus.of_kind(source::Kind::Decision) {
        let source::Record::Decision(decision) = &doc.record else {
            continue;
        };
        total += 1;
        if decision.sources.is_empty() {
            decisions.push(doc.id.clone());
        }
    }
    decisions.sort();
    Unsourced {
        count: decisions.len(),
        total,
        ratio: (total > 0).then(|| ratio(decisions.len(), total)),
        decisions,
    }
}

/// 커밋마다 건드린 스펙으로 크기 신호를 센다.
fn spec_size(touched: &[BTreeSet<&str>]) -> SpecSize {
    let commits = touched.len();
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    let mut pairs: BTreeMap<(&str, &str), usize> = BTreeMap::new();
    let mut groups: BTreeMap<Vec<&str>, usize> = BTreeMap::new();
    for ids in touched {
        for id in ids {
            *counts.entry(id).or_default() += 1;
        }
        let ids: Vec<&str> = ids.iter().copied().collect();
        for (i, a) in ids.iter().enumerate() {
            for b in &ids[i + 1..] {
                *pairs.entry((a, b)).or_default() += 1;
            }
        }
        if ids.len() >= GROUP_SIZE {
            *groups.entry(ids).or_default() += 1;
        }
    }

    let split = counts
        .iter()
        .filter(|(_, count)| **count * 100 >= commits * SPLIT_PERCENT)
        .map(|(spec, count)| Split {
            spec: spec.to_string(),
            commits: *count,
            ratio: ratio(*count, commits),
        })
        .collect();
    let merge = pairs
        .iter()
        .filter_map(|((a, b), together)| {
            let either = counts[a] + counts[b] - together;
            (*together * 100 >= either * MERGE_PERCENT).then(|| Merge {
                specs: [a.to_string(), b.to_string()],
                together: *together,
                either,
                ratio: ratio(*together, either),
            })
        })
        .collect();
    let groups = groups
        .into_iter()
        .map(|(ids, commits)| Group {
            specs: ids.into_iter().map(str::to_string).collect(),
            commits,
        })
        .collect();
    SpecSize {
        commits,
        thresholds: Thresholds {
            split_percent: SPLIT_PERCENT,
            merge_percent: MERGE_PERCENT,
            group_size: GROUP_SIZE,
        },
        split,
        merge,
        groups,
    }
}

/// 소수 둘째 자리까지의 비율.
fn ratio(part: usize, whole: usize) -> f64 {
    (part as f64 / whole as f64 * 100.0).round() / 100.0
}

fn percent(ratio: f64) -> String {
    format!("{}%", (ratio * 100.0).round())
}

/// 사람용 보고. 검사 결과 뒤에 붙인다.
pub fn render(report: &Report) -> String {
    let unsourced = &report.unsourced_decisions;
    let mut text = format!(
        "보고\n  근거 없는 결정: {}/{}",
        unsourced.count, unsourced.total
    );
    if let Some(ratio) = unsourced.ratio {
        text.push_str(&format!(" ({})", percent(ratio)));
    }
    let size = &report.spec_size;
    let none = |empty: bool| if empty { " 없음" } else { "" };
    text.push_str(&format!(
        "\n  스펙을 건드린 커밋 {}건 기준\n  나눌 스펙 후보 (커밋의 {}% 이상):{}",
        size.commits,
        size.thresholds.split_percent,
        none(size.split.is_empty())
    ));
    for split in &size.split {
        text.push_str(&format!(
            "\n    {} {}/{} ({})",
            split.spec,
            split.commits,
            size.commits,
            percent(split.ratio)
        ));
    }
    text.push_str(&format!(
        "\n  합칠 스펙 후보 (둘 중 하나가 바뀐 커밋의 {}% 이상에서 함께 바뀜):{}",
        size.thresholds.merge_percent,
        none(size.merge.is_empty())
    ));
    for merge in &size.merge {
        text.push_str(&format!(
            "\n    {}, {} {}/{} ({})",
            merge.specs[0],
            merge.specs[1],
            merge.together,
            merge.either,
            percent(merge.ratio)
        ));
    }
    text.push_str(&format!(
        "\n  합칠 스펙 후보 (한 커밋에서 {}개 이상 함께 바뀜):{}",
        size.thresholds.group_size,
        none(size.groups.is_empty())
    ));
    for group in &size.groups {
        text.push_str(&format!(
            "\n    {} {}건",
            group.specs.join(", "),
            group.commits
        ));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 기준_비율에_닿으면_후보로_낸다() {
        let touched: Vec<BTreeSet<&str>> = vec![
            ["a", "b"].into(),
            ["a", "b"].into(),
            ["a", "b", "c", "d"].into(),
            ["a", "c"].into(),
            ["e"].into(),
        ];
        let size = spec_size(&touched);
        assert_eq!(size.commits, 5);
        // a는 5건 중 4건, b는 3건, c는 2건이다.
        let split: Vec<&str> = size.split.iter().map(|s| s.spec.as_str()).collect();
        assert_eq!(split, ["a", "b"]);
        assert_eq!(size.split[0].ratio, 0.8);
        // a, b는 4건 중 3건(75%)이라 아니고, c, d는 2건 중 1건, b, d는 3건 중 1건이다.
        assert!(size.merge.is_empty());
        assert_eq!(size.groups.len(), 1);
        assert_eq!(size.groups[0].specs, ["a", "b", "c", "d"]);

        let size = spec_size(&[["a", "b"].into(), ["a", "b"].into(), ["a"].into()]);
        // a, b는 3건 중 2건(67%)이라 아니다.
        assert!(size.merge.is_empty());
        let mut touched = vec![BTreeSet::from(["a", "b"]); 4];
        touched.push(BTreeSet::from(["a"]));
        let size = spec_size(&touched);
        assert_eq!(size.merge.len(), 1);
        assert_eq!(size.merge[0].together, 4);
        assert_eq!(size.merge[0].either, 5);
    }
}

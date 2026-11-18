//! `lore check`: 소스와 코드가 어긋났는지 검사한다. 파일은 고치지 않는다.
//!
//! 기계적 검사는 판단이 없어 실패하면 병합을 막는다. 판단이 드는 검사(스펙 동반 갱신, 매핑 누락)는
//! 경고만 내고, 경고마다 무엇이 걸렸고 어떻게 처리됐는지를 기록으로 남긴다.

use std::collections::{BTreeMap, BTreeSet};

use globset::GlobMatcher;
use serde::{Deserialize, Serialize};

use crate::config::{CONFIG_FILE, Config};
use crate::error::{LoreError, Problem};
use crate::git::{Change, ChangeKind};
use crate::install::{self, AGENTS_BODY, AGENTS_FILE, TOOL_FILES, TOOL_POINTER};
use crate::{git, graph, index, report, source};

/// 검사 하나의 결과.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Pass,
    Fail,
    Warn,
    Skip,
}

impl Status {
    fn label(self) -> &'static str {
        match self {
            Status::Pass => "통과",
            Status::Fail => "실패",
            Status::Warn => "경고",
            Status::Skip => "건너뜀",
        }
    }

    const ALL: [Status; 4] = [Status::Fail, Status::Warn, Status::Skip, Status::Pass];
}

#[derive(Debug, Serialize)]
pub struct Check {
    /// 검사 이름. `paths`, `references`, `frontmatter`, `freshness`, `index`, `guide`,
    /// `spec_update`, `unmapped`
    pub id: &'static str,
    /// 사람용 이름. 기획서 5장의 검사 이름과 같다.
    pub label: &'static str,
    pub status: Status,
    /// 실패하면 병합을 막는 검사인지.
    pub blocking: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub problems: Vec<Problem>,
    /// 실패했을 때 고치는 방법.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fix: Option<&'static str>,
    /// 건너뛴 이유.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<&'static str>,
    /// 확인하지 못하고 넘어간 항목의 수. 신선도 검사에서 목록에 없는 원본이다.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unchecked: Option<usize>,
    /// 판단이 드는 검사의 기록. 경고로 운영하는 동안 모아 차단 전환 여부를 정한다.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub findings: Vec<Finding>,
}

/// 판단이 드는 검사에 걸린 것 하나.
#[derive(Debug, Serialize)]
pub struct Finding {
    /// 걸린 스펙의 ID. 매핑 누락에는 없다.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spec: Option<String>,
    /// 걸린 파일. 스펙 동반 갱신에서는 그 스펙에 닿는 바뀐 파일이다.
    pub files: Vec<String>,
    pub outcome: Outcome,
    /// `lore: no-impact` 줄에 적은 사유.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// 걸린 것을 어떻게 처리했는지.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// 처리하지 않아 경고를 냈다.
    Warned,
    /// 스펙도 같은 비교 범위에서 바뀌었다.
    Updated,
    /// PR 설명의 `lore: no-impact` 줄로 넘겼다.
    NoImpact,
}

impl Check {
    /// 문제가 있으면 실패, 없으면 통과인 차단 검사.
    fn blocking(
        id: &'static str,
        label: &'static str,
        problems: Vec<Problem>,
        fix: &'static str,
    ) -> Check {
        let failed = !problems.is_empty();
        Check {
            id,
            label,
            status: if failed { Status::Fail } else { Status::Pass },
            blocking: true,
            problems,
            fix: failed.then_some(fix),
            reason: None,
            unchecked: None,
            findings: Vec::new(),
        }
    }

    /// 처리하지 않은 것이 있으면 경고, 없으면 통과인 판단이 드는 검사.
    fn judged(
        id: &'static str,
        label: &'static str,
        findings: Vec<Finding>,
        fix: &'static str,
    ) -> Check {
        let warned = findings.iter().any(|f| f.outcome == Outcome::Warned);
        Check {
            id,
            label,
            status: if warned { Status::Warn } else { Status::Pass },
            blocking: false,
            problems: Vec::new(),
            fix: warned.then_some(fix),
            reason: None,
            unchecked: None,
            findings,
        }
    }

    fn skipped(
        id: &'static str,
        label: &'static str,
        blocking: bool,
        reason: &'static str,
    ) -> Check {
        Check {
            id,
            label,
            status: Status::Skip,
            blocking,
            problems: Vec::new(),
            fix: None,
            reason: Some(reason),
            unchecked: None,
            findings: Vec::new(),
        }
    }
}

/// 검사 결과. JSON 출력이 된다.
#[derive(Debug, Serialize)]
pub struct CheckReport {
    pub checks: Vec<Check>,
    /// 병합을 막지 않는 보고. `--report`일 때만 있다.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report: Option<report::Report>,
}

/// `lore check`의 옵션.
#[derive(Debug, Default)]
pub struct Options<'a> {
    /// 원본의 최신 판 목록. 없으면 신선도 검사를 건너뛴다.
    pub origins: Option<&'a Origins>,
    /// 비교 기준이 될 ref. 없으면 바뀐 파일이 필요한 검사를 건너뛴다.
    pub base: Option<&'a str>,
    /// PR 설명. `lore: no-impact` 줄을 읽는다.
    pub pr_body: Option<&'a str>,
    /// 병합을 막지 않는 보고를 낼지.
    pub report: bool,
}

impl CheckReport {
    /// 차단 검사가 하나도 실패하지 않았는지. 경고와 건너뜀은 영향을 주지 않는다.
    pub fn passed(&self) -> bool {
        !self
            .checks
            .iter()
            .any(|check| check.blocking && check.status == Status::Fail)
    }
}

/// 문제 코드가 속한 검사. 여기에 없는 코드는 머리말 형식으로 본다.
fn check_of(code: &str) -> &'static str {
    match code {
        "no_matching_files" | "invalid_glob" => "paths",
        "unknown_reference" | "invalid_reference" => "references",
        _ => "frontmatter",
    }
}

pub fn check(config: &Config, options: &Options) -> Result<CheckReport, LoreError> {
    let changes = options
        .base
        .map(|base| git::changed_files(&config.root, base))
        .transpose()?;
    let (corpus, mut problems) = source::read_lenient(config);
    let files = git::tracked_files(&config.root)?;
    if let Err(more) = graph::build(&corpus, &config.root, &files) {
        problems.extend(more);
    }
    problems.sort();
    problems.dedup();
    let sources_ok = problems.is_empty();
    let of = |id: &str| -> Vec<Problem> {
        problems
            .iter()
            .filter(|problem| check_of(problem.code) == id)
            .cloned()
            .collect()
    };

    let checks = vec![
        Check::blocking(
            "paths",
            "경로 유효성",
            of("paths"),
            "스펙의 paths, tests를 실제 파일에 맞게 고치세요. 새 파일은 git add가 필요합니다.",
        ),
        Check::blocking(
            "references",
            "참조 유효성",
            of("references"),
            "가리키는 ID를 있는 문서의 ID로 고치세요.",
        ),
        Check::blocking(
            "frontmatter",
            "머리말 형식",
            of("frontmatter"),
            "문제에 적힌 머리말 항목을 고치세요.",
        ),
        freshness_check(&corpus, options.origins),
        index_check(config, &corpus, sources_ok)?,
        guide_check(config)?,
        spec_update_check(
            &corpus,
            changes.as_deref(),
            &options.pr_body.map(parse_no_impact).unwrap_or_default(),
        ),
        unmapped_check(config, &corpus, changes.as_deref()),
    ];
    let report = options
        .report
        .then(|| report::report(config, &corpus))
        .transpose()?;
    Ok(CheckReport { checks, report })
}

const NO_BASE: &str = "--base로 비교 기준을 주지 않았습니다.";

/// 스펙과 그 스펙의 `paths`, `tests` 글롭. 읽을 수 없는 글롭은 경로 유효성이 알리므로 뺀다.
pub(crate) struct SpecGlobs<'a> {
    pub doc: &'a source::Document,
    globs: Vec<GlobMatcher>,
}

impl SpecGlobs<'_> {
    pub fn all(corpus: &source::Corpus) -> Vec<SpecGlobs<'_>> {
        corpus
            .of_kind(source::Kind::Spec)
            .filter_map(|doc| match &doc.record {
                source::Record::Spec(spec) => Some(SpecGlobs {
                    doc,
                    globs: compile(spec.paths.iter().chain(&spec.tests)),
                }),
                _ => None,
            })
            .collect()
    }

    pub fn matches(&self, path: &str) -> bool {
        self.globs.iter().any(|glob| glob.is_match(path))
    }
}

fn compile<'a>(patterns: impl IntoIterator<Item = &'a String>) -> Vec<GlobMatcher> {
    patterns
        .into_iter()
        .filter_map(|pattern| graph::matcher(pattern).ok())
        .collect()
}

/// PR 설명에서 `lore: no-impact <스펙 ID> <사유>` 줄을 읽는다. 스펙 ID → 사유.
///
/// 사유가 없는 줄은 인정하지 않는다. 같은 스펙이 여러 번 나오면 처음 줄을 쓴다.
pub fn parse_no_impact(text: &str) -> BTreeMap<String, String> {
    let mut found = BTreeMap::new();
    for line in text.lines() {
        let Some(rest) = line.trim().strip_prefix("lore: no-impact") else {
            continue;
        };
        if !rest.starts_with(char::is_whitespace) {
            continue;
        }
        let Some((spec, reason)) = rest.trim().split_once(char::is_whitespace) else {
            continue;
        };
        found
            .entry(spec.to_string())
            .or_insert_with(|| reason.trim().to_string());
    }
    found
}

/// 바뀐 파일이 구현함이나 검증함으로 닿는 스펙이 같은 비교 범위에서 바뀌었는지.
///
/// 이름이 바뀐 파일은 원래 경로와 새 경로를 모두 보고, 삭제된 파일은 지워지기 전 경로를 본다.
/// 글롭은 지금 스펙의 것을 쓴다. 기준 커밋과 글롭이 다르면 그 스펙 파일이 바뀐 것이라 결과가 같다.
fn spec_update_check(
    corpus: &source::Corpus,
    changes: Option<&[Change]>,
    no_impact: &BTreeMap<String, String>,
) -> Check {
    const ID: &str = "spec_update";
    const LABEL: &str = "스펙 동반 갱신";
    let Some(changes) = changes else {
        return Check::skipped(ID, LABEL, false, NO_BASE);
    };
    let touched: BTreeSet<&str> = changes
        .iter()
        .flat_map(|change| std::iter::once(change.path.as_str()).chain(change.from.as_deref()))
        .collect();
    let mut findings = Vec::new();
    for spec in SpecGlobs::all(corpus) {
        let files: Vec<String> = touched
            .iter()
            .filter(|path| spec.matches(path))
            .map(|path| path.to_string())
            .collect();
        if files.is_empty() {
            continue;
        }
        let reason = no_impact.get(&spec.doc.id).cloned();
        let outcome = if touched.contains(spec.doc.path.as_str()) {
            Outcome::Updated
        } else if reason.is_some() {
            Outcome::NoImpact
        } else {
            Outcome::Warned
        };
        findings.push(Finding {
            spec: Some(spec.doc.id.clone()),
            files,
            reason: reason.filter(|_| outcome == Outcome::NoImpact),
            outcome,
        });
    }
    Check::judged(
        ID,
        LABEL,
        findings,
        "스펙 본문을 바뀐 코드에 맞게 고치세요. 영향이 없으면 PR 설명에 'lore: no-impact <스펙 ID> <사유>'를 적으세요.",
    )
}

/// 새로 생긴 파일이 어떤 스펙의 `paths`, `tests`에 맞는지.
///
/// 소스 폴더, 설정 파일, lore가 설치한 파일, 설정의 `check.ignore`에 맞는 파일은 보지 않는다.
fn unmapped_check(config: &Config, corpus: &source::Corpus, changes: Option<&[Change]>) -> Check {
    const ID: &str = "unmapped";
    const LABEL: &str = "매핑 누락";
    let Some(changes) = changes else {
        return Check::skipped(ID, LABEL, false, NO_BASE);
    };
    let specs = SpecGlobs::all(corpus);
    let ignore = compile(&config.ignore);
    let installed = install::installed_files();
    let docs = format!("{}/", config.docs);
    let findings = changes
        .iter()
        .filter(|change| matches!(change.kind, ChangeKind::Added | ChangeKind::Renamed))
        .map(|change| change.path.as_str())
        .filter(|path| {
            !path.starts_with(&docs)
                && *path != CONFIG_FILE
                && !installed.iter().any(|file| file == path)
                && !ignore.iter().any(|glob| glob.is_match(path))
                && !specs.iter().any(|spec| spec.matches(path))
        })
        .map(|path| Finding {
            spec: None,
            files: vec![path.to_string()],
            outcome: Outcome::Warned,
            reason: None,
        })
        .collect();
    Check::judged(
        ID,
        LABEL,
        findings,
        "파일을 맡을 스펙의 paths나 tests에 더하세요. 코드가 아니면 lore.config.yaml의 check.ignore에 더하세요.",
    )
}

/// 원본의 최신 판 목록. 원본 링크 → 판(해시나 수정 시각).
///
/// 원본을 읽으려면 네트워크가 필요하므로 코어는 목록을 받아 비교만 한다. 목록은 커넥터가 만든다.
pub type Origins = BTreeMap<String, String>;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OriginEntry {
    origin: String,
    origin_version: String,
}

/// `--origins` 파일을 읽는다. `[{"origin": "...", "origin_version": "..."}]` 꼴이다.
pub fn parse_origins(name: &str, text: &str) -> Result<Origins, LoreError> {
    let invalid = |code, message: String| LoreError::InvalidInput {
        problems: vec![Problem::new(name, None, code, message)],
    };
    let entries: Vec<OriginEntry> =
        serde_json::from_str(text).map_err(|error| invalid("invalid_json", error.to_string()))?;
    let mut origins = Origins::new();
    for entry in entries {
        if let Some(old) = origins.get(&entry.origin)
            && *old != entry.origin_version
        {
            return Err(invalid(
                "duplicate_origin",
                format!("원본 '{}'의 판이 두 가지로 적혀 있습니다.", entry.origin),
            ));
        }
        origins.insert(entry.origin, entry.origin_version);
    }
    Ok(origins)
}

/// 소스 노트에 적힌 원본의 판이 목록의 최신 판과 같은지.
fn freshness_check(corpus: &source::Corpus, origins: Option<&Origins>) -> Check {
    const ID: &str = "freshness";
    const LABEL: &str = "소스 노트 신선도";
    let Some(origins) = origins else {
        return Check::skipped(
            ID,
            LABEL,
            true,
            "--origins로 원본의 최신 판 목록을 주지 않았습니다.",
        );
    };
    let mut problems = Vec::new();
    let mut unchecked = 0;
    for doc in corpus.of_kind(source::Kind::Source) {
        let source::Record::Source(note) = &doc.record else {
            continue;
        };
        match origins.get(&note.origin) {
            None => unchecked += 1,
            Some(latest) if *latest != note.origin_version => problems.push(Problem::new(
                &doc.path,
                Some("origin_version"),
                "stale_source",
                format!(
                    "원본의 판이 '{latest}'로 바뀌었습니다. 소스 노트는 '{}'에서 추출했습니다.",
                    note.origin_version
                ),
            )),
            Some(_) => {}
        }
    }
    let mut check = Check::blocking(
        ID,
        LABEL,
        problems,
        "원본을 다시 정제해 lore add source로 소스 노트를 갱신하세요.",
    );
    check.unchecked = (unchecked > 0).then_some(unchecked);
    check
}

/// 커밋된 인덱스가 소스에서 다시 만든 결과와 같은지.
fn index_check(
    config: &Config,
    corpus: &source::Corpus,
    sources_ok: bool,
) -> Result<Check, LoreError> {
    let path = format!("{}/{}", config.docs, index::FILE_NAME);
    if !sources_ok {
        return Ok(Check::skipped(
            "index",
            "인덱스 일치",
            true,
            "소스에 문제가 있어 인덱스를 다시 만들 수 없습니다.",
        ));
    }
    let expected = index::render(corpus, &config.docs);
    let problem = match read(config, &path)? {
        None => Some(Problem::new(
            &path,
            None,
            "index_missing",
            "인덱스 파일이 없습니다.",
        )),
        Some(text) if text != expected => Some(Problem::new(
            &path,
            None,
            "index_outdated",
            "소스에서 다시 만든 결과와 다릅니다.",
        )),
        Some(_) => None,
    };
    Ok(Check::blocking(
        "index",
        "인덱스 일치",
        problem.into_iter().collect(),
        "lore build를 실행하고 바뀐 인덱스를 커밋하세요.",
    ))
}

/// `AGENTS.md`와 도구별 파일의 lore 구간이 지금 바이너리의 안내문과 같은지.
fn guide_check(config: &Config) -> Result<Check, LoreError> {
    let mut problems = Vec::new();
    match read(config, AGENTS_FILE)? {
        None => problems.push(Problem::new(
            AGENTS_FILE,
            None,
            "guide_missing",
            "파일이 없습니다.",
        )),
        Some(text) => problems.extend(section_problem(&text, AGENTS_BODY, AGENTS_FILE)),
    }
    // 도구별 파일은 있을 때만 본다.
    for file in TOOL_FILES {
        if let Some(text) = read(config, file)? {
            problems.extend(section_problem(&text, TOOL_POINTER, file));
        }
    }
    Ok(Check::blocking(
        "guide",
        "안내문 일치",
        problems,
        "lore install을 실행하고 바뀐 파일을 커밋하세요.",
    ))
}

fn section_problem(text: &str, body: &str, path: &str) -> Option<Problem> {
    let (code, message) = match install::find_section(text, path) {
        Err(_) => (
            "broken_marker",
            "lore 구간의 표식이 깨져 있습니다. 시작과 끝 표식이 하나씩 있어야 합니다.",
        ),
        Ok(None) => ("guide_missing", "lore 구간이 없습니다."),
        Ok(Some((start, end))) if text[start..end] != install::section(body) => (
            "guide_outdated",
            "lore 구간이 이 lore 버전의 안내문과 다릅니다.",
        ),
        Ok(Some(_)) => return None,
    };
    Some(Problem::new(path, None, code, message))
}

fn read(config: &Config, path: &str) -> Result<Option<String>, LoreError> {
    match std::fs::read_to_string(config.root.join(path)) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(LoreError::Io {
            path: path.to_string(),
            message: error.to_string(),
        }),
    }
}

/// 사람용 검사 결과. 실패, 경고, 건너뜀, 통과 순으로 보여 준다.
pub fn render(report: &CheckReport) -> String {
    let counts: Vec<String> = Status::ALL
        .into_iter()
        .filter_map(|status| {
            let count = report.checks.iter().filter(|c| c.status == status).count();
            (count > 0).then(|| format!("{} {count}", status.label()))
        })
        .collect();
    let mut text = format!("lore check: {}", counts.join(", "));
    for status in Status::ALL {
        for check in report.checks.iter().filter(|c| c.status == status) {
            text.push_str(&format!("\n\n{}  {}", status.label(), check.label));
            for problem in &check.problems {
                text.push_str(&format!("\n  {problem}"));
            }
            for finding in &check.findings {
                if let Some(line) = render_finding(finding) {
                    text.push_str(&format!("\n  {line}"));
                }
            }
            if let Some(fix) = check.fix {
                text.push_str(&format!("\n  고치려면: {fix}"));
            }
            if let Some(reason) = check.reason {
                text.push_str(&format!("\n  {reason}"));
            }
            if let Some(count) = check.unchecked {
                text.push_str(&format!("\n  목록에 없어 확인하지 못한 원본 {count}건"));
            }
        }
    }
    if let Some(report) = &report.report {
        text.push_str(&format!("\n\n{}", report::render(report)));
    }
    text
}

/// 기록 한 줄. 스펙도 함께 바뀐 것은 따로 보여 주지 않는다.
fn render_finding(finding: &Finding) -> Option<String> {
    let files = finding.files.join(", ");
    Some(match (&finding.spec, finding.outcome) {
        (_, Outcome::Updated) => return None,
        (Some(spec), Outcome::Warned) => {
            format!("{spec} 스펙이 바뀌지 않았습니다. 바뀐 파일: {files}")
        }
        (Some(spec), Outcome::NoImpact) => format!(
            "{spec} 영향 없음으로 넘김: {}",
            finding.reason.as_deref().unwrap_or_default()
        ),
        (None, _) => format!("{files} 어떤 스펙의 paths, tests에도 맞지 않습니다."),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 사유가_있는_no_impact_줄만_읽는다() {
        let body = "설명\r\nlore: no-impact payment-retry 로그 문구만 바꿈\r\n  lore: no-impact payment-core\nlore: no-impactpayment\nlore: no-impact payment-retry 두 번째\n";
        let found = parse_no_impact(body);
        assert_eq!(found.len(), 1);
        assert_eq!(found["payment-retry"], "로그 문구만 바꿈");
    }

    #[test]
    fn 문제_코드를_검사로_나눈다() {
        assert_eq!(check_of("no_matching_files"), "paths");
        assert_eq!(check_of("invalid_glob"), "paths");
        assert_eq!(check_of("unknown_reference"), "references");
        assert_eq!(check_of("invalid_reference"), "references");
        for code in ["missing_field", "id_mismatch", "duplicate_id", "unreadable"] {
            assert_eq!(check_of(code), "frontmatter");
        }
    }
}

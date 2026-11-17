//! `lore check`: 소스와 코드가 어긋났는지 검사한다. 파일은 고치지 않는다.
//!
//! 지금은 기계적 검사만 있다. 판단이 없는 검사라 실패하면 병합을 막는다.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::error::{LoreError, Problem};
use crate::install::{self, AGENTS_BODY, AGENTS_FILE, TOOL_FILES, TOOL_POINTER};
use crate::{git, graph, index, source};

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
    /// 검사 이름. `paths`, `references`, `frontmatter`, `freshness`, `index`, `guide`
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
        }
    }

    fn skipped(id: &'static str, label: &'static str, reason: &'static str) -> Check {
        Check {
            id,
            label,
            status: Status::Skip,
            blocking: true,
            problems: Vec::new(),
            fix: None,
            reason: Some(reason),
            unchecked: None,
        }
    }
}

/// 검사 결과. JSON 출력이 된다.
#[derive(Debug, Serialize)]
pub struct CheckReport {
    pub checks: Vec<Check>,
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

pub fn check(config: &Config, origins: Option<&Origins>) -> Result<CheckReport, LoreError> {
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
        freshness_check(&corpus, origins),
        index_check(config, &corpus, sources_ok)?,
        guide_check(config)?,
    ];
    Ok(CheckReport { checks })
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
    text
}

#[cfg(test)]
mod tests {
    use super::*;

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

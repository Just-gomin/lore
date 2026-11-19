//! `lore init`: 기존 프로젝트에 lore를 들이는 세 단계 중 어디까지 왔는지 낸다. 파일은 고치지 않는다.
//!
//! 채널을 고르고(1단계), 자료를 모으고(2단계), 스펙과 결정 초안을 쓰는(3단계) 일은 판단이 필요해
//! `lore-init-*` 스킬이 맡는다. 명령은 스킬이 어디서 다시 시작할지 알 수 있게 상태만 낸다.

use std::path::Path;

use serde::Serialize;

use crate::check::Coverage;
use crate::config::Config;
use crate::error::{LoreError, Problem};
use crate::{git, query, source};

#[derive(Debug, Serialize)]
pub struct InitReport {
    /// 다음에 할 단계. `channels`, `collect`, `draft`, `done`.
    /// 범위에 스펙이 맡을 코드 파일이 하나도 없으면 `no_files`다. 아직 `git add`하지 않은 폴더일 수 있다.
    pub next: &'static str,
    pub channels: Vec<ChannelStatus>,
    pub sources: usize,
    pub specs: usize,
    pub coverage: CoverageReport,
}

#[derive(Debug, Serialize)]
pub struct ChannelStatus {
    pub id: String,
    pub sensitivity: &'static str,
    /// 수집 경로. 설정에 없으면 없다.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
}

/// 대상 범위의 코드 파일이 스펙에 얼마나 연결됐는지.
#[derive(Debug, Serialize)]
pub struct CoverageReport {
    /// 프로젝트 홈 기준 폴더. 전체면 빈 문자열이다.
    pub scope: String,
    /// 스펙이 맡아야 할 파일 수. 소스 폴더, 설정, lore가 설치한 파일, `check.ignore`는 뺀다.
    pub files: usize,
    pub mapped: usize,
    /// 대상 파일이 없으면 없다.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ratio: Option<f64>,
    /// 어떤 스펙에도 속하지 않은 파일.
    pub unmapped: Vec<String>,
}

pub fn init(config: &Config, cwd: &Path, scope: Option<&str>) -> Result<InitReport, LoreError> {
    let scope = match scope {
        None => String::new(),
        Some(input) => query::project_path(&config.root, cwd, input)
            .filter(|path| config.root.join(path).is_dir())
            .ok_or_else(|| LoreError::InvalidInput {
                problems: vec![Problem::new(
                    input,
                    None,
                    "invalid_scope",
                    "프로젝트 홈 안의 폴더가 아닙니다.",
                )],
            })?,
    };
    let (corpus, _) = source::read_lenient(config);
    let coverage = Coverage::new(config, &corpus);
    let prefix = if scope.is_empty() {
        String::new()
    } else {
        format!("{scope}/")
    };
    let files: Vec<String> = git::tracked_files(&config.root)?
        .into_iter()
        .filter(|path| path.starts_with(&prefix) && coverage.is_code(path))
        .collect();
    let unmapped: Vec<String> = files
        .iter()
        .filter(|path| !coverage.is_mapped(path))
        .cloned()
        .collect();
    let mapped = files.len() - unmapped.len();

    let channels: Vec<ChannelStatus> = config
        .channels
        .iter()
        .map(|channel| ChannelStatus {
            id: channel.id.clone(),
            sensitivity: channel.sensitivity.name(),
            tool: channel.tool.clone(),
        })
        .collect();
    let sources = corpus.of_kind(source::Kind::Source).count();
    let specs = corpus.of_kind(source::Kind::Spec).count();
    let next = if channels.is_empty() {
        "channels"
    } else if sources == 0 {
        "collect"
    } else if files.is_empty() {
        "no_files"
    } else if !unmapped.is_empty() {
        "draft"
    } else {
        "done"
    };
    Ok(InitReport {
        next,
        channels,
        sources,
        specs,
        coverage: CoverageReport {
            scope,
            files: files.len(),
            mapped,
            // 버림으로 계산해 속하지 않은 파일이 남았는데 1.0이 되지 않게 합니다.
            ratio: (!files.is_empty())
                .then(|| (mapped as f64 / files.len() as f64 * 100.0).floor() / 100.0),
            unmapped,
        },
    })
}

/// 사람용 결과.
pub fn render(report: &InitReport) -> String {
    let step = match report.next {
        "channels" => "1단계 채널 고르기 (lore-init-channels)",
        "collect" => "2단계 자료 모으기 (lore-init-collect)",
        "draft" => "3단계 스펙과 결정 초안 (lore-init-draft)",
        "no_files" => "범위에 스펙이 맡을 코드 파일이 없음. 새 파일이면 git add가 필요합니다",
        _ => "끝",
    };
    let coverage = &report.coverage;
    let scope = if coverage.scope.is_empty() {
        "프로젝트 전체"
    } else {
        &coverage.scope
    };
    let mut text = format!(
        "lore init: 다음은 {step}\n  채널 {}개, 소스 노트 {}개, 스펙 {}개\n  {scope}: 코드 파일 {}개 중 {}개가 스펙에 속함",
        report.channels.len(),
        report.sources,
        report.specs,
        coverage.files,
        coverage.mapped
    );
    if let Some(ratio) = coverage.ratio {
        text.push_str(&format!(" ({}%)", (ratio * 100.0).floor()));
    }
    for path in &coverage.unmapped {
        text.push_str(&format!("\n    {path}"));
    }
    text
}

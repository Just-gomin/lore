//! `lore build`: 소스를 읽어 디비를 만든다. 2단계에서는 매번 전체를 다시 만든다.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::config::Config;
use crate::db::{self, Meta};
use crate::error::LoreError;
use crate::{git, graph, source};

/// lore가 만드는 파일을 두는 폴더. 프로젝트 홈 기준이다.
pub const STATE_DIR: &str = ".lore";
/// 디비 파일. 프로젝트 홈 기준이다.
pub const DB_PATH: &str = ".lore/lore.db";

/// 빌드 결과. JSON 출력이 된다.
#[derive(Debug, Serialize)]
pub struct BuildReport {
    pub db: String,
    pub commit: Option<String>,
    pub nodes: BTreeMap<&'static str, usize>,
    pub edges: BTreeMap<&'static str, usize>,
}

pub fn db_path(config: &Config) -> PathBuf {
    config.root.join(DB_PATH)
}

/// 소스를 읽어 디비를 새로 만든다.
pub fn build(config: &Config) -> Result<BuildReport, LoreError> {
    let corpus = source::read(config)?;
    let files = git::tracked_files(&config.root)?;
    let graph = graph::build(&corpus, &config.root, &files)
        .map_err(|problems| LoreError::InvalidSources { problems })?;

    prepare_state_dir(&config.root)?;
    let commit = git::head_commit(&config.root);
    let meta = Meta {
        commit: commit.clone().unwrap_or_default(),
        fingerprint: fingerprint(&config.root)?,
    };
    db::write(&db_path(config), &graph, &meta)?;

    Ok(BuildReport {
        db: DB_PATH.to_string(),
        commit,
        nodes: graph.count_nodes(),
        edges: graph.count_edges(),
    })
}

/// 디비가 기억하는 작업 트리 상태와 비교할 지금의 값.
pub fn fingerprint(root: &Path) -> Result<String, LoreError> {
    git::worktree_fingerprint(root, &format!("{STATE_DIR}/"))
}

/// `.lore/`를 만들고, 안의 파일을 git이 모두 무시하게 한다.
fn prepare_state_dir(root: &Path) -> Result<(), LoreError> {
    let dir = root.join(STATE_DIR);
    let io_error = |error: std::io::Error| LoreError::Io {
        path: STATE_DIR.to_string(),
        message: error.to_string(),
    };
    std::fs::create_dir_all(&dir).map_err(io_error)?;
    let ignore = dir.join(".gitignore");
    if !ignore.exists() {
        std::fs::write(
            &ignore,
            "# lore가 만드는 빌드 산출물입니다. 커밋하지 않습니다.\n*\n",
        )
        .map_err(io_error)?;
    }
    Ok(())
}

/// 사람용 빌드 결과.
pub fn render(report: &BuildReport) -> String {
    let total = |counts: &BTreeMap<&'static str, usize>| counts.values().sum::<usize>();
    let mut text = format!(
        "lore build: 노드 {}개, 엣지 {}개\n  디비  {}",
        total(&report.nodes),
        total(&report.edges),
        report.db
    );
    if let Some(commit) = &report.commit {
        text.push_str(&format!("\n  커밋  {}", &commit[..commit.len().min(12)]));
    }
    text
}

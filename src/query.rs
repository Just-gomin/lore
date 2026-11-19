//! `lore why`와 `lore impact`가 함께 쓰는 조회 준비: 디비를 최신으로 맞추고 대상을 노드로 바꾼다.

use std::path::Path;

use rusqlite::{Connection, OptionalExtension};

use crate::build;
use crate::config::Config;
use crate::db::{self, database_error};
use crate::error::LoreError;

/// 디비를 최신으로 맞춰 열고 대상을 해석한다. 다시 빌드했는지, 노드 ID, 노드 종류를 함께 돌려준다.
pub fn open_target(
    config: &Config,
    cwd: &Path,
    input: &str,
) -> Result<(Connection, bool, String, String), LoreError> {
    let rebuilt = ensure_fresh(config)?;
    let conn = db::open(&build::db_path(config))?;
    let (id, kind) =
        resolve(&conn, config, cwd, input)?.ok_or_else(|| LoreError::TargetNotFound {
            target: input.to_string(),
        })?;

    Ok((conn, rebuilt, id, kind))
}

/// 디비가 지금의 작업 트리로 만든 것인지 보고, 아니면 다시 만든다.
fn ensure_fresh(config: &Config) -> Result<bool, LoreError> {
    let path = build::db_path(config);
    if path.exists() {
        let conn = db::open(&path)?;
        let schema = db::read_meta(&conn, "schema_version");
        let version = db::read_meta(&conn, "lore_version");
        let fingerprint = db::read_meta(&conn, "fingerprint");
        drop(conn);
        if schema.as_deref() == Some(db::SCHEMA_VERSION)
            && version.as_deref() == Some(db::LORE_VERSION)
            && fingerprint == Some(build::fingerprint(&config.root)?)
        {
            return Ok(false);
        }
    }
    build::build(config)?;
    Ok(true)
}

/// 입력을 노드 ID와 종류로 바꾼다. ID가 먼저이고, 그다음 파일 경로다.
///
/// 파일 경로는 현재 폴더 기준으로 읽고, 프로젝트 홈 밖이면 프로젝트 홈 기준으로 다시 읽는다.
fn resolve(
    conn: &Connection,
    config: &Config,
    cwd: &Path,
    input: &str,
) -> Result<Option<(String, String)>, LoreError> {
    let mut candidates = vec![input.to_string()];
    if let Some(path) = project_path(&config.root, cwd, input) {
        candidates.push(path);
    }
    // 심볼은 `#` 앞의 파일 경로만 프로젝트 홈 기준으로 바꾼다.
    if let Some((file, name)) = input.rsplit_once('#')
        && let Some(path) = project_path(&config.root, cwd, file)
    {
        candidates.push(format!("{path}#{name}"));
    }
    candidates.push(input.trim_start_matches("./").to_string());

    for id in candidates {
        let kind: Option<String> = conn
            .query_row("SELECT kind FROM nodes WHERE id = ?1", [&id], |row| {
                row.get(0)
            })
            .optional()
            .map_err(database_error)?;
        if let Some(kind) = kind {
            return Ok(Some((id, kind)));
        }
    }
    Ok(None)
}

/// 현재 폴더 기준 경로를 프로젝트 홈 기준 경로로 바꾼다. 파일이 없거나 홈 밖이면 `None`이다.
pub(crate) fn project_path(root: &Path, cwd: &Path, input: &str) -> Option<String> {
    let root = root.canonicalize().ok()?;
    let file = cwd.join(input).canonicalize().ok()?;
    let relative = file.strip_prefix(&root).ok()?;
    let parts: Vec<String> = relative
        .components()
        .map(|part| part.as_os_str().to_string_lossy().into_owned())
        .collect();
    Some(parts.join("/"))
}

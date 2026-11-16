//! SQLite 디비의 스키마와 쓰기. 테이블은 기획서 3장의 네 개다.

use std::path::Path;

use rusqlite::{Connection, OpenFlags, params};

use crate::error::LoreError;
use crate::graph::Graph;

/// 스키마가 바뀌면 올린다. 다른 버전의 디비는 다시 만든다.
pub const SCHEMA_VERSION: &str = "1";

const SCHEMA: &str = "
CREATE TABLE meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
CREATE TABLE nodes (
    id           TEXT PRIMARY KEY,
    kind         TEXT NOT NULL,
    title        TEXT,
    summary      TEXT,
    status       TEXT,
    source_path  TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    frontmatter  TEXT
);
CREATE TABLE edges (
    src    TEXT NOT NULL,
    dst    TEXT NOT NULL,
    kind   TEXT NOT NULL,
    origin TEXT NOT NULL,
    PRIMARY KEY (src, dst, kind)
);
CREATE INDEX edges_by_dst ON edges (dst, kind);
CREATE VIRTUAL TABLE content USING fts5 (id UNINDEXED, title, body, tokenize = 'trigram');
";

pub fn database_error(error: rusqlite::Error) -> LoreError {
    LoreError::Database {
        message: error.to_string(),
    }
}

/// `meta` 테이블에 적는 값.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Meta {
    /// 빌드 시점의 커밋. 커밋이 없는 레포면 빈 문자열이다.
    pub commit: String,
    /// 빌드 시점의 작업 트리 상태. 조회 전에 다시 빌드할지 정할 때 쓴다.
    pub fingerprint: String,
}

/// 그래프를 `path`에 새 디비로 쓴다. 같은 이름의 임시 파일에 쓴 뒤 바꿔 끼운다.
pub fn write(path: &Path, graph: &Graph, meta: &Meta) -> Result<(), LoreError> {
    let temp = path.with_extension("db.tmp");
    let io_error = |error: std::io::Error| LoreError::Io {
        path: temp.display().to_string(),
        message: error.to_string(),
    };
    if temp.exists() {
        std::fs::remove_file(&temp).map_err(io_error)?;
    }

    let mut conn = Connection::open(&temp).map_err(database_error)?;
    conn.execute_batch(SCHEMA).map_err(database_error)?;
    let tx = conn.transaction().map_err(database_error)?;
    {
        let mut insert_meta = tx
            .prepare("INSERT INTO meta (key, value) VALUES (?1, ?2)")
            .map_err(database_error)?;
        for (key, value) in [
            ("schema_version", SCHEMA_VERSION),
            ("commit", meta.commit.as_str()),
            ("fingerprint", meta.fingerprint.as_str()),
        ] {
            insert_meta
                .execute(params![key, value])
                .map_err(database_error)?;
        }

        let mut insert_node = tx
            .prepare(
                "INSERT INTO nodes (id, kind, title, summary, status, source_path, content_hash, frontmatter)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            )
            .map_err(database_error)?;
        let mut insert_content = tx
            .prepare("INSERT INTO content (id, title, body) VALUES (?1, ?2, ?3)")
            .map_err(database_error)?;
        for node in &graph.nodes {
            insert_node
                .execute(params![
                    node.id,
                    node.kind,
                    node.title,
                    node.summary,
                    node.status,
                    node.source_path,
                    node.content_hash,
                    node.frontmatter,
                ])
                .map_err(database_error)?;
            if let Some(body) = &node.body {
                insert_content
                    .execute(params![node.id, node.title, body])
                    .map_err(database_error)?;
            }
        }

        let mut insert_edge = tx
            .prepare("INSERT INTO edges (src, dst, kind, origin) VALUES (?1, ?2, ?3, ?4)")
            .map_err(database_error)?;
        for edge in &graph.edges {
            insert_edge
                .execute(params![edge.src, edge.dst, edge.kind.name(), edge.origin])
                .map_err(database_error)?;
        }
    }
    tx.commit().map_err(database_error)?;
    conn.close().map_err(|(_, error)| database_error(error))?;

    std::fs::rename(&temp, path).map_err(|error| LoreError::Io {
        path: path.display().to_string(),
        message: error.to_string(),
    })
}

/// 디비를 읽기 전용으로 연다.
pub fn open(path: &Path) -> Result<Connection, LoreError> {
    Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(database_error)
}

/// `meta`에서 값 하나를 읽는다. 디비가 없거나 값이 없으면 `None`이다.
pub fn read_meta(conn: &Connection, key: &str) -> Option<String> {
    conn.query_row("SELECT value FROM meta WHERE key = ?1", [key], |row| {
        row.get(0)
    })
    .ok()
}

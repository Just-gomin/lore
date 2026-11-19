//! lore 커넥터가 함께 쓰는 출력 형식과 소스 노트 읽기.
//!
//! 커넥터는 lore 코어에 의존하지 않습니다. 형식은 기획서 6장의 커넥터 규약으로만 맞춥니다.

use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_yaml_ng::Value;

/// 소스 노트 후보 하나. `lore add source` 입력에서 `slug`와 `body`를 빼고, 정제하기 전 원문을 더했습니다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Candidate {
    pub title: String,
    /// `discussion`, `client-doc`, `evidence`, `reference`
    pub kind: String,
    pub channel: String,
    pub origin: String,
    pub origin_version: String,
    /// `YYYY-MM-DD`
    pub occurred_at: String,
    /// 정제하기 전 원문. 스킬이 읽고 결정에 관련된 내용만 `body`로 옮깁니다.
    pub text: String,
}

/// 원본 하나의 최신 판. `lore check --origins`가 받는 형식입니다.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct OriginVersion {
    pub origin: String,
    pub origin_version: String,
}

/// 커넥터가 실패한 이유. 표준 출력의 JSON `error`가 됩니다.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConnectorError {
    pub code: &'static str,
    pub message: String,
}

impl ConnectorError {
    pub fn new(code: &'static str, message: impl Into<String>) -> ConnectorError {
        ConnectorError {
            code,
            message: message.into(),
        }
    }
}

/// 후보 출력: `{"ok": true, "candidates": [...]}`
pub fn candidates_json(candidates: &[Candidate]) -> String {
    serde_json::json!({ "ok": true, "candidates": candidates }).to_string()
}

/// 최신 판 출력. `lore check --origins`에 그대로 넘기도록 감싸지 않은 배열입니다.
pub fn origins_json(origins: &[OriginVersion]) -> String {
    serde_json::to_string(origins).expect("문자열만 담은 구조체는 직렬화된다")
}

/// 오류 출력: `{"ok": false, "error": {"code": ..., "message": ...}}`
pub fn error_json(error: &ConnectorError) -> String {
    serde_json::json!({ "ok": false, "error": error }).to_string()
}

/// 소스 폴더의 `sources/`에 있는 소스 노트에서 `origin`과 `origin_version`을 읽습니다. `origin` 순입니다.
///
/// 머리말이 없거나 두 항목이 문자열이 아닌 파일은 건너뜁니다. 형식 검사는 `lore check`가 맡습니다.
pub fn read_source_origins(docs: &Path) -> Result<Vec<OriginVersion>, ConnectorError> {
    let dir = docs.join("sources");
    let entries = match std::fs::read_dir(&dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(ConnectorError::new(
                "io",
                format!("{}: {error}", dir.display()),
            ));
        }
    };
    let mut origins = Vec::new();
    for entry in entries {
        let path = entry
            .map_err(|error| ConnectorError::new("io", error.to_string()))?
            .path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("md") {
            continue;
        }
        let text = std::fs::read_to_string(&path)
            .map_err(|error| ConnectorError::new("io", format!("{}: {error}", path.display())))?;
        if let Some(origin) = origin_of(&text) {
            origins.push(origin);
        }
    }
    origins.sort();
    Ok(origins)
}

/// 소스 노트의 머리말에서 `origin`과 `origin_version`을 읽습니다.
fn origin_of(text: &str) -> Option<OriginVersion> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let rest = text.strip_prefix("---")?;
    let rest = rest
        .strip_prefix("\r\n")
        .or_else(|| rest.strip_prefix('\n'))?;
    let mut yaml_end = None;
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        if line.trim_end_matches(['\r', '\n']) == "---" {
            yaml_end = Some(offset);
            break;
        }
        offset += line.len();
    }
    let Value::Mapping(map) = serde_yaml_ng::from_str(&rest[..yaml_end?]).ok()? else {
        return None;
    };
    let field = |key: &str| map.get(key)?.as_str().map(str::to_string);
    Some(OriginVersion {
        origin: field("origin")?,
        origin_version: field("origin_version")?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 소스_노트의_원본과_판을_읽고_머리말이_없는_파일은_건너뛴다() {
        let docs = tempfile::tempdir().unwrap();
        let sources = docs.path().join("sources");
        std::fs::create_dir_all(&sources).unwrap();
        std::fs::write(
            sources.join("SRC-20261103-b.md"),
            "---\nid: SRC-20261103-b\norigin: https://example.com/b\norigin_version: '2'\n---\n본문\n",
        )
        .unwrap();
        std::fs::write(
            sources.join("SRC-20261020-a.md"),
            "---\r\norigin: https://example.com/a\r\norigin_version: abc\r\n---\r\n",
        )
        .unwrap();
        std::fs::write(sources.join("README.md"), "# 소스\n").unwrap();
        std::fs::write(sources.join(".gitkeep"), "").unwrap();

        assert_eq!(
            read_source_origins(docs.path()).unwrap(),
            [
                OriginVersion {
                    origin: "https://example.com/a".to_string(),
                    origin_version: "abc".to_string(),
                },
                OriginVersion {
                    origin: "https://example.com/b".to_string(),
                    origin_version: "2".to_string(),
                },
            ]
        );
    }

    #[test]
    fn 소스_폴더가_없으면_빈_목록이다() {
        let docs = tempfile::tempdir().unwrap();
        assert!(read_source_origins(docs.path()).unwrap().is_empty());
    }

    #[test]
    fn 출력은_규약의_모양이다() {
        let candidate = Candidate {
            title: "t".to_string(),
            kind: "evidence".to_string(),
            channel: "git".to_string(),
            origin: "git:abc".to_string(),
            origin_version: "abc".to_string(),
            occurred_at: "2026-11-19".to_string(),
            text: "x".to_string(),
        };
        let value: serde_json::Value =
            serde_json::from_str(&candidates_json(&[candidate])).unwrap();
        assert_eq!(value["ok"], true);
        assert_eq!(value["candidates"][0]["origin"], "git:abc");
        assert_eq!(
            origins_json(&[OriginVersion {
                origin: "o".to_string(),
                origin_version: "v".to_string(),
            }]),
            r#"[{"origin":"o","origin_version":"v"}]"#
        );
        let error = error_json(&ConnectorError::new("git", "실패"));
        assert_eq!(
            error,
            r#"{"error":{"code":"git","message":"실패"},"ok":false}"#
        );
    }
}

//! `lore.config.yaml`을 찾고 읽는다.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::LoreError;

pub const CONFIG_FILE: &str = "lore.config.yaml";

/// 2단계에서 쓰는 설정 항목. 나머지 항목은 읽고 버린다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// 프로젝트 홈. 설정 파일이 있는 폴더다.
    pub root: PathBuf,
    /// 소스 폴더. 프로젝트 홈 기준 경로다.
    pub docs: String,
    /// 자료를 가져오는 채널. `lore add source` 입력의 `channel`과 짝을 이룬다.
    pub channels: Vec<Channel>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Channel {
    pub id: String,
    pub sensitivity: Sensitivity,
}

/// 출처별 민감도 등급. 기획서 2장의 세 등급이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Sensitivity {
    Normal,
    Sensitive,
    Excluded,
}

impl Sensitivity {
    pub fn from_name(name: &str) -> Option<Sensitivity> {
        match name {
            "normal" => Some(Sensitivity::Normal),
            "sensitive" => Some(Sensitivity::Sensitive),
            "excluded" => Some(Sensitivity::Excluded),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Sensitivity::Normal => "normal",
            Sensitivity::Sensitive => "sensitive",
            Sensitivity::Excluded => "excluded",
        }
    }
}

impl Config {
    /// 채널의 등급. 설정에 없는 채널은 민감으로 다룬다.
    pub fn sensitivity(&self, channel: &str) -> Sensitivity {
        self.channels
            .iter()
            .find(|known| known.id == channel)
            .map_or(Sensitivity::Sensitive, |known| known.sensitivity)
    }
}

#[derive(Deserialize)]
struct RawConfig {
    version: Option<u32>,
    #[serde(default)]
    project: RawProject,
    #[serde(default)]
    channels: Vec<RawChannel>,
}

#[derive(Deserialize)]
struct RawChannel {
    id: String,
    sensitivity: Option<String>,
}

#[derive(Deserialize, Default)]
struct RawProject {
    docs: Option<String>,
}

/// `start`부터 위로 올라가며 설정 파일이 있는 폴더를 찾는다.
pub fn find_root(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .find(|dir| dir.join(CONFIG_FILE).is_file())
        .map(Path::to_path_buf)
}

/// `start`에서 프로젝트 홈을 찾아 설정을 읽는다.
pub fn discover(start: &Path) -> Result<Config, LoreError> {
    let root = find_root(start).ok_or_else(|| LoreError::ConfigNotFound {
        start: start.display().to_string(),
    })?;
    load(&root)
}

/// 프로젝트 홈의 설정 파일을 읽는다.
pub fn load(root: &Path) -> Result<Config, LoreError> {
    let path = root.join(CONFIG_FILE);
    let text = std::fs::read_to_string(&path).map_err(|error| LoreError::Io {
        path: CONFIG_FILE.to_string(),
        message: error.to_string(),
    })?;
    parse(root, &text)
}

fn parse(root: &Path, text: &str) -> Result<Config, LoreError> {
    let invalid = |message: String| LoreError::InvalidConfig { message };
    let raw: RawConfig =
        serde_yaml_ng::from_str(text).map_err(|error| invalid(error.to_string()))?;

    match raw.version {
        Some(1) => {}
        Some(other) => {
            return Err(invalid(format!(
                "version {other}은 지원하지 않습니다. 1이어야 합니다."
            )));
        }
        None => return Err(invalid("version 항목이 없습니다.".to_string())),
    }

    let docs = raw.project.docs.unwrap_or_else(|| "docs".to_string());
    let docs = docs.trim_end_matches('/').to_string();
    if docs.is_empty() || Path::new(&docs).is_absolute() || docs.split('/').any(|part| part == "..")
    {
        return Err(invalid(format!(
            "project.docs '{docs}'는 프로젝트 홈 안의 상대 경로여야 합니다."
        )));
    }

    let mut channels = Vec::new();
    for raw in raw.channels {
        let sensitivity = match raw.sensitivity.as_deref() {
            None => Sensitivity::Sensitive,
            Some(name) => Sensitivity::from_name(name).ok_or_else(|| {
                invalid(format!(
                    "채널 '{}'의 sensitivity '{name}'는 normal, sensitive, excluded 중 하나여야 합니다.",
                    raw.id
                ))
            })?,
        };
        channels.push(Channel {
            id: raw.id,
            sensitivity,
        });
    }

    Ok(Config {
        root: root.to_path_buf(),
        docs,
        channels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(text: &str) -> Result<Config, LoreError> {
        parse(Path::new("/repo"), text)
    }

    #[test]
    fn 모르는_항목은_무시하고_docs를_읽는다() {
        let text = "version: 1\nproject:\n  docs: notes/\nchannels:\n  - id: git\n";
        assert_eq!(config(text).unwrap().docs, "notes");
    }

    #[test]
    fn 채널_등급을_읽고_없는_채널은_민감으로_본다() {
        let text = "version: 1\nchannels:\n  - id: git\n    sensitivity: normal\n  - id: hr\n    sensitivity: excluded\n  - id: memo\n";
        let config = config(text).unwrap();
        assert_eq!(config.sensitivity("git"), Sensitivity::Normal);
        assert_eq!(config.sensitivity("hr"), Sensitivity::Excluded);
        assert_eq!(config.sensitivity("memo"), Sensitivity::Sensitive);
        assert_eq!(config.sensitivity("kakaotalk"), Sensitivity::Sensitive);
        assert!(matches!(
            parse(
                Path::new("/repo"),
                "version: 1\nchannels:\n  - id: a\n    sensitivity: secret\n"
            ),
            Err(LoreError::InvalidConfig { .. })
        ));
    }

    #[test]
    fn docs가_없으면_docs_폴더를_쓴다() {
        assert_eq!(config("version: 1\n").unwrap().docs, "docs");
    }

    #[test]
    fn 버전이_1이_아니면_오류다() {
        assert!(matches!(
            config("version: 2\n"),
            Err(LoreError::InvalidConfig { .. })
        ));
        assert!(matches!(
            config("project: {}\n"),
            Err(LoreError::InvalidConfig { .. })
        ));
    }

    #[test]
    fn docs가_홈_밖을_가리키면_오류다() {
        let text = "version: 1\nproject:\n  docs: ../docs\n";
        assert!(matches!(config(text), Err(LoreError::InvalidConfig { .. })));
    }
}

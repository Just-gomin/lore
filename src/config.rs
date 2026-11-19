//! `lore.config.yaml`을 찾고 읽는다.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::error::LoreError;

pub const CONFIG_FILE: &str = "lore.config.yaml";

/// lore가 쓰는 설정 항목. 나머지 항목은 읽고 버린다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// 프로젝트 홈. 설정 파일이 있는 폴더다.
    pub root: PathBuf,
    /// 소스 폴더. 프로젝트 홈 기준 경로다.
    pub docs: String,
    /// 자료를 가져오는 채널. `lore add source` 입력의 `channel`과 짝을 이룬다.
    pub channels: Vec<Channel>,
    /// 어떤 스펙에도 속하지 않아도 되는 파일의 글롭. 매핑 누락 검사에서 뺀다.
    pub ignore: Vec<String>,
    /// 정적 분석 설정. 없으면 분석하지 않는다.
    pub analysis: Option<Analysis>,
    /// 판단이 드는 검사가 볼 파일의 글롭. 없으면 전체, 빈 목록이면 아무 파일도 보지 않는다.
    /// `lore init`을 마친 모듈부터 검사를 켜기 위해서다.
    pub scope: Option<Vec<String>>,
    /// 파일 확장자마다 쓸 추출기. 코어는 형식만 검사하고 실행하지 않는다. 스킬이 읽는다.
    pub extractors: Vec<Extractor>,
}

/// `extractors`의 항목. 기획서 6장의 추출기 규약을 따르는 실행 파일이다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Extractor {
    pub command: String,
    /// 점 없는 소문자 확장자
    pub extensions: Vec<String>,
}

/// `analysis` 항목. `language`가 있어야 분석한다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Analysis {
    /// `typescript`: TypeScript와 JavaScript를 함께 읽는다.
    pub language: String,
    /// `pnpm`이면 워크스페이스의 패키지 이름을 레포 안의 파일로 해석한다.
    pub workspace: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Channel {
    pub id: String,
    pub sensitivity: Sensitivity,
    /// 수집 경로. 커넥터 실행 파일 이름, `agent`(에이전트 도구), `file`(파일 가져오기). 코어는 읽어서 보여 주기만 한다.
    pub tool: Option<String>,
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
    #[serde(default)]
    check: RawCheck,
    analysis: Option<RawAnalysis>,
    #[serde(default)]
    extractors: Vec<RawExtractor>,
}

#[derive(Deserialize)]
struct RawExtractor {
    command: String,
    #[serde(default)]
    extensions: Vec<String>,
}

#[derive(Deserialize)]
struct RawAnalysis {
    language: Option<String>,
    workspace: Option<String>,
}

#[derive(Deserialize, Default)]
struct RawCheck {
    #[serde(default)]
    ignore: Vec<String>,
    scope: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct RawChannel {
    id: String,
    sensitivity: Option<String>,
    tool: Option<String>,
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
            tool: raw.tool,
        });
    }

    for (field, patterns) in [
        ("check.ignore", &raw.check.ignore),
        (
            "check.scope",
            raw.check.scope.as_ref().unwrap_or(&Vec::new()),
        ),
    ] {
        for pattern in patterns {
            if let Err(error) = crate::graph::matcher(pattern) {
                return Err(invalid(format!(
                    "{field}의 '{pattern}'는 글롭으로 읽을 수 없습니다: {error}"
                )));
            }
        }
    }

    let analysis = match raw.analysis {
        None | Some(RawAnalysis { language: None, .. }) => None,
        Some(RawAnalysis {
            language: Some(language),
            workspace,
        }) => {
            if language != "typescript" {
                return Err(invalid(format!(
                    "analysis.language '{language}'는 지원하지 않습니다. typescript여야 합니다."
                )));
            }
            if let Some(workspace) = &workspace
                && workspace != "pnpm"
            {
                return Err(invalid(format!(
                    "analysis.workspace '{workspace}'는 지원하지 않습니다. pnpm이어야 합니다."
                )));
            }
            Some(Analysis {
                language,
                workspace,
            })
        }
    };

    let mut extractors = Vec::new();
    let mut owners: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    for raw in raw.extractors {
        let command = raw.command.trim().to_string();
        if command.is_empty() {
            return Err(invalid("extractors의 command가 비어 있습니다.".to_string()));
        }
        if raw.extensions.is_empty() {
            return Err(invalid(format!(
                "추출기 '{command}'의 extensions가 비어 있습니다."
            )));
        }
        for extension in &raw.extensions {
            let valid = !extension.is_empty()
                && extension
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
            if !valid {
                return Err(invalid(format!(
                    "추출기 '{command}'의 확장자 '{extension}'는 점 없는 소문자여야 합니다(예: png)."
                )));
            }
            if let Some(other) = owners.insert(extension.clone(), command.clone()) {
                let message = if other == command {
                    format!("추출기 '{command}'의 extensions에 '{extension}'가 두 번 있습니다.")
                } else {
                    format!(
                        "확장자 '{extension}'가 추출기 '{other}'와 '{command}'에 함께 있습니다."
                    )
                };
                return Err(invalid(message));
            }
        }
        extractors.push(Extractor {
            command,
            extensions: raw.extensions,
        });
    }

    Ok(Config {
        root: root.to_path_buf(),
        docs,
        channels,
        ignore: raw.check.ignore,
        scope: raw.check.scope,
        analysis,
        extractors,
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
    fn 매핑_누락에서_뺄_글롭을_읽고_틀린_글롭은_오류다() {
        let text = "version: 1\ncheck:\n  ignore:\n    - '*.md'\n";
        assert_eq!(config(text).unwrap().ignore, ["*.md"]);
        assert!(config("version: 1\n").unwrap().ignore.is_empty());
        assert!(matches!(
            config("version: 1\ncheck:\n  ignore: ['a/[b']\n"),
            Err(LoreError::InvalidConfig { .. })
        ));
    }

    #[test]
    fn 채널의_수집_경로와_검사_범위를_읽는다() {
        let text = "version: 1\nchannels:\n  - id: git\n    tool: lore-connector-git\n  - id: memo\ncheck:\n  scope: ['server/**']\n";
        let parsed = config(text).unwrap();
        assert_eq!(
            parsed.channels[0].tool.as_deref(),
            Some("lore-connector-git")
        );
        assert_eq!(parsed.channels[1].tool, None);
        assert_eq!(parsed.scope, Some(vec!["server/**".to_string()]));
        assert_eq!(config("version: 1\n").unwrap().scope, None);
        assert_eq!(
            config("version: 1\ncheck:\n  scope: []\n").unwrap().scope,
            Some(Vec::new())
        );
        assert!(matches!(
            config("version: 1\ncheck:\n  scope: ['a/[b']\n"),
            Err(LoreError::InvalidConfig { .. })
        ));
    }

    #[test]
    fn analysis는_language가_있을_때만_켜고_모르는_값은_오류다() {
        let on =
            config("version: 1\nanalysis:\n  language: typescript\n  workspace: pnpm\n").unwrap();
        assert_eq!(
            on.analysis,
            Some(Analysis {
                language: "typescript".to_string(),
                workspace: Some("pnpm".to_string()),
            })
        );
        assert_eq!(config("version: 1\n").unwrap().analysis, None);
        assert_eq!(
            config("version: 1\nanalysis:\n  workspace: pnpm\n")
                .unwrap()
                .analysis,
            None
        );
        for text in [
            "version: 1\nanalysis:\n  language: rust\n",
            "version: 1\nanalysis:\n  language: typescript\n  workspace: yarn\n",
        ] {
            assert!(
                matches!(config(text), Err(LoreError::InvalidConfig { .. })),
                "{text}"
            );
        }
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

    #[test]
    fn 추출기는_확장자마다_하나만_등록한다() {
        let parsed = config(
            "version: 1\nextractors:\n  - command: lore-extractor-image\n    extensions: [png, jpg]\n",
        )
        .unwrap();
        assert_eq!(
            parsed.extractors,
            [Extractor {
                command: "lore-extractor-image".to_string(),
                extensions: vec!["png".to_string(), "jpg".to_string()],
            }]
        );
        for text in [
            "version: 1\nextractors:\n  - command: a\n    extensions: [png]\n  - command: b\n    extensions: [png]\n",
            "version: 1\nextractors:\n  - command: a\n    extensions: ['.png']\n",
            "version: 1\nextractors:\n  - command: a\n    extensions: [PNG]\n",
            "version: 1\nextractors:\n  - command: a\n",
            "version: 1\nextractors:\n  - command: ' '\n    extensions: [png]\n",
            "version: 1\nextractors:\n  - extensions: [png]\n",
        ] {
            assert!(
                matches!(config(text), Err(LoreError::InvalidConfig { .. })),
                "{text}"
            );
        }
    }
}

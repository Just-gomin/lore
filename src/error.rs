//! 명령이 실패한 이유. JSON 출력의 `error` 객체가 된다.

use std::fmt;

use serde::Serialize;

/// 소스 파일 하나에서 찾은 문제.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Problem {
    /// 프로젝트 홈 기준 경로. `/`로 구분한다.
    pub path: String,
    /// 문제가 있는 머리말 항목. 파일 전체의 문제면 없다.
    pub field: Option<String>,
    pub code: &'static str,
    pub message: String,
}

impl Problem {
    pub fn new(
        path: impl Into<String>,
        field: Option<&str>,
        code: &'static str,
        message: impl Into<String>,
    ) -> Problem {
        Problem {
            path: path.into(),
            field: field.map(str::to_string),
            code,
            message: message.into(),
        }
    }
}

impl fmt::Display for Problem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.field {
            Some(field) => write!(f, "{} [{}] {}", self.path, field, self.message),
            None => write!(f, "{} {}", self.path, self.message),
        }
    }
}

/// 명령을 끝까지 실행하지 못한 이유.
#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum LoreError {
    /// 현재 폴더와 그 위에서 `lore.config.yaml`을 찾지 못했다.
    ConfigNotFound {
        start: String,
    },
    InvalidConfig {
        message: String,
    },
    /// 소스를 읽다가 찾은 문제. 한 번에 모아 보고한다.
    InvalidSources {
        problems: Vec<Problem>,
    },
    Io {
        path: String,
        message: String,
    },
    /// git 명령이 실패했다. git 레포가 아닐 때도 여기에 든다.
    Git {
        message: String,
    },
    /// 디비를 읽거나 쓰지 못했다.
    Database {
        message: String,
    },
    /// 명령에 넘긴 입력(JSON 등)이 형식에 맞지 않다. 문제를 모아 한 번에 보고한다.
    InvalidInput {
        problems: Vec<Problem>,
    },
    /// 제외 등급 채널의 자료는 받지 않는다.
    ExcludedChannel {
        channel: String,
    },
    /// 설정 파일도 없고 git 레포도 아니어서 프로젝트 홈을 정할 수 없다.
    NotAGitRepo {
        start: String,
    },
    /// lore 구간의 표식이 깨져 있다. 고치지 않고 사람에게 맡긴다.
    BrokenMarker {
        path: String,
    },
    /// 만들려는 기록의 ID를 이미 쓰고 있다.
    IdExists {
        id: String,
        path: String,
    },
    /// 조회 대상에 해당하는 노드가 없다.
    TargetNotFound {
        target: String,
    },
    /// 아직 지원하지 않는 형식의 조회 대상.
    UnsupportedTarget {
        target: String,
        message: String,
    },
}

impl LoreError {
    /// 프로세스 종료 코드. 사용법 오류(2)와 구별한다.
    pub fn exit_code(&self) -> u8 {
        1
    }
}

impl fmt::Display for LoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoreError::ConfigNotFound { start } => {
                write!(
                    f,
                    "{start}와 그 위 폴더에서 lore.config.yaml을 찾지 못했습니다."
                )
            }
            LoreError::InvalidConfig { message } => {
                write!(f, "lore.config.yaml을 읽지 못했습니다: {message}")
            }
            LoreError::InvalidSources { problems } => {
                write!(f, "소스에서 문제 {}개를 찾았습니다.", problems.len())?;
                for problem in problems {
                    write!(f, "\n  {problem}")?;
                }
                Ok(())
            }
            LoreError::Io { path, message } => write!(f, "{path}: {message}"),
            LoreError::Git { message } => write!(f, "git: {message}"),
            LoreError::Database { message } => write!(f, "디비: {message}"),
            LoreError::InvalidInput { problems } => {
                write!(f, "입력에서 문제 {}개를 찾았습니다.", problems.len())?;
                for problem in problems {
                    write!(f, "\n  {problem}")?;
                }
                Ok(())
            }
            LoreError::ExcludedChannel { channel } => write!(
                f,
                "채널 '{channel}'은 제외 등급이라 소스 노트로 만들지 않습니다."
            ),
            LoreError::NotAGitRepo { start } => write!(
                f,
                "{start}는 git 레포가 아니고 위 폴더에 lore.config.yaml도 없습니다."
            ),
            LoreError::BrokenMarker { path } => write!(
                f,
                "{path}의 lore 구간 표식이 깨져 있습니다. 시작과 끝 표식이 하나씩 있도록 손으로 고친 뒤 다시 실행하세요."
            ),
            LoreError::IdExists { id, path } => write!(
                f,
                "ID '{id}'를 이미 씁니다({path}). 슬러그나 날짜를 바꾸세요."
            ),
            LoreError::TargetNotFound { target } => write!(
                f,
                "'{target}'에 해당하는 기록이나 파일이 없습니다. ID나 스펙의 paths에 든 파일 경로를 넘기세요."
            ),
            LoreError::UnsupportedTarget { target, message } => write!(f, "'{target}': {message}"),
        }
    }
}

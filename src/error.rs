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
        }
    }
}

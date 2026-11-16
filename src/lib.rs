//! lore: 결정 기록을 코드 옆에 두고 조회하는 도구.
//!
//! 이 크레이트는 네트워크와 LLM 없이 동작한다. 같은 입력에는 같은 출력을 낸다.

pub mod cli;
pub mod config;
pub mod error;
pub mod frontmatter;
pub mod output;
pub mod source;

use cli::{Invocation, ParseError};
use output::Outcome;

/// 인자를 받아 명령을 실행하고, 출력할 내용과 종료 코드를 돌려준다.
///
/// 표준 출력에 직접 쓰지 않으므로 테스트에서 그대로 호출할 수 있다.
pub fn run(args: &[String]) -> Outcome {
    match cli::parse(args) {
        Ok(Invocation::Version { for_human }) => output::version(for_human),
        Ok(Invocation::Help { for_human }) => output::help(for_human),
        Ok(Invocation::Command { command, for_human }) => {
            output::not_implemented(command, for_human)
        }
        Err(ParseError::UnknownCommand { name, for_human }) => {
            output::unknown_command(&name, for_human)
        }
        Err(ParseError::InvalidArguments { message, for_human }) => {
            output::invalid_arguments(&message, for_human)
        }
    }
}

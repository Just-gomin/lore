//! lore: 결정 기록을 코드 옆에 두고 조회하는 도구.
//!
//! 이 크레이트는 네트워크와 LLM 없이 동작한다. 같은 입력에는 같은 출력을 낸다.

pub mod build;
pub mod cli;
pub mod config;
pub mod db;
pub mod error;
pub mod frontmatter;
pub mod git;
pub mod graph;
pub mod index;
pub mod output;
pub mod source;
pub mod why;

use std::path::Path;

use cli::{Invocation, ParseError, Request};
use output::Outcome;

/// 인자를 받아 명령을 실행하고, 출력할 내용과 종료 코드를 돌려준다.
///
/// 표준 출력에 직접 쓰지 않으므로 테스트에서 그대로 호출할 수 있다.
pub fn run(args: &[String]) -> Outcome {
    match std::env::current_dir() {
        Ok(cwd) => run_in(args, &cwd),
        Err(error) => output::error(
            &error::LoreError::Io {
                path: ".".to_string(),
                message: error.to_string(),
            },
            args.iter().any(|arg| arg == "--for-human"),
        ),
    }
}

/// `cwd`를 현재 폴더로 보고 명령을 실행한다.
pub fn run_in(args: &[String], cwd: &Path) -> Outcome {
    match cli::parse(args) {
        Ok(Invocation::Version { for_human }) => output::version(for_human),
        Ok(Invocation::Help { for_human }) => output::help(for_human),
        Ok(Invocation::Run { request, for_human }) => execute(request, cwd, for_human),
        Err(ParseError::UnknownCommand { name, for_human }) => {
            output::unknown_command(&name, for_human)
        }
        Err(ParseError::InvalidArguments { message, for_human }) => {
            output::invalid_arguments(&message, for_human)
        }
    }
}

fn execute(request: Request, cwd: &Path, for_human: bool) -> Outcome {
    let result = match request {
        Request::Pending(command) => return output::not_implemented(command, for_human),
        Request::Build => config::discover(cwd)
            .and_then(|config| build::build(&config))
            .map(|report| output::success(&report, for_human, build::render)),
        Request::Why { target, depth } => config::discover(cwd)
            .and_then(|config| why::why(&config, cwd, &target, depth))
            .map(|report| output::success(&report, for_human, why::render)),
    };
    result.unwrap_or_else(|error| output::error(&error, for_human))
}

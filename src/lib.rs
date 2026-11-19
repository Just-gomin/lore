//! lore: 결정 기록을 코드 옆에 두고 조회하는 도구.
//!
//! 이 크레이트는 네트워크와 LLM 없이 동작한다. 같은 입력에는 같은 출력을 낸다.

pub mod add;
pub mod add_source;
pub mod build;
pub mod check;
pub mod cli;
pub mod config;
pub mod db;
pub mod error;
pub mod frontmatter;
pub mod git;
pub mod graph;
pub mod impact;
pub mod index;
pub mod init;
pub mod install;
pub mod output;
pub mod query;
pub mod report;
pub mod source;
pub mod sync;
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

/// 입력 파일을 읽는다. 파일이 없으면 표준 입력에서 읽고 이름은 `-`이다.
fn read_input(from: Option<&str>, cwd: &Path) -> Result<(String, String), error::LoreError> {
    let io_error = |path: &str, error: std::io::Error| error::LoreError::Io {
        path: path.to_string(),
        message: error.to_string(),
    };
    match from {
        Some(path) => std::fs::read_to_string(cwd.join(path))
            .map(|text| (path.to_string(), text))
            .map_err(|error| io_error(path, error)),
        None => {
            let mut text = String::new();
            std::io::Read::read_to_string(&mut std::io::stdin(), &mut text)
                .map_err(|error| io_error("-", error))?;
            Ok(("-".to_string(), text))
        }
    }
}

/// `--origins` 파일들을 읽어 하나의 목록으로 합칩니다. 파일이 없으면 `None`이다.
fn read_origins(paths: &[String], cwd: &Path) -> Result<Option<check::Origins>, error::LoreError> {
    if paths.is_empty() {
        return Ok(None);
    }
    let mut origins = check::Origins::new();
    for path in paths {
        let (name, text) = read_input(Some(path), cwd)?;
        check::parse_origins(&name, &text, &mut origins)?;
    }
    Ok(Some(origins))
}

fn execute(request: Request, cwd: &Path, for_human: bool) -> Outcome {
    let result = match request {
        Request::Build => config::discover(cwd)
            .and_then(|config| build::build(&config))
            .map(|report| output::success(&report, for_human, build::render)),
        Request::Add(record) => config::discover(cwd)
            .and_then(|config| add::add(&config, &record))
            .map(|report| output::success(&report, for_human, add::render_report)),
        Request::Check {
            origins,
            base,
            pr_body,
            report,
        } => read_origins(&origins, cwd)
            .and_then(|origins| {
                let pr_body = pr_body
                    .map(|path| read_input(Some(&path), cwd).map(|(_, text)| text))
                    .transpose()?;
                let options = check::Options {
                    origins: origins.as_ref(),
                    base: base.as_deref(),
                    pr_body: pr_body.as_deref(),
                    report,
                };
                config::discover(cwd).and_then(|config| check::check(&config, &options))
            })
            .map(|report| output::verdict(&report, report.passed(), for_human, check::render)),
        Request::Init { scope } => config::discover(cwd)
            .and_then(|config| init::init(&config, cwd, scope.as_deref()))
            .map(|report| output::success(&report, for_human, init::render)),
        Request::Sync { base, dry_run } => config::discover(cwd)
            .and_then(|config| sync::sync(&config, &base, dry_run))
            .map(|report| output::success(&report, for_human, sync::render)),
        Request::Uninstall {
            dry_run,
            with_config,
        } => install::uninstall(cwd, dry_run, with_config)
            .map(|report| output::success(&report, for_human, install::render)),
        Request::Install {
            dry_run,
            skills,
            ci,
        } => install::install(cwd, dry_run, install::Parts { skills, ci })
            .map(|report| output::success(&report, for_human, install::render)),
        Request::AddSource { from, date } => read_input(from.as_deref(), cwd)
            .and_then(|(name, text)| add_source::parse_input(&name, &text))
            .and_then(|input| {
                config::discover(cwd)
                    .and_then(|config| add_source::add_source(&config, &input, date.as_deref()))
            })
            .map(|report| output::success(&report, for_human, add_source::render_report)),
        Request::Impact { target, hop, edges } => config::discover(cwd)
            .and_then(|config| impact::impact(&config, cwd, &target, hop, &edges))
            .map(|report| output::success(&report, for_human, impact::render)),
        Request::Why { target, depth } => config::discover(cwd)
            .and_then(|config| why::why(&config, cwd, &target, depth))
            .map(|report| output::success(&report, for_human, why::render)),
    };
    result.unwrap_or_else(|error| output::error(&error, for_human))
}

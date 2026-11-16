//! 명령 이름과 공통 옵션을 해석한다.

use std::ffi::OsString;

use clap::{Arg, ArgAction, ColorChoice};

/// lore가 제공하는 명령.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Install,
    Uninstall,
    Init,
    Add,
    Why,
    Impact,
    Build,
    Check,
    Sync,
}

impl Command {
    /// 도움말에 보여 주는 순서.
    pub const ALL: [Command; 9] = [
        Command::Install,
        Command::Uninstall,
        Command::Init,
        Command::Add,
        Command::Why,
        Command::Impact,
        Command::Build,
        Command::Check,
        Command::Sync,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Command::Install => "install",
            Command::Uninstall => "uninstall",
            Command::Init => "init",
            Command::Add => "add",
            Command::Why => "why",
            Command::Impact => "impact",
            Command::Build => "build",
            Command::Check => "check",
            Command::Sync => "sync",
        }
    }

    pub fn summary(self) -> &'static str {
        match self {
            Command::Install => "설정 파일, 문서 뼈대, 에이전트 안내문, 스킬, CI 워크플로 설치",
            Command::Uninstall => "설치한 안내문 구간, 스킬, CI 워크플로 제거",
            Command::Init => "기존 이력 확보와 첫 디비 구축",
            Command::Add => "결정, 사고, 소스 노트 추가",
            Command::Why => "코드나 기능의 배경 조회",
            Command::Impact => "변경의 영향 범위 파악",
            Command::Build => "소스를 읽어 디비로 컴파일",
            Command::Check => "신선도 검사",
            Command::Sync => "코드 변경분에 맞춰 소스 갱신",
        }
    }

    pub fn from_name(name: &str) -> Option<Command> {
        Command::ALL
            .into_iter()
            .find(|command| command.name() == name)
    }
}

/// 해석이 끝난 호출.
#[derive(Debug, PartialEq, Eq)]
pub enum Invocation {
    Version { for_human: bool },
    Help { for_human: bool },
    Run { request: Request, for_human: bool },
}

/// 실행할 명령과 그 명령의 옵션.
#[derive(Debug, PartialEq, Eq)]
pub enum Request {
    Build,
    /// 아직 구현하지 않은 명령.
    Pending(Command),
}

impl Request {
    fn from_matches(command: Command, _matches: &clap::ArgMatches) -> Request {
        match command {
            Command::Build => Request::Build,
            other => Request::Pending(other),
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum ParseError {
    UnknownCommand { name: String, for_human: bool },
    InvalidArguments { message: String, for_human: bool },
}

const FOR_HUMAN: &str = "for-human";
const HELP: &str = "help";
const VERSION: &str = "version";

/// clap 명령 트리. 하위 명령은 [`Command::ALL`]에서 만든다.
///
/// clap이 직접 만드는 도움말과 버전 출력은 끈다. 출력 형식은 `output`이 정한다.
fn definition() -> clap::Command {
    let subcommands = Command::ALL
        .into_iter()
        .map(|command| clap::Command::new(command.name()).about(command.summary()));

    clap::Command::new("lore")
        .disable_help_flag(true)
        .disable_version_flag(true)
        .disable_help_subcommand(true)
        .allow_external_subcommands(true)
        .color(ColorChoice::Never)
        .arg(
            Arg::new(FOR_HUMAN)
                .long(FOR_HUMAN)
                .global(true)
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new(HELP)
                .short('h')
                .long(HELP)
                .global(true)
                .action(ArgAction::SetTrue),
        )
        .arg(
            Arg::new(VERSION)
                .short('V')
                .long(VERSION)
                .action(ArgAction::SetTrue),
        )
        .subcommand(clap::Command::new(HELP).hide(true))
        .subcommands(subcommands)
}

/// 프로그램 이름을 뺀 인자를 해석한다.
///
/// `--for-human`은 어느 위치에 있어도 받는다. 명령별 옵션은 아직 없다.
pub fn parse(args: &[String]) -> Result<Invocation, ParseError> {
    let argv = std::iter::once("lore").chain(args.iter().map(String::as_str));
    let matches =
        definition()
            .try_get_matches_from(argv)
            .map_err(|error| ParseError::InvalidArguments {
                message: error.render().to_string().trim().to_string(),
                // 해석에 실패했으므로 원래 인자에서 직접 찾는다.
                for_human: args.iter().any(|arg| arg == "--for-human"),
            })?;

    let for_human = matches.get_flag(FOR_HUMAN);
    if matches.get_flag(VERSION) {
        return Ok(Invocation::Version { for_human });
    }
    if matches.get_flag(HELP) {
        return Ok(Invocation::Help { for_human });
    }

    match matches.subcommand() {
        None | Some((HELP, _)) => Ok(Invocation::Help { for_human }),
        Some((name, sub)) => match Command::from_name(name) {
            Some(command) => Ok(Invocation::Run {
                request: Request::from_matches(command, sub),
                for_human,
            }),
            None => {
                // 모르는 명령 뒤의 인자는 clap이 해석하지 않고 그대로 넘긴다.
                let trailing = sub.get_many::<OsString>("").into_iter().flatten();
                let for_human = for_human || trailing.into_iter().any(|arg| arg == "--for-human");
                Err(ParseError::UnknownCommand {
                    name: name.to_string(),
                    for_human,
                })
            }
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|arg| arg.to_string()).collect()
    }

    #[test]
    fn 인자가_없으면_도움말이다() {
        assert_eq!(parse(&[]), Ok(Invocation::Help { for_human: false }));
    }

    #[test]
    fn 모든_명령_이름을_알아본다() {
        for command in Command::ALL {
            let expected = match command {
                Command::Build => Request::Build,
                other => Request::Pending(other),
            };
            assert_eq!(
                parse(&args(&[command.name()])),
                Ok(Invocation::Run {
                    request: expected,
                    for_human: false
                })
            );
        }
    }

    #[test]
    fn for_human은_위치와_상관없이_받는다() {
        let expected = Ok(Invocation::Run {
            request: Request::Build,
            for_human: true,
        });
        assert_eq!(parse(&args(&["--for-human", "build"])), expected);
        assert_eq!(parse(&args(&["build", "--for-human"])), expected);
    }

    #[test]
    fn 모르는_명령은_오류다() {
        assert_eq!(
            parse(&args(&["explain"])),
            Err(ParseError::UnknownCommand {
                name: "explain".to_string(),
                for_human: false
            })
        );
    }

    #[test]
    fn 모르는_명령_뒤의_for_human도_받는다() {
        assert_eq!(
            parse(&args(&["explain", "--for-human"])),
            Err(ParseError::UnknownCommand {
                name: "explain".to_string(),
                for_human: true
            })
        );
    }

    #[test]
    fn 도움말과_버전_플래그를_알아본다() {
        for flag in ["help", "--help", "-h"] {
            assert_eq!(
                parse(&args(&[flag])),
                Ok(Invocation::Help { for_human: false })
            );
        }
        for flag in ["--version", "-V"] {
            assert_eq!(
                parse(&args(&[flag, "--for-human"])),
                Ok(Invocation::Version { for_human: true })
            );
        }
    }

    #[test]
    fn 명령이_받지_않는_인자는_오류다() {
        match parse(&args(&["build", "--nope", "--for-human"])) {
            Err(ParseError::InvalidArguments { message, for_human }) => {
                assert!(message.contains("--nope"), "{message}");
                assert!(for_human);
            }
            other => panic!("InvalidArguments가 아니다: {other:?}"),
        }
    }
}

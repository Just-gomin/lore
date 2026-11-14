//! 명령 이름과 공통 옵션을 해석한다.

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
    Command { command: Command, for_human: bool },
}

#[derive(Debug, PartialEq, Eq)]
pub enum ParseError {
    UnknownCommand { name: String, for_human: bool },
}

/// 프로그램 이름을 뺀 인자를 해석한다.
///
/// `--for-human`은 어느 위치에 있어도 받는다. 명령별 옵션은 아직 해석하지 않는다.
pub fn parse(args: &[String]) -> Result<Invocation, ParseError> {
    let for_human = args.iter().any(|arg| arg == "--for-human");
    let first = args.iter().find(|arg| arg.as_str() != "--for-human");

    match first.map(String::as_str) {
        None | Some("help" | "--help" | "-h") => Ok(Invocation::Help { for_human }),
        Some("--version" | "-V") => Ok(Invocation::Version { for_human }),
        Some(name) => match Command::from_name(name) {
            Some(command) => Ok(Invocation::Command { command, for_human }),
            None => Err(ParseError::UnknownCommand {
                name: name.to_string(),
                for_human,
            }),
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
            assert_eq!(
                parse(&args(&[command.name()])),
                Ok(Invocation::Command {
                    command,
                    for_human: false
                })
            );
        }
    }

    #[test]
    fn for_human은_위치와_상관없이_받는다() {
        let expected = Ok(Invocation::Command {
            command: Command::Why,
            for_human: true,
        });
        assert_eq!(parse(&args(&["--for-human", "why"])), expected);
        assert_eq!(parse(&args(&["why", "--for-human"])), expected);
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
}

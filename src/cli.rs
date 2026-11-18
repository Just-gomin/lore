//! 명령 이름과 공통 옵션을 해석한다.

use std::ffi::OsString;

use clap::{Arg, ArgAction, ColorChoice};

use crate::add::NewRecord;
use crate::graph::EdgeKind;
use crate::impact::TRAVERSABLE;
use crate::source::{Kind, is_iso_date, is_slug};
use crate::why::Depth;

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
    Check {
        /// 원본의 최신 판 목록 파일.
        origins: Option<String>,
        /// 바뀐 파일을 구할 비교 기준 ref.
        base: Option<String>,
        /// PR 설명을 담은 파일.
        pr_body: Option<String>,
    },
    Why {
        target: String,
        depth: Depth,
    },
    Impact {
        target: String,
        hop: u32,
        edges: Vec<EdgeKind>,
    },
    /// 기록 하나의 인자가 다른 요청보다 훨씬 커서 상자에 담는다.
    Add(Box<NewRecord>),
    Uninstall {
        dry_run: bool,
        with_config: bool,
    },
    Install {
        dry_run: bool,
        /// `--no-skills`면 거짓이다.
        skills: bool,
    },
    AddSource {
        /// 입력 JSON 파일. 없으면 표준 입력에서 읽는다.
        from: Option<String>,
        date: Option<String>,
    },
    /// 아직 구현하지 않은 명령.
    Pending(Command),
}

impl Request {
    fn from_matches(command: Command, matches: &clap::ArgMatches) -> Request {
        match command {
            Command::Build => Request::Build,
            Command::Check => Request::Check {
                origins: matches.get_one::<String>(ORIGINS).cloned(),
                base: matches.get_one::<String>(BASE).cloned(),
                pr_body: matches.get_one::<String>(PR_BODY).cloned(),
            },
            Command::Impact => Request::Impact {
                target: matches
                    .get_one::<String>(TARGET)
                    .cloned()
                    .unwrap_or_default(),
                hop: matches.get_one::<u32>(HOP).copied().unwrap_or(1),
                edges: matches
                    .get_many::<String>(EDGES)
                    .into_iter()
                    .flatten()
                    .filter_map(|name| EdgeKind::from_name(name))
                    .collect(),
            },
            Command::Why => Request::Why {
                target: matches
                    .get_one::<String>(TARGET)
                    .cloned()
                    .unwrap_or_default(),
                depth: matches
                    .get_one::<String>(DEPTH)
                    .and_then(|name| Depth::from_name(name))
                    .unwrap_or(Depth::Index),
            },
            Command::Uninstall => Request::Uninstall {
                dry_run: matches.get_flag(DRY_RUN),
                with_config: matches.get_flag(WITH_CONFIG),
            },
            Command::Install => Request::Install {
                dry_run: matches.get_flag(DRY_RUN),
                skills: !matches.get_flag(NO_SKILLS),
            },
            Command::Add => match matches.subcommand() {
                Some((SOURCE, sub)) => Request::AddSource {
                    from: sub.get_one::<String>(FROM).cloned(),
                    date: sub.get_one::<String>(DATE).cloned(),
                },
                Some((name, sub)) => Request::Add(Box::new(new_record(name, sub))),
                None => Request::Pending(Command::Add),
            },
            other => Request::Pending(other),
        }
    }
}

/// `lore add decision|incident`의 인자를 읽는다.
fn new_record(name: &str, matches: &clap::ArgMatches) -> NewRecord {
    let one = |id: &str| matches.get_one::<String>(id).cloned();
    let many = |id: &str| -> Vec<String> {
        matches
            .try_get_many::<String>(id)
            .ok()
            .flatten()
            .into_iter()
            .flatten()
            .cloned()
            .collect()
    };
    let kind = if name == DECISION {
        Kind::Decision
    } else {
        Kind::Incident
    };
    NewRecord {
        kind,
        slug: one(SLUG).unwrap_or_default(),
        title: one(TITLE).unwrap_or_default(),
        specs: many(SPECS),
        sources: many(SOURCES),
        date: one(DATE),
        supersedes: matches
            .try_get_one::<String>(SUPERSEDES)
            .ok()
            .flatten()
            .cloned(),
        status: matches
            .try_get_one::<String>(STATUS)
            .ok()
            .flatten()
            .cloned()
            .unwrap_or_else(|| "accepted".to_string()),
        confidence: matches
            .try_get_one::<String>(CONFIDENCE)
            .ok()
            .flatten()
            .cloned()
            .unwrap_or_else(|| "inferred".to_string()),
        decisions: many(DECISIONS),
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
const TARGET: &str = "target";
const DEPTH: &str = "depth";
const HOP: &str = "hop";
const EDGES: &str = "edges";
const DECISION: &str = "decision";
const INCIDENT: &str = "incident";
const SLUG: &str = "slug";
const TITLE: &str = "title";
const SPECS: &str = "specs";
const SOURCES: &str = "sources";
const DATE: &str = "date";
const SUPERSEDES: &str = "supersedes";
const STATUS: &str = "status";
const CONFIDENCE: &str = "confidence";
const DECISIONS: &str = "decisions";
const SOURCE: &str = "source";
const FROM: &str = "from";
const DRY_RUN: &str = "dry-run";
const ORIGINS: &str = "origins";
const BASE: &str = "base";
const PR_BODY: &str = "pr-body";
const NO_SKILLS: &str = "no-skills";
const WITH_CONFIG: &str = "with-config";

fn parse_slug(value: &str) -> Result<String, String> {
    if is_slug(value) {
        Ok(value.to_string())
    } else {
        Err("영문 소문자, 숫자, -로 된 슬러그여야 합니다".to_string())
    }
}

fn parse_date(value: &str) -> Result<String, String> {
    if is_iso_date(value) {
        Ok(value.to_string())
    } else {
        Err("YYYY-MM-DD 형식이어야 합니다".to_string())
    }
}

/// 쉼표로 구분한 목록 인자.
fn list_arg(id: &'static str) -> Arg {
    Arg::new(id).long(id).value_delimiter(',').num_args(1)
}

/// 결정과 사고가 함께 받는 인자.
fn record_command(name: &'static str, about: &'static str) -> clap::Command {
    clap::Command::new(name)
        .about(about)
        .arg(
            Arg::new(SLUG)
                .long(SLUG)
                .required(true)
                .value_parser(parse_slug),
        )
        .arg(Arg::new(TITLE).long(TITLE).required(true))
        .arg(list_arg(SPECS))
        .arg(list_arg(SOURCES))
        .arg(Arg::new(DATE).long(DATE).value_parser(parse_date))
}

/// 명령별 인자를 붙인다.
fn with_arguments(command: Command, definition: clap::Command) -> clap::Command {
    match command {
        Command::Why => definition
            .arg(Arg::new(TARGET).required(true).value_name("ID | 파일 경로"))
            .arg(
                Arg::new(DEPTH)
                    .long(DEPTH)
                    .value_parser(Depth::NAMES)
                    .default_value("index"),
            ),
        Command::Impact => definition
            .arg(Arg::new(TARGET).required(true).value_name("ID | 파일 경로"))
            .arg(
                Arg::new(HOP)
                    .long(HOP)
                    .value_parser(clap::value_parser!(u32))
                    .default_value("1"),
            )
            .arg(
                Arg::new(EDGES)
                    .long(EDGES)
                    .value_delimiter(',')
                    .value_parser(TRAVERSABLE.map(EdgeKind::name))
                    .default_values(TRAVERSABLE.map(EdgeKind::name)),
            ),
        Command::Check => definition
            .arg(Arg::new(ORIGINS).long(ORIGINS).value_name("JSON 파일"))
            .arg(Arg::new(BASE).long(BASE).value_name("ref"))
            .arg(Arg::new(PR_BODY).long(PR_BODY).value_name("파일")),
        Command::Uninstall => definition
            .arg(Arg::new(DRY_RUN).long(DRY_RUN).action(ArgAction::SetTrue))
            .arg(
                Arg::new(WITH_CONFIG)
                    .long(WITH_CONFIG)
                    .action(ArgAction::SetTrue),
            ),
        Command::Install => definition
            .arg(Arg::new(DRY_RUN).long(DRY_RUN).action(ArgAction::SetTrue))
            .arg(
                Arg::new(NO_SKILLS)
                    .long(NO_SKILLS)
                    .action(ArgAction::SetTrue),
            ),
        Command::Add => definition.subcommand_required(true).subcommands([
            record_command(DECISION, "결정 기록 추가")
                .arg(Arg::new(SUPERSEDES).long(SUPERSEDES))
                .arg(
                    Arg::new(STATUS)
                        .long(STATUS)
                        .value_parser(["proposed", "accepted"])
                        .default_value("accepted"),
                )
                .arg(
                    Arg::new(CONFIDENCE)
                        .long(CONFIDENCE)
                        .value_parser(["stated", "inferred"])
                        .default_value("inferred"),
                ),
            record_command(INCIDENT, "사고 기록 추가").arg(list_arg(DECISIONS)),
            clap::Command::new(SOURCE)
                .about("소스 노트 추가")
                .arg(Arg::new(FROM).long(FROM).value_name("JSON 파일"))
                .arg(Arg::new(DATE).long(DATE).value_parser(parse_date)),
        ]),
        _ => definition,
    }
}

/// clap 명령 트리. 하위 명령은 [`Command::ALL`]에서 만든다.
///
/// clap이 직접 만드는 도움말과 버전 출력은 끈다. 출력 형식은 `output`이 정한다.
fn definition() -> clap::Command {
    let subcommands = Command::ALL.into_iter().map(|command| {
        with_arguments(
            command,
            clap::Command::new(command.name()).about(command.summary()),
        )
    });

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
/// `--for-human`은 어느 위치에 있어도 받는다.
pub fn parse(args: &[String]) -> Result<Invocation, ParseError> {
    // 하위 명령 뒤의 도움말 플래그는 필수 인자 검사보다 먼저 본다.
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        return Ok(Invocation::Help {
            for_human: args.iter().any(|arg| arg == "--for-human"),
        });
    }
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
            let (argv, expected) = match command {
                Command::Build => (vec!["build"], Request::Build),
                Command::Check => (
                    vec!["check"],
                    Request::Check {
                        origins: None,
                        base: None,
                        pr_body: None,
                    },
                ),
                Command::Why => (
                    vec!["why", "payment-retry"],
                    Request::Why {
                        target: "payment-retry".to_string(),
                        depth: Depth::Index,
                    },
                ),
                Command::Impact => (
                    vec!["impact", "common"],
                    Request::Impact {
                        target: "common".to_string(),
                        hop: 1,
                        edges: TRAVERSABLE.to_vec(),
                    },
                ),
                Command::Add => (
                    vec!["add", "incident", "--slug", "dup", "--title", "중복"],
                    Request::Add(Box::new(NewRecord {
                        kind: Kind::Incident,
                        slug: "dup".to_string(),
                        title: "중복".to_string(),
                        specs: Vec::new(),
                        sources: Vec::new(),
                        date: None,
                        supersedes: None,
                        status: "accepted".to_string(),
                        confidence: "inferred".to_string(),
                        decisions: Vec::new(),
                    })),
                ),
                Command::Uninstall => (
                    vec!["uninstall", "--with-config"],
                    Request::Uninstall {
                        dry_run: false,
                        with_config: true,
                    },
                ),
                Command::Install => (
                    vec!["install"],
                    Request::Install {
                        dry_run: false,
                        skills: true,
                    },
                ),
                other => (vec![other.name()], Request::Pending(other)),
            };
            assert_eq!(
                parse(&args(&argv)),
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
    fn 하위_명령_뒤의_도움말은_필수_인자보다_먼저_본다() {
        assert_eq!(
            parse(&args(&["add", "decision", "--help"])),
            Ok(Invocation::Help { for_human: false })
        );
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
    fn why는_대상과_깊이를_받는다() {
        assert_eq!(
            parse(&args(&["why", "a.ts", "--depth", "source"])),
            Ok(Invocation::Run {
                request: Request::Why {
                    target: "a.ts".to_string(),
                    depth: Depth::Source,
                },
                for_human: false
            })
        );
        assert!(matches!(
            parse(&args(&["why"])),
            Err(ParseError::InvalidArguments { .. })
        ));
        assert!(matches!(
            parse(&args(&["why", "a.ts", "--depth=all"])),
            Err(ParseError::InvalidArguments { .. })
        ));
    }

    #[test]
    fn impact는_홉과_건널_엣지를_받는다() {
        assert_eq!(
            parse(&args(&[
                "impact",
                "common",
                "--hop=2",
                "--edges=depends_on"
            ])),
            Ok(Invocation::Run {
                request: Request::Impact {
                    target: "common".to_string(),
                    hop: 2,
                    edges: vec![EdgeKind::DependsOn],
                },
                for_human: false
            })
        );
        for bad in ["--hop=-1", "--edges=implements"] {
            assert!(matches!(
                parse(&args(&["impact", "common", bad])),
                Err(ParseError::InvalidArguments { .. })
            ));
        }
    }

    #[test]
    fn add_decision은_목록을_쉼표로_받는다() {
        let parsed = parse(&args(&[
            "add",
            "decision",
            "--slug",
            "retry-5",
            "--title",
            "재시도 5회",
            "--specs",
            "payment-retry,payment-core",
            "--supersedes",
            "ADR-20261105-retry-queue",
            "--status",
            "proposed",
            "--date",
            "2026-11-12",
        ]));
        let Ok(Invocation::Run {
            request: Request::Add(record),
            ..
        }) = parsed
        else {
            panic!("add가 아니다: {parsed:?}");
        };
        assert_eq!(record.kind, Kind::Decision);
        assert_eq!(record.specs, ["payment-retry", "payment-core"]);
        assert_eq!(
            record.supersedes.as_deref(),
            Some("ADR-20261105-retry-queue")
        );
        assert_eq!(record.status, "proposed");
        assert_eq!(record.confidence, "inferred");
        assert_eq!(record.date.as_deref(), Some("2026-11-12"));
    }

    #[test]
    fn add_source는_입력_파일과_날짜를_받는다() {
        assert_eq!(
            parse(&args(&[
                "add",
                "source",
                "--from",
                "in.json",
                "--date",
                "2026-11-04"
            ])),
            Ok(Invocation::Run {
                request: Request::AddSource {
                    from: Some("in.json".to_string()),
                    date: Some("2026-11-04".to_string()),
                },
                for_human: false
            })
        );
        assert_eq!(
            parse(&args(&["add", "source"])),
            Ok(Invocation::Run {
                request: Request::AddSource {
                    from: None,
                    date: None
                },
                for_human: false
            })
        );
    }

    #[test]
    fn add의_잘못된_인자는_사용법_오류다() {
        for bad in [
            vec!["add"],
            vec!["add", "decision", "--title", "제목"],
            vec!["add", "decision", "--slug", "Retry", "--title", "제목"],
            vec![
                "add", "decision", "--slug", "a", "--title", "제목", "--date", "2026-1-2",
            ],
            vec![
                "add",
                "incident",
                "--slug",
                "a",
                "--title",
                "제목",
                "--supersedes",
                "X",
            ],
        ] {
            assert!(
                matches!(parse(&args(&bad)), Err(ParseError::InvalidArguments { .. })),
                "{bad:?}"
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

//! `lore-connector-git`: git 이력에서 소스 노트 후보를 뽑는 커넥터.
//!
//! 현재 커밋에서 첫 부모를 따라간 커밋 하나가 후보 하나입니다. 레포는 고치지 않습니다.
//! 출력 형식은 기획서 6장의 커넥터 규약을 따릅니다.

use std::path::Path;
use std::process::{Command, ExitCode};

use clap::{Arg, ArgAction};
use globset::{GlobBuilder, GlobMatcher};
use lore_connector_common::{
    Candidate, ConnectorError, OriginVersion, candidates_json, error_json, origins_json,
    read_source_origins,
};

const CHANNEL: &str = "channel";
const SINCE: &str = "since";
const PATHS: &str = "paths";
const LIMIT: &str = "limit";
const REMOTE: &str = "remote";
const EXCLUDE_PR_COMMITS: &str = "exclude-pr-commits";
const ORIGINS: &str = "origins";

fn main() -> ExitCode {
    let command = clap::Command::new("lore-connector-git")
        .about("git 이력에서 lore 소스 노트 후보를 뽑습니다")
        .version(env!("CARGO_PKG_VERSION"))
        .arg(Arg::new(CHANNEL).long(CHANNEL).value_name("채널 ID"))
        .arg(Arg::new(SINCE).long(SINCE).value_name("YYYY-MM-DD"))
        .arg(
            Arg::new(PATHS)
                .long(PATHS)
                .value_name("글롭,...")
                .value_delimiter(','),
        )
        .arg(
            Arg::new(LIMIT)
                .long(LIMIT)
                .value_name("수")
                .value_parser(clap::value_parser!(usize)),
        )
        .arg(Arg::new(REMOTE).long(REMOTE).default_value("origin"))
        .arg(
            Arg::new(EXCLUDE_PR_COMMITS)
                .long(EXCLUDE_PR_COMMITS)
                .action(ArgAction::SetTrue),
        )
        .arg(Arg::new(ORIGINS).long(ORIGINS).value_name("소스 폴더"));
    let matches = match command.try_get_matches() {
        Ok(matches) => matches,
        Err(error) if !error.use_stderr() => {
            // --help, --version
            print!("{error}");
            return ExitCode::SUCCESS;
        }
        Err(error) => return fail(&ConnectorError::new("usage", error.to_string()), 2),
    };

    let result = match matches.get_one::<String>(ORIGINS) {
        Some(docs) => origins(Path::new(docs)).map(|list| origins_json(&list)),
        None => {
            let Some(channel) = matches.get_one::<String>(CHANNEL) else {
                return fail(
                    &ConnectorError::new("usage", "--channel <채널 ID>가 필요합니다."),
                    2,
                );
            };
            let options = Options {
                channel: channel.clone(),
                since: matches.get_one::<String>(SINCE).cloned(),
                paths: matches
                    .get_many::<String>(PATHS)
                    .into_iter()
                    .flatten()
                    .cloned()
                    .collect(),
                limit: matches.get_one::<usize>(LIMIT).copied(),
                remote: matches
                    .get_one::<String>(REMOTE)
                    .cloned()
                    .unwrap_or_default(),
                exclude_pr_commits: matches.get_flag(EXCLUDE_PR_COMMITS),
            };
            candidates(&options).map(|list| candidates_json(&list))
        }
    };
    match result {
        Ok(json) => {
            println!("{json}");
            ExitCode::SUCCESS
        }
        Err(error) => fail(&error, 1),
    }
}

fn fail(error: &ConnectorError, code: u8) -> ExitCode {
    println!("{}", error_json(error));
    ExitCode::from(code)
}

struct Options {
    channel: String,
    since: Option<String>,
    paths: Vec<String>,
    limit: Option<usize>,
    remote: String,
    exclude_pr_commits: bool,
}

fn git(args: &[&str]) -> Result<String, ConnectorError> {
    let output = Command::new("git").args(args).output().map_err(|error| {
        ConnectorError::new("git", format!("git을 실행하지 못했습니다: {error}"))
    })?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(ConnectorError::new(
            "git",
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ))
    }
}

/// 첫 부모를 따라간 커밋을 최신순으로 후보로 만듭니다.
fn candidates(options: &Options) -> Result<Vec<Candidate>, ConnectorError> {
    let globs = options
        .paths
        .iter()
        .map(|pattern| matcher(pattern))
        .collect::<Result<Vec<_>, _>>()?;
    let web = remote_web_url(&options.remote);

    let mut args = vec!["rev-list", "--first-parent"];
    let since;
    if let Some(date) = &options.since {
        since = format!("--since={date}");
        args.push(&since);
    }
    args.push("HEAD");
    let mut candidates = Vec::new();
    for sha in git(&args)?.lines() {
        if options.limit.is_some_and(|limit| candidates.len() >= limit) {
            break;
        }
        let commit = read_commit(sha)?;
        if options.exclude_pr_commits && is_pr_commit(&commit.subject) {
            continue;
        }
        if !globs.is_empty()
            && !commit
                .files
                .iter()
                .any(|file| globs.iter().any(|glob| glob.is_match(file)))
        {
            continue;
        }
        let mut text = commit.message.trim_end().to_string();
        if !commit.files.is_empty() {
            text.push_str("\n\n바뀐 파일:");
            for file in &commit.files {
                text.push_str(&format!("\n- {file}"));
            }
        }
        candidates.push(Candidate {
            title: commit.subject,
            kind: "evidence".to_string(),
            channel: options.channel.clone(),
            origin: match &web {
                Some(base) => format!("{base}/commit/{sha}"),
                None => format!("git:{sha}"),
            },
            origin_version: sha.to_string(),
            occurred_at: commit.date,
            text,
        });
    }
    Ok(candidates)
}

struct Commit {
    subject: String,
    message: String,
    /// 커미터 날짜. 실행 환경의 로컬 시간대로 본 `YYYY-MM-DD`
    date: String,
    files: Vec<String>,
}

fn read_commit(sha: &str) -> Result<Commit, ConnectorError> {
    let header = git(&[
        "show",
        "-s",
        "--date=format-local:%Y-%m-%d",
        "--format=%cd%x00%s%x00%B",
        sha,
    ])?;
    let mut parts = header.splitn(3, '\0');
    let date = parts.next().unwrap_or_default().to_string();
    let subject = parts.next().unwrap_or_default().to_string();
    let message = parts.next().unwrap_or_default().to_string();
    // 첫 커밋은 --root로, 병합 커밋은 첫 부모와 견줘 바뀐 파일을 구합니다.
    let files = git(&[
        "diff-tree",
        "--no-commit-id",
        "-r",
        "--root",
        "-m",
        "--first-parent",
        "--name-only",
        sha,
    ])?
    .lines()
    .map(str::to_string)
    .collect();
    Ok(Commit {
        subject,
        message,
        date,
        files,
    })
}

/// 제목이 `(#123)`으로 끝나는 커밋. squash merge한 PR 커밋입니다.
fn is_pr_commit(subject: &str) -> bool {
    let Some(rest) = subject.trim_end().strip_suffix(')') else {
        return false;
    };
    let Some(start) = rest.rfind("(#") else {
        return false;
    };
    let number = &rest[start + 2..];
    !number.is_empty() && number.chars().all(|ch| ch.is_ascii_digit())
}

/// 원격이 GitHub이면 `https://github.com/<owner>/<repo>`. 원격이 없거나 GitHub가 아니면 `None`입니다.
fn remote_web_url(remote: &str) -> Option<String> {
    let url = git(&["remote", "get-url", remote]).ok()?;
    github_web_url(url.trim())
}

fn github_web_url(url: &str) -> Option<String> {
    let path = url
        .strip_prefix("https://github.com/")
        .or_else(|| url.strip_prefix("http://github.com/"))
        .or_else(|| url.strip_prefix("git@github.com:"))
        .or_else(|| url.strip_prefix("ssh://git@github.com/"))?;
    let path = path.trim_end_matches('/').trim_end_matches(".git");
    let mut parts = path.split('/');
    let (owner, repo) = (parts.next()?, parts.next()?);
    (parts.next().is_none() && !owner.is_empty() && !repo.is_empty())
        .then(|| format!("https://github.com/{owner}/{repo}"))
}

/// 이 커넥터가 맡는 원본(`git:`와 GitHub 커밋 주소)의 판. 커밋은 바뀌지 않으므로 적힌 판 그대로입니다.
fn origins(docs: &Path) -> Result<Vec<OriginVersion>, ConnectorError> {
    Ok(read_source_origins(docs)?
        .into_iter()
        .filter(|entry| is_commit_origin(&entry.origin))
        .collect())
}

fn is_commit_origin(origin: &str) -> bool {
    if origin.starts_with("git:") {
        return true;
    }
    let Some(path) = origin.strip_prefix("https://github.com/") else {
        return false;
    };
    let parts: Vec<&str> = path.split('/').collect();
    parts.len() == 4 && parts[2] == "commit" && !parts[3].is_empty()
}

/// 스펙의 `paths`와 같은 글롭 규칙입니다. `*`는 폴더 경계를 넘지 않습니다.
fn matcher(pattern: &str) -> Result<GlobMatcher, ConnectorError> {
    GlobBuilder::new(pattern.trim_start_matches("./"))
        .literal_separator(true)
        .build()
        .map(|glob| glob.compile_matcher())
        .map_err(|error| {
            ConnectorError::new(
                "usage",
                format!("'{pattern}'는 글롭으로 읽을 수 없습니다: {error}"),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 제목이_번호_괄호로_끝나는_커밋은_pr_커밋이다() {
        assert!(is_pr_commit("feat: 태그 필터 (#27)"));
        assert!(is_pr_commit("fix: x (#3) "));
        assert!(!is_pr_commit("feat: 태그 필터"));
        assert!(!is_pr_commit("feat: (#) "));
        assert!(!is_pr_commit("feat: (#2a)"));
    }

    #[test]
    fn github_원격_주소를_웹_주소로_바꾼다() {
        for url in [
            "https://github.com/Just-gomin/lore.git",
            "https://github.com/Just-gomin/lore",
            "git@github.com:Just-gomin/lore.git",
            "ssh://git@github.com/Just-gomin/lore.git",
        ] {
            assert_eq!(
                github_web_url(url).as_deref(),
                Some("https://github.com/Just-gomin/lore"),
                "{url}"
            );
        }
        assert_eq!(github_web_url("https://gitlab.com/a/b.git"), None);
        assert_eq!(github_web_url("https://github.com/a"), None);
    }

    #[test]
    fn 커밋_원본만_맡는다() {
        assert!(is_commit_origin("git:abc"));
        assert!(is_commit_origin("https://github.com/o/r/commit/abc"));
        assert!(!is_commit_origin("https://github.com/o/r/pull/3"));
        assert!(!is_commit_origin("https://drive.google.com/file/d/x"));
    }
}

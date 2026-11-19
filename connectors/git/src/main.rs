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
                .value_name("글롭")
                .action(ArgAction::Append),
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
                remote_given: matches.value_source(REMOTE)
                    == Some(clap::parser::ValueSource::CommandLine),
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
        Err(error) => fail(&error, if error.code == "usage" { 2 } else { 1 }),
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
    /// `--remote`를 직접 줬는지. 기본 원격은 없어도 되지만, 고른 원격이 없으면 오류입니다.
    remote_given: bool,
    exclude_pr_commits: bool,
}

/// 사용자 설정이 출력 모양을 바꾸지 않게 고정해 git을 부릅니다.
fn git(args: &[&str]) -> Result<String, ConnectorError> {
    let output = Command::new("git")
        .args([
            "-c",
            "core.quotepath=false",
            "-c",
            "log.showSignature=false",
        ])
        .args(args)
        .output()
        .map_err(|error| {
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
///
/// `git log` 한 번으로 이력을 읽습니다. 병합 커밋은 첫 부모와 견준 파일만 봅니다.
fn candidates(options: &Options) -> Result<Vec<Candidate>, ConnectorError> {
    let globs = options
        .paths
        .iter()
        .map(|pattern| matcher(pattern))
        .collect::<Result<Vec<_>, _>>()?;
    let since = options.since.as_deref().map(since_timestamp).transpose()?;
    let web = remote_web_url(&options.remote, options.remote_given)?;

    git(&["rev-parse", "--git-dir"])?;
    // 커밋이 하나도 없는 레포는 후보가 없습니다.
    if git(&["rev-parse", "--verify", "--quiet", "HEAD"]).is_err() {
        return Ok(Vec::new());
    }
    let log = git(&[
        "log",
        "--first-parent",
        "--diff-merges=first-parent",
        "--name-only",
        "-z",
        "--date=format-local:%Y-%m-%d",
        "--format=%x1e%H%x1f%ct%x1f%cd%x1f%s%x1f%B%x1f",
        "HEAD",
    ])?;

    let mut candidates = Vec::new();
    for commit in log.split('\x1e').filter_map(parse_commit) {
        if options.limit.is_some_and(|limit| candidates.len() >= limit) {
            break;
        }
        // 커미터 시각이 앞뒤로 엇갈려도 이력을 끝까지 보도록 git에 맡기지 않고 여기서 거릅니다.
        if since.is_some_and(|since| commit.timestamp < since) {
            continue;
        }
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
        let title = if commit.subject.is_empty() {
            format!("커밋 {}", &commit.sha[..commit.sha.len().min(12)])
        } else {
            commit.subject
        };
        candidates.push(Candidate {
            title,
            kind: "evidence".to_string(),
            channel: options.channel.clone(),
            origin: match &web {
                Some(base) => format!("{base}/commit/{}", commit.sha),
                None => format!("git:{}", commit.sha),
            },
            origin_version: commit.sha,
            occurred_at: commit.date,
            text: text.trim_start().to_string(),
        });
    }
    Ok(candidates)
}

struct Commit {
    sha: String,
    /// 커미터 시각(유닉스 초)
    timestamp: i64,
    /// 커미터 날짜. 실행 환경의 로컬 시간대로 본 `YYYY-MM-DD`
    date: String,
    subject: String,
    message: String,
    files: Vec<String>,
}

/// `git log` 출력의 커밋 하나. 칸은 0x1f로, 바뀐 파일은 NUL로 나뉩니다.
fn parse_commit(record: &str) -> Option<Commit> {
    let mut fields = record.splitn(6, '\x1f');
    let sha = fields.next()?.trim().to_string();
    if sha.is_empty() {
        return None;
    }
    let timestamp = fields.next()?.trim().parse().ok()?;
    let date = fields.next()?.to_string();
    let subject = fields.next()?.to_string();
    let message = fields.next()?.to_string();
    let mut files: Vec<String> = fields
        .next()
        .unwrap_or_default()
        .split('\0')
        .map(|file| file.trim_start_matches('\n'))
        .filter(|file| !file.is_empty())
        .map(str::to_string)
        .collect();
    files.dedup();
    Some(Commit {
        sha,
        timestamp,
        date,
        subject,
        message,
        files,
    })
}

/// `YYYY-MM-DD`의 로컬 자정. git이 실행 환경의 시간대로 계산합니다.
fn since_timestamp(date: &str) -> Result<i64, ConnectorError> {
    let invalid = || {
        ConnectorError::new(
            "usage",
            format!("--since '{date}'는 YYYY-MM-DD 날짜가 아닙니다."),
        )
    };
    let parts: Vec<&str> = date.split('-').collect();
    let valid = parts.len() == 3
        && [4, 2, 2]
            .iter()
            .zip(&parts)
            .all(|(len, part)| part.len() == *len && part.chars().all(|ch| ch.is_ascii_digit()));
    let month: u32 = parts.get(1).and_then(|m| m.parse().ok()).unwrap_or(0);
    let day: u32 = parts.get(2).and_then(|d| d.parse().ok()).unwrap_or(0);
    if !valid || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return Err(invalid());
    }
    let output = git(&["rev-parse", &format!("--since={date} 00:00:00")])?;
    output
        .trim()
        .strip_prefix("--max-age=")
        .and_then(|value| value.parse().ok())
        .ok_or_else(invalid)
}

/// 제목이 `(#123)`으로 끝나는 squash 커밋과 `Merge pull request #123`으로 시작하는 병합 커밋.
fn is_pr_commit(subject: &str) -> bool {
    if let Some(rest) = subject.strip_prefix("Merge pull request #") {
        return rest.chars().next().is_some_and(|ch| ch.is_ascii_digit());
    }
    let Some(rest) = subject.trim_end().strip_suffix(')') else {
        return false;
    };
    let Some(start) = rest.rfind("(#") else {
        return false;
    };
    let number = &rest[start + 2..];
    !number.is_empty() && number.chars().all(|ch| ch.is_ascii_digit())
}

/// 원격이 GitHub이면 `https://github.com/<owner>/<repo>`, GitHub가 아니거나 기본 원격이 없으면 `None`입니다.
/// `--remote`로 직접 고른 원격이 없으면 오류입니다.
fn remote_web_url(remote: &str, given: bool) -> Result<Option<String>, ConnectorError> {
    match git(&["remote", "get-url", remote]) {
        Ok(url) => Ok(github_web_url(url.trim())),
        Err(_) if !given => Ok(None),
        Err(_) => Err(ConnectorError::new(
            "usage",
            format!("원격 '{remote}'가 없습니다."),
        )),
    }
}

fn github_web_url(url: &str) -> Option<String> {
    // https://user:token@github.com/... 처럼 인증 정보가 든 주소도 읽습니다.
    let url = match url.split_once("://") {
        Some((scheme, rest)) => {
            let host_end = rest.find('/').unwrap_or(rest.len());
            match rest[..host_end].rfind('@') {
                Some(at) => format!("{scheme}://{}", &rest[at + 1..]),
                None => url.to_string(),
            }
        }
        None => url.to_string(),
    };
    let path = url
        .strip_prefix("https://github.com/")
        .or_else(|| url.strip_prefix("http://github.com/"))
        .or_else(|| url.strip_prefix("git@github.com:"))
        .or_else(|| url.strip_prefix("ssh://github.com/"))?;
    let path = path.trim_end_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
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
        assert!(is_pr_commit("Merge pull request #7 from a/b"));
        assert!(!is_pr_commit("Merge branch 'main'"));
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
        assert_eq!(
            github_web_url("https://user:token@github.com/o/r.git").as_deref(),
            Some("https://github.com/o/r")
        );
        assert_eq!(
            github_web_url("https://github.com/o/x.git.git").as_deref(),
            Some("https://github.com/o/x.git")
        );
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

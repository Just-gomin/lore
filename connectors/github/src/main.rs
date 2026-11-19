//! `lore-connector-github`: GitHub PR과 이슈에서 소스 노트 후보를 뽑는 커넥터.
//!
//! 병합된 PR과 닫힌 이슈 하나가 후보 하나입니다. GitHub 호출은 `gh api`가 맡고 인증도 `gh`에 맡깁니다.
//! 출력 형식은 기획서 6장의 커넥터 규약을 따릅니다.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use chrono::{DateTime, Local, NaiveDate, TimeZone};
use clap::{Arg, ArgAction};
use lore_connector_common::{
    Candidate, ConnectorError, OriginVersion, candidates_json, error_json, github_web_url,
    origins_json, read_source_origins,
};
use serde_json::Value;
use sha2::{Digest, Sha256};

const CHANNEL: &str = "channel";
const REPO: &str = "repo";
const SINCE: &str = "since";
const LIMIT: &str = "limit";
const INCLUDE: &str = "include";
const FIXTURE: &str = "fixture";
const ORIGINS: &str = "origins";

fn main() -> ExitCode {
    let command = clap::Command::new("lore-connector-github")
        .about("GitHub PR과 이슈에서 lore 소스 노트 후보를 뽑습니다")
        .version(env!("CARGO_PKG_VERSION"))
        .arg(Arg::new(CHANNEL).long(CHANNEL).value_name("채널 ID"))
        .arg(Arg::new(REPO).long(REPO).value_name("owner/name"))
        .arg(Arg::new(SINCE).long(SINCE).value_name("YYYY-MM-DD"))
        .arg(
            Arg::new(LIMIT)
                .long(LIMIT)
                .value_name("수")
                .value_parser(clap::value_parser!(usize)),
        )
        .arg(
            Arg::new(INCLUDE)
                .long(INCLUDE)
                .value_parser(["pulls", "issues"])
                .action(ArgAction::Append),
        )
        .arg(Arg::new(FIXTURE).long(FIXTURE).value_name("폴더"))
        .arg(Arg::new(ORIGINS).long(ORIGINS).value_name("소스 폴더"));
    let matches = match command.try_get_matches() {
        Ok(matches) => matches,
        Err(error) if !error.use_stderr() => {
            print!("{error}");
            return ExitCode::SUCCESS;
        }
        Err(error) => return fail(&ConnectorError::new("usage", error.to_string())),
    };

    let api = match matches.get_one::<String>(FIXTURE) {
        Some(dir) => Api::Fixture(PathBuf::from(dir)),
        None => Api::Gh,
    };
    let result = match matches.get_one::<String>(ORIGINS) {
        Some(docs) => origins(&api, Path::new(docs)).map(|list| origins_json(&list)),
        None => options(&matches)
            .and_then(|options| candidates(&api, &options))
            .map(|list| candidates_json(&list)),
    };
    match result {
        Ok(json) => {
            println!("{json}");
            ExitCode::SUCCESS
        }
        Err(error) => fail(&error),
    }
}

fn fail(error: &ConnectorError) -> ExitCode {
    println!("{}", error_json(error));
    ExitCode::from(if error.code == "usage" { 2 } else { 1 })
}

struct Options {
    channel: String,
    /// `owner/name`
    repo: String,
    /// `--since`의 로컬 자정
    since: Option<DateTime<Local>>,
    limit: Option<usize>,
    pulls: bool,
    issues: bool,
}

fn options(matches: &clap::ArgMatches) -> Result<Options, ConnectorError> {
    let usage = |message: String| ConnectorError::new("usage", message);
    let channel = matches
        .get_one::<String>(CHANNEL)
        .ok_or_else(|| usage("--channel <채널 ID>가 필요합니다.".to_string()))?;
    let repo = match matches.get_one::<String>(REPO) {
        Some(repo) => repo.clone(),
        None => repo_from_remote()?,
    };
    let parts: Vec<&str> = repo.split('/').collect();
    if parts.len() != 2 || parts.iter().any(|part| part.is_empty()) {
        return Err(usage(format!(
            "--repo '{repo}'는 owner/name 꼴이어야 합니다."
        )));
    }
    let since = match matches.get_one::<String>(SINCE) {
        Some(date) => Some(
            local_midnight(date)
                .ok_or_else(|| usage(format!("--since '{date}'는 YYYY-MM-DD 날짜가 아닙니다.")))?,
        ),
        None => None,
    };
    let include: Vec<String> = matches
        .get_many::<String>(INCLUDE)
        .map(|values| values.cloned().collect())
        .unwrap_or_else(|| vec!["pulls".to_string(), "issues".to_string()]);
    Ok(Options {
        channel: channel.clone(),
        repo,
        since,
        limit: matches.get_one::<usize>(LIMIT).copied(),
        pulls: include.iter().any(|kind| kind == "pulls"),
        issues: include.iter().any(|kind| kind == "issues"),
    })
}

/// GitHub API를 읽는 곳. 시험에서는 `gh` 대신 고정 응답 파일을 읽습니다.
enum Api {
    Gh,
    /// 엔드포인트의 쿼리를 뺀 경로에서 `/`를 `_`로 바꾼 이름의 `.json` 파일을 읽습니다.
    /// 파일이 없으면 GitHub의 404와 같게 봅니다.
    Fixture(PathBuf),
}

impl Api {
    /// 한 건을 돌려주는 엔드포인트. 없거나 지워졌으면(404, 410) `None`입니다.
    fn get(&self, endpoint: &str) -> Result<Option<Value>, ConnectorError> {
        match self {
            Api::Gh => match gh(&["api", endpoint]) {
                Ok(text) => parse(&text).map(Some),
                Err(error) if is_gone(&error) => Ok(None),
                Err(error) => Err(error),
            },
            Api::Fixture(dir) => self.fixture(dir, endpoint),
        }
    }

    /// 목록을 돌려주는 엔드포인트. 모든 쪽을 읽어 하나로 합칩니다.
    fn list(&self, endpoint: &str) -> Result<Vec<Value>, ConnectorError> {
        let value = match self {
            Api::Gh => {
                let pages = parse(&gh(&["api", "--paginate", "--slurp", endpoint])?)?;
                Value::Array(
                    pages
                        .as_array()
                        .into_iter()
                        .flatten()
                        .flat_map(|page| page.as_array().cloned().unwrap_or_default())
                        .collect(),
                )
            }
            Api::Fixture(dir) => self.fixture(dir, endpoint)?.unwrap_or_default(),
        };
        Ok(value.as_array().cloned().unwrap_or_default())
    }

    fn fixture(&self, dir: &Path, endpoint: &str) -> Result<Option<Value>, ConnectorError> {
        let path = endpoint.split('?').next().unwrap_or(endpoint);
        let file = dir.join(format!("{}.json", path.replace('/', "_")));
        match std::fs::read_to_string(&file) {
            Ok(text) => parse(&text).map(Some),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(ConnectorError::new(
                "github",
                format!("{}: {error}", file.display()),
            )),
        }
    }
}

fn gh(args: &[&str]) -> Result<String, ConnectorError> {
    let output = Command::new("gh").args(args).output().map_err(|error| {
        ConnectorError::new("github", format!("gh를 실행하지 못했습니다: {error}"))
    })?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(ConnectorError::new(
            "github",
            String::from_utf8_lossy(&output.stderr).trim().to_string(),
        ))
    }
}

/// 원본이 없거나 지워졌다는 응답. 인증, 권한, 호출 한도, 네트워크 오류는 아닙니다.
fn is_gone(error: &ConnectorError) -> bool {
    error.message.contains("(HTTP 404)") || error.message.contains("(HTTP 410)")
}

fn parse(text: &str) -> Result<Value, ConnectorError> {
    serde_json::from_str(text)
        .map_err(|error| ConnectorError::new("github", format!("응답을 읽지 못했습니다: {error}")))
}

fn repo_from_remote() -> Result<String, ConnectorError> {
    let url = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string());
    url.as_deref()
        .and_then(github_web_url)
        .and_then(|web| web.strip_prefix("https://github.com/").map(str::to_string))
        .ok_or_else(|| {
            ConnectorError::new(
                "usage",
                "origin 원격에서 GitHub 레포를 찾지 못했습니다. --repo <owner/name>을 주세요.",
            )
        })
}

/// 후보로 만들 PR이나 이슈 하나.
struct Item {
    pull: bool,
    number: u64,
    title: String,
    url: String,
    /// 병합하거나 닫은 시각(RFC 3339). 아직이면 연 시각입니다.
    done_at: String,
    body: String,
}

/// 이슈 API의 항목 하나. PR도 이슈 API에 나오며 `pull_request`가 붙습니다.
fn item(value: &Value) -> Option<Item> {
    let pull = value.get("pull_request").is_some_and(|p| !p.is_null());
    let done_at = if pull {
        value["pull_request"]["merged_at"].as_str()
    } else {
        value["closed_at"].as_str()
    }
    .or_else(|| value["created_at"].as_str())?;
    Some(Item {
        pull,
        number: value["number"].as_u64()?,
        title: value["title"].as_str()?.to_string(),
        url: value["html_url"].as_str()?.to_string(),
        done_at: done_at.to_string(),
        body: value["body"].as_str().unwrap_or_default().to_string(),
    })
}

/// 병합된 PR과 닫힌 이슈를 병합하거나 닫은 시각의 최신순으로 후보로 만듭니다.
///
/// 이슈 API 목록 하나로 PR과 이슈를 함께 읽습니다. `--since`는 API에도 넘겨 읽는 쪽 수를 줄입니다.
/// 수정 시각은 닫은 시각보다 늦으므로 API가 빼는 항목은 어차피 범위 밖입니다.
fn candidates(api: &Api, options: &Options) -> Result<Vec<Candidate>, ConnectorError> {
    let repo = &options.repo;
    let mut endpoint = format!("repos/{repo}/issues?state=closed&per_page=100");
    if let Some(since) = options.since {
        endpoint.push_str(&format!(
            "&since={}",
            since.to_utc().format("%Y-%m-%dT%H:%M:%SZ")
        ));
    }
    let mut items: Vec<Item> = api
        .list(&endpoint)?
        .iter()
        .filter(|value| {
            let pull = value.get("pull_request").is_some_and(|p| !p.is_null());
            if pull {
                // 병합하지 않고 닫은 PR은 뺍니다.
                options.pulls && value["pull_request"]["merged_at"].is_string()
            } else {
                options.issues
            }
        })
        .filter_map(item)
        .filter(|item| {
            options.since.is_none_or(|since| {
                DateTime::parse_from_rfc3339(&item.done_at).is_ok_and(|done| done >= since)
            })
        })
        .collect();
    items.sort_by(|a, b| b.done_at.cmp(&a.done_at).then(b.number.cmp(&a.number)));
    if let Some(limit) = options.limit {
        items.truncate(limit);
    }

    // 코멘트는 남은 항목만 읽습니다.
    items
        .into_iter()
        .map(|item| {
            let text = text(api, repo, &item)?;
            Ok(Candidate {
                title: item.title,
                kind: "discussion".to_string(),
                channel: options.channel.clone(),
                origin: item.url,
                origin_version: sha256_hex(&text),
                occurred_at: local_date(&item.done_at),
                text,
            })
        })
        .collect()
}

/// 후보의 원문. 제목, 본문, 코멘트, 리뷰를 정해진 순서로 이어 붙여 같은 내용이면 같은 글이 됩니다.
///
/// 봇의 코멘트와 아직 제출하지 않은 리뷰는 넣지 않습니다. 논의가 그대로인데 판이 바뀌지 않게 하기 위해서입니다.
fn text(api: &Api, repo: &str, item: &Item) -> Result<String, ConnectorError> {
    let mut text = format!("{}\n\n{}", item.title, item.body.trim());
    let comments = api.list(&format!(
        "repos/{repo}/issues/{}/comments?per_page=100",
        item.number
    ))?;
    push_section(&mut text, "코멘트", comments.iter().map(|c| entry(c, None)));
    if item.pull {
        let reviews = api.list(&format!(
            "repos/{repo}/pulls/{}/reviews?per_page=100",
            item.number
        ))?;
        let review_comments = api.list(&format!(
            "repos/{repo}/pulls/{}/comments?per_page=100",
            item.number
        ))?;
        push_section(
            &mut text,
            "리뷰",
            reviews
                .iter()
                .filter(|review| review["state"] != "PENDING")
                .map(|review| entry(review, review["state"].as_str()))
                .chain(
                    review_comments
                        .iter()
                        .map(|comment| entry(comment, comment["path"].as_str())),
                ),
        );
    }
    Ok(text.trim_end().to_string())
}

/// `@작성자 (덧붙임): 본문`. 본문이 비었거나 봇이 쓴 것은 없습니다.
fn entry(value: &Value, note: Option<&str>) -> Option<String> {
    let body = value["body"].as_str().unwrap_or_default().trim();
    if body.is_empty() || value["user"]["type"] == "Bot" {
        return None;
    }
    let login = value["user"]["login"].as_str().unwrap_or("?");
    Some(match note {
        Some(note) => format!("@{login} ({note}): {body}"),
        None => format!("@{login}: {body}"),
    })
}

fn push_section(text: &mut String, title: &str, entries: impl Iterator<Item = Option<String>>) {
    let entries: Vec<String> = entries.flatten().collect();
    if entries.is_empty() {
        return;
    }
    text.push_str(&format!("\n\n## {title}"));
    for entry in entries {
        text.push_str(&format!("\n\n{entry}"));
    }
}

/// 소스 노트 중 GitHub PR과 이슈 원본의 지금 판. 원문을 다시 만들어 해시를 냅니다.
///
/// 지워진 원본(404, 410)만 빼서 `lore check`의 확인하지 못한 건수로 나오게 합니다.
/// 인증, 권한, 호출 한도, 네트워크 오류는 실패입니다. 조용히 빼면 신선도 검사가 아무것도 보지 않고 통과합니다.
fn origins(api: &Api, docs: &Path) -> Result<Vec<OriginVersion>, ConnectorError> {
    let mut latest = Vec::new();
    for entry in read_source_origins(docs)? {
        let Some((repo, number)) = parse_item_url(&entry.origin) else {
            continue;
        };
        // PR 주소든 이슈 주소든 이슈 API로 읽고, PR이면 리뷰까지 넣습니다.
        let Some(value) = api.get(&format!("repos/{repo}/issues/{number}"))? else {
            continue;
        };
        let Some(item) = item(&value) else {
            continue;
        };
        latest.push(OriginVersion {
            origin: entry.origin,
            origin_version: sha256_hex(&text(api, &repo, &item)?),
        });
    }
    Ok(latest)
}

/// `https://github.com/<owner>/<repo>/(pull|pulls|issues)/<번호>`. `www.`, `http://`, `#…`도 받습니다.
fn parse_item_url(url: &str) -> Option<(String, u64)> {
    let url = url.split('#').next()?;
    let path = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let path = path.strip_prefix("www.").unwrap_or(path);
    let path = path.strip_prefix("github.com/")?;
    let parts: Vec<&str> = path.trim_end_matches('/').split('/').collect();
    let [owner, repo, kind, number] = parts.as_slice() else {
        return None;
    };
    if !matches!(*kind, "pull" | "pulls" | "issues") {
        return None;
    }
    Some((format!("{owner}/{repo}"), number.parse().ok()?))
}

/// RFC 3339 시각을 실행 환경의 로컬 시간대로 본 `YYYY-MM-DD`로 바꿉니다.
fn local_date(timestamp: &str) -> String {
    DateTime::parse_from_rfc3339(timestamp)
        .map(|time| time.with_timezone(&Local).format("%Y-%m-%d").to_string())
        .unwrap_or_else(|_| timestamp.chars().take(10).collect())
}

/// `YYYY-MM-DD`의 로컬 자정.
fn local_midnight(date: &str) -> Option<DateTime<Local>> {
    if date.len() != 10 {
        return None;
    }
    let day = NaiveDate::parse_from_str(date, "%Y-%m-%d").ok()?;
    Local
        .from_local_datetime(&day.and_hms_opt(0, 0, 0)?)
        .earliest()
}

fn sha256_hex(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pr과_이슈_주소만_읽는다() {
        for url in [
            "https://github.com/o/r/pull/3",
            "https://github.com/o/r/pulls/3",
            "https://github.com/o/r/pull/3#issuecomment-1",
            "http://www.github.com/o/r/pull/3/",
        ] {
            assert_eq!(parse_item_url(url), Some(("o/r".to_string(), 3)), "{url}");
        }
        assert_eq!(
            parse_item_url("https://github.com/o/r/issues/12"),
            Some(("o/r".to_string(), 12))
        );
        assert_eq!(parse_item_url("https://github.com/o/r/commit/abc"), None);
        assert_eq!(parse_item_url("https://github.com/o/r/pull/x"), None);
        assert_eq!(parse_item_url("https://example.com/o/r/pull/3"), None);
    }

    #[test]
    fn 날짜는_yyyy_mm_dd만_받는다() {
        assert!(local_midnight("2026-11-03").is_some());
        assert!(local_midnight("2026-13-03").is_none());
        assert!(local_midnight("2026-1-3").is_none());
    }

    #[test]
    fn 지워진_원본만_건너뛸_오류로_본다() {
        assert!(is_gone(&ConnectorError::new(
            "github",
            "gh: Not Found (HTTP 404)"
        )));
        assert!(!is_gone(&ConnectorError::new(
            "github",
            "gh: Bad credentials (HTTP 401)"
        )));
        assert!(!is_gone(&ConnectorError::new(
            "github",
            "gh를 실행하지 못했습니다"
        )));
    }
}

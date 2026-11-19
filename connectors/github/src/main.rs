//! `lore-connector-github`: GitHub PR과 이슈에서 소스 노트 후보를 뽑는 커넥터.
//!
//! 병합된 PR과 닫힌 이슈 하나가 후보 하나입니다. GitHub 호출은 `gh api`가 맡고 인증도 `gh`에 맡깁니다.
//! 출력 형식은 기획서 6장의 커넥터 규약을 따릅니다.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use chrono::{DateTime, Local};
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
        None => (|| {
            let channel = matches
                .get_one::<String>(CHANNEL)
                .ok_or_else(|| ConnectorError::new("usage", "--channel <채널 ID>가 필요합니다."))?;
            let repo = match matches.get_one::<String>(REPO) {
                Some(repo) => repo.clone(),
                None => repo_from_remote()?,
            };
            let include: Vec<String> = matches
                .get_many::<String>(INCLUDE)
                .map(|values| values.cloned().collect())
                .unwrap_or_else(|| vec!["pulls".to_string(), "issues".to_string()]);
            let since = matches.get_one::<String>(SINCE).cloned();
            if let Some(date) = &since
                && !is_iso_date(date)
            {
                return Err(ConnectorError::new(
                    "usage",
                    format!("--since '{date}'는 YYYY-MM-DD 날짜가 아닙니다."),
                ));
            }
            let options = Options {
                channel: channel.clone(),
                repo,
                since,
                limit: matches.get_one::<usize>(LIMIT).copied(),
                pulls: include.iter().any(|kind| kind == "pulls"),
                issues: include.iter().any(|kind| kind == "issues"),
            };
            candidates(&api, &options).map(|list| candidates_json(&list))
        })(),
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
    since: Option<String>,
    limit: Option<usize>,
    pulls: bool,
    issues: bool,
}

/// GitHub API를 읽는 곳. 시험에서는 `gh` 대신 고정 응답 파일을 읽습니다.
enum Api {
    Gh,
    /// 엔드포인트의 쿼리를 뺀 경로에서 `/`를 `_`로 바꾼 이름의 `.json` 파일을 읽습니다.
    Fixture(PathBuf),
}

impl Api {
    /// 한 건을 돌려주는 엔드포인트.
    fn get(&self, endpoint: &str) -> Result<Value, ConnectorError> {
        match self {
            Api::Gh => parse(&gh(&["api", endpoint])?),
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
            Api::Fixture(dir) => self.fixture(dir, endpoint)?,
        };
        Ok(value.as_array().cloned().unwrap_or_default())
    }

    fn fixture(&self, dir: &Path, endpoint: &str) -> Result<Value, ConnectorError> {
        let path = endpoint.split('?').next().unwrap_or(endpoint);
        let file = dir.join(format!("{}.json", path.replace('/', "_")));
        let text = std::fs::read_to_string(&file).map_err(|error| {
            ConnectorError::new("github", format!("{}: {error}", file.display()))
        })?;
        parse(&text)
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

fn parse(text: &str) -> Result<Value, ConnectorError> {
    serde_json::from_str(text)
        .map_err(|error| ConnectorError::new("github", format!("응답을 읽지 못했습니다: {error}")))
}

fn repo_from_remote() -> Result<String, ConnectorError> {
    let output = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .output()
        .map_err(|error| ConnectorError::new("usage", error.to_string()))?;
    let url = String::from_utf8_lossy(&output.stdout);
    github_web_url(url.trim())
        .and_then(|web| web.strip_prefix("https://github.com/").map(str::to_string))
        .ok_or_else(|| {
            ConnectorError::new(
                "usage",
                "origin 원격이 GitHub가 아닙니다. --repo <owner/name>을 주세요.",
            )
        })
}

/// 후보로 만들 PR이나 이슈 하나.
struct Item {
    /// `pull` 또는 `issue`
    kind: &'static str,
    number: u64,
    title: String,
    url: String,
    /// 병합하거나 닫은 시각(RFC 3339)
    done_at: String,
    body: String,
}

impl Item {
    fn from(kind: &'static str, value: &Value, done_key: &str) -> Option<Item> {
        Some(Item {
            kind,
            number: value["number"].as_u64()?,
            title: value["title"].as_str()?.to_string(),
            url: value["html_url"].as_str()?.to_string(),
            done_at: value[done_key].as_str()?.to_string(),
            body: value["body"].as_str().unwrap_or_default().to_string(),
        })
    }
}

/// 병합된 PR과 닫힌 이슈를 병합하거나 닫은 시각의 최신순으로 후보로 만듭니다.
fn candidates(api: &Api, options: &Options) -> Result<Vec<Candidate>, ConnectorError> {
    let repo = &options.repo;
    let mut items = Vec::new();
    if options.pulls {
        for pull in api.list(&format!("repos/{repo}/pulls?state=closed&per_page=100"))? {
            items.extend(Item::from("pull", &pull, "merged_at"));
        }
    }
    if options.issues {
        for issue in api.list(&format!("repos/{repo}/issues?state=closed&per_page=100"))? {
            // 이슈 목록에는 PR도 섞여 나옵니다.
            if issue.get("pull_request").is_none() {
                items.extend(Item::from("issue", &issue, "closed_at"));
            }
        }
    }
    items.retain(|item| {
        options
            .since
            .as_deref()
            .is_none_or(|since| local_date(&item.done_at).as_str() >= since)
    });
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
fn text(api: &Api, repo: &str, item: &Item) -> Result<String, ConnectorError> {
    let mut text = format!("{}\n\n{}", item.title, item.body.trim());
    let comments = api.list(&format!(
        "repos/{repo}/issues/{}/comments?per_page=100",
        item.number
    ))?;
    push_section(&mut text, "코멘트", comments.iter().map(|c| entry(c, None)));
    if item.kind == "pull" {
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

/// `@작성자 (덧붙임): 본문`. 본문이 비면 없습니다.
fn entry(value: &Value, note: Option<&str>) -> Option<String> {
    let body = value["body"].as_str().unwrap_or_default().trim();
    if body.is_empty() {
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
/// 읽지 못한 원본(지워졌거나 권한이 없음)은 빼므로 `lore check`에서 확인하지 못한 건수로 나옵니다.
fn origins(api: &Api, docs: &Path) -> Result<Vec<OriginVersion>, ConnectorError> {
    let mut latest = Vec::new();
    for entry in read_source_origins(docs)? {
        let Some((repo, kind, number)) = parse_item_url(&entry.origin) else {
            continue;
        };
        let endpoint = match kind {
            "pull" => format!("repos/{repo}/pulls/{number}"),
            _ => format!("repos/{repo}/issues/{number}"),
        };
        let Ok(value) = api.get(&endpoint) else {
            continue;
        };
        let done_key = if kind == "pull" {
            "merged_at"
        } else {
            "closed_at"
        };
        let mut item = match Item::from(kind, &value, done_key) {
            Some(item) => item,
            // 아직 병합하거나 닫지 않은 것도 원문은 비교합니다.
            None => match Item::from(kind, &value, "created_at") {
                Some(item) => item,
                None => continue,
            },
        };
        item.number = number;
        latest.push(OriginVersion {
            origin: entry.origin,
            origin_version: sha256_hex(&text(api, &repo, &item)?),
        });
    }
    Ok(latest)
}

/// `https://github.com/<owner>/<repo>/(pull|issues)/<번호>`
fn parse_item_url(url: &str) -> Option<(String, &'static str, u64)> {
    let path = url.strip_prefix("https://github.com/")?;
    let parts: Vec<&str> = path.trim_end_matches('/').split('/').collect();
    let [owner, repo, kind, number] = parts.as_slice() else {
        return None;
    };
    let kind = match *kind {
        "pull" => "pull",
        "issues" => "issue",
        _ => return None,
    };
    Some((format!("{owner}/{repo}"), kind, number.parse().ok()?))
}

/// RFC 3339 시각을 실행 환경의 로컬 시간대로 본 `YYYY-MM-DD`로 바꿉니다.
fn local_date(timestamp: &str) -> String {
    DateTime::parse_from_rfc3339(timestamp)
        .map(|time| time.with_timezone(&Local).format("%Y-%m-%d").to_string())
        .unwrap_or_else(|_| timestamp.chars().take(10).collect())
}

fn is_iso_date(date: &str) -> bool {
    chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").is_ok() && date.len() == 10
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
        assert_eq!(
            parse_item_url("https://github.com/o/r/pull/3"),
            Some(("o/r".to_string(), "pull", 3))
        );
        assert_eq!(
            parse_item_url("https://github.com/o/r/issues/12"),
            Some(("o/r".to_string(), "issue", 12))
        );
        assert_eq!(parse_item_url("https://github.com/o/r/commit/abc"), None);
        assert_eq!(parse_item_url("https://github.com/o/r/pull/x"), None);
        assert_eq!(parse_item_url("https://example.com/o/r/pull/3"), None);
    }

    #[test]
    fn 날짜는_yyyy_mm_dd만_받는다() {
        assert!(is_iso_date("2026-11-03"));
        assert!(!is_iso_date("2026-13-03"));
        assert!(!is_iso_date("2026-1-3"));
    }
}

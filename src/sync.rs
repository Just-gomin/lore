//! `lore sync`: 비교 기준 이후의 코드 변경에 맞춰 스펙을 고친다.
//!
//! 이름이 바뀐 파일을 글자 그대로 가리키던 `paths`, `tests` 항목만 고친다. 글롭과 본문은 고치지 않고
//! 확인할 목록으로 낸다. 본문을 고치는 일에는 판단이 필요해 `lore-sync` 스킬이 맡는다.

use serde::Serialize;
use serde_yaml_ng::Value;

use crate::check::{Diff, reached_specs};
use crate::config::Config;
use crate::error::LoreError;
use crate::git::ChangeKind;
use crate::{frontmatter, graph, source};

#[derive(Debug, Serialize)]
pub struct SyncReport {
    pub dry_run: bool,
    /// 비교 기준. `--base`와 `HEAD`의 merge-base 커밋이다.
    pub merge_base: String,
    /// 고친(`--dry-run`이면 고칠) 항목.
    pub fixed: Vec<Fix>,
    /// 고치지 않고 사람이나 스킬이 확인할 항목.
    pub to_check: Vec<ToCheck>,
    /// 바뀐 파일이 닿지만 같은 범위에서 본문이 바뀌지 않은 스펙.
    pub specs_to_review: Vec<Review>,
}

#[derive(Debug, Serialize)]
pub struct Fix {
    pub spec: String,
    pub path: String,
    /// `paths` 또는 `tests`
    pub field: &'static str,
    pub from: String,
    pub to: String,
}

#[derive(Debug, Serialize)]
pub struct ToCheck {
    pub spec: String,
    pub path: String,
    pub field: &'static str,
    /// 머리말에 적힌 항목.
    pub entry: String,
    /// 이름이 바뀐 파일의 원래 경로와 새 경로.
    pub from: String,
    pub to: String,
    /// `glob`: 글롭에 맞던 파일이 글롭 밖으로 옮겨졌다. `unsupported_format`: 목록의 모양을 고칠 수 없다.
    pub reason: &'static str,
}

#[derive(Debug, Serialize)]
pub struct Review {
    pub spec: String,
    pub path: String,
    /// 이 스펙에 닿는 바뀐 파일.
    pub files: Vec<String>,
}

const FIELDS: [&str; 2] = ["paths", "tests"];

pub fn sync(config: &Config, base: &str, dry_run: bool) -> Result<SyncReport, LoreError> {
    let diff = Diff::new(config, base)?;
    let renames: Vec<(&str, &str)> = diff
        .changes
        .iter()
        .filter(|change| change.kind == ChangeKind::Renamed)
        .filter_map(|change| Some((change.from.as_deref()?, change.path.as_str())))
        .collect();
    let (corpus, _) = source::read_lenient(config);

    let mut fixed = Vec::new();
    let mut to_check = Vec::new();
    for doc in corpus.of_kind(source::Kind::Spec) {
        let source::Record::Spec(spec) = &doc.record else {
            continue;
        };
        let mut text = read(config, &doc.path)?;
        let original = text.clone();
        for (field, entries) in FIELDS.into_iter().zip([&spec.paths, &spec.tests]) {
            for entry in entries {
                for (from, to) in &renames {
                    let check = |reason| ToCheck {
                        spec: doc.id.clone(),
                        path: doc.path.clone(),
                        field,
                        entry: entry.clone(),
                        from: from.to_string(),
                        to: to.to_string(),
                        reason,
                    };
                    if is_literal(entry) {
                        if normalize(entry) != *from {
                            continue;
                        }
                        match replace_item(&text, field, entry, to) {
                            Some(edited) => {
                                text = edited;
                                fixed.push(Fix {
                                    spec: doc.id.clone(),
                                    path: doc.path.clone(),
                                    field,
                                    from: from.to_string(),
                                    to: to.to_string(),
                                });
                            }
                            None => to_check.push(check("unsupported_format")),
                        }
                    } else if let Ok(glob) = graph::matcher(entry)
                        && glob.is_match(from)
                        && !glob.is_match(to)
                    {
                        to_check.push(check("glob"));
                    }
                }
            }
        }
        if text != original && !dry_run {
            write(config, &doc.path, &text)?;
        }
    }

    let mut specs_to_review = Vec::new();
    for (doc, files) in reached_specs(&corpus, &diff.changes) {
        if !diff.body_changed(config, doc)? {
            specs_to_review.push(Review {
                spec: doc.id.clone(),
                path: doc.path.clone(),
                files,
            });
        }
    }

    Ok(SyncReport {
        dry_run,
        merge_base: diff.merge_base,
        fixed,
        to_check,
        specs_to_review,
    })
}

fn read(config: &Config, path: &str) -> Result<String, LoreError> {
    std::fs::read_to_string(config.root.join(path)).map_err(|error| LoreError::Io {
        path: path.to_string(),
        message: error.to_string(),
    })
}

fn write(config: &Config, path: &str, text: &str) -> Result<(), LoreError> {
    std::fs::write(config.root.join(path), text).map_err(|error| LoreError::Io {
        path: path.to_string(),
        message: error.to_string(),
    })
}

/// 글롭 문자가 없는 항목. 파일 하나를 글자 그대로 가리킨다.
fn is_literal(entry: &str) -> bool {
    !entry.contains(['*', '?', '[', ']', '{', '}'])
}

fn normalize(entry: &str) -> &str {
    entry.trim_start_matches("./")
}

/// 머리말의 `field` 목록에서 `entry` 항목을 `to`로 바꾼다. 나머지 글자와 주석은 그대로 둔다.
///
/// 블록 형식(`- 경로`)과 한 줄 형식(`[a, b]`)을 고친다. 고친 뒤 다시 읽어 `to`가 들어갔는지 확인하고,
/// 고치지 못했으면 `None`이다.
pub fn replace_item(text: &str, field: &str, entry: &str, to: &str) -> Option<String> {
    let bom = if text.starts_with('\u{feff}') {
        "\u{feff}"
    } else {
        ""
    };
    let text = &text[bom.len()..];
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    if lines.first()?.trim_end() != "---" {
        return None;
    }
    let end = lines
        .iter()
        .skip(1)
        .position(|line| line.trim_end() == "---")?
        + 1;
    let key = format!("{field}:");
    let start = (1..end).find(|&i| lines[i].starts_with(&key))?;

    let mut out: Vec<String> = lines.iter().map(|line| line.to_string()).collect();
    let mut replaced = false;
    let rest = &lines[start][key.len()..];
    if rest.trim_start().starts_with('[') {
        let open = key.len() + rest.find('[')?;
        let close = open + closing_bracket(&lines[start][open..])?;
        let inner = &lines[start][open + 1..close];
        let items: Vec<String> = inner
            .split(',')
            .map(|item| {
                if replaced {
                    return item.to_string();
                }
                let edited = replace_scalar(item, entry, to);
                replaced = edited.is_some();
                edited.unwrap_or_else(|| item.to_string())
            })
            .collect();
        out[start] = format!(
            "{}{}{}",
            &lines[start][..=open],
            items.join(","),
            &lines[start][close..]
        );
    } else {
        for i in start + 1..end {
            let line = lines[i];
            let trimmed = line.trim_start();
            if trimmed.trim().is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let indented = line.starts_with([' ', '\t']);
            if !trimmed.starts_with("- ") || (!indented && !line.starts_with("- ")) {
                break;
            }
            let item_start = line.len() - trimmed.len() + 2;
            let (value, comment) = split_comment(&line[item_start..]);
            if let Some(edited) = replace_scalar(value, entry, to) {
                out[i] = format!("{}{edited}{comment}", &line[..item_start]);
                replaced = true;
                break;
            }
        }
    }
    if !replaced {
        return None;
    }
    let edited = format!("{bom}{}", out.concat());
    contains_item(&edited, field, to).then_some(edited)
}

/// `[`로 시작하는 글에서 짝이 되는 `]`의 위치. 따옴표 안의 `]`는 건너뛴다.
fn closing_bracket(text: &str) -> Option<usize> {
    let mut quote = None;
    for (i, ch) in text.char_indices().skip(1) {
        match (quote, ch) {
            (None, '\'' | '"') => quote = Some(ch),
            (Some(q), c) if c == q => quote = None,
            (None, ']') => return Some(i),
            _ => {}
        }
    }
    None
}

/// 목록 항목 하나의 값이 `entry`면 `to`로 바꾼다. 앞뒤 공백과 따옴표 모양은 그대로 둔다.
fn replace_scalar(item: &str, entry: &str, to: &str) -> Option<String> {
    let value = item.trim();
    let lead = &item[..item.len() - item.trim_start().len()];
    let tail = &item[item.trim_end().len()..];
    let (quote, inner) = match value.chars().next() {
        Some(q @ ('\'' | '"')) if value.len() >= 2 && value.ends_with(q) => {
            (Some(q), &value[1..value.len() - 1])
        }
        _ => (None, value),
    };
    let unquoted = match quote {
        Some('\'') => inner.replace("''", "'"),
        Some(_) => inner.replace("\\\"", "\"").replace("\\\\", "\\"),
        None => inner.to_string(),
    };
    if unquoted != entry {
        return None;
    }
    let replaced = match quote {
        Some('\'') => format!("'{}'", to.replace('\'', "''")),
        Some(_) => format!("\"{}\"", to.replace('\\', "\\\\").replace('"', "\\\"")),
        None if needs_quote(to) => format!("'{}'", to.replace('\'', "''")),
        None => to.to_string(),
    };
    Some(format!("{lead}{replaced}{tail}"))
}

/// 따옴표 없이 쓰면 같은 문자열로 읽히지 않는 값인지. 한 줄 목록의 쉼표와 괄호도 따옴표가 필요하다.
fn needs_quote(value: &str) -> bool {
    value.contains([',', '[', ']', '{', '}', '#', '\t'])
        || value.starts_with(['-', '?', ':'])
        || serde_yaml_ng::from_str::<Value>(value).ok() != Some(Value::String(value.to_string()))
}

/// 블록 항목의 값과 줄 끝 주석(줄바꿈 포함)을 나눈다.
fn split_comment(rest: &str) -> (&str, &str) {
    let content_end = rest.trim_end_matches(['\r', '\n']).len();
    let mut quote = None;
    let value_start = rest.len() - rest.trim_start().len();
    for (i, ch) in rest[..content_end].char_indices() {
        match (quote, ch) {
            // 따옴표는 값의 첫 글자일 때만 감싼다. `it's.ts`의 `'`는 글자다.
            (None, '\'' | '"') if i == value_start => quote = Some(ch),
            (Some(q), c) if c == q => quote = None,
            (None, '#') if rest[..i].ends_with([' ', '\t']) => {
                let value_end = rest[..i].trim_end().len();
                return (&rest[..value_end], &rest[value_end..]);
            }
            _ => {}
        }
    }
    (&rest[..content_end], &rest[content_end..])
}

fn contains_item(text: &str, field: &str, value: &str) -> bool {
    let Some((yaml, _)) = frontmatter::split(text) else {
        return false;
    };
    let Ok(Value::Mapping(map)) = serde_yaml_ng::from_str::<Value>(yaml) else {
        return false;
    };
    map.get(field)
        .and_then(Value::as_sequence)
        .is_some_and(|items| items.iter().any(|item| item.as_str() == Some(value)))
}

/// 사람용 결과.
pub fn render(report: &SyncReport) -> String {
    let fixed_label = if report.dry_run {
        "고칠 항목"
    } else {
        "고친 항목"
    };
    let mut text = format!(
        "lore sync: {fixed_label} {}, 확인할 항목 {}, 본문을 확인할 스펙 {}",
        report.fixed.len(),
        report.to_check.len(),
        report.specs_to_review.len()
    );
    if !report.fixed.is_empty() {
        text.push_str(&format!("\n\n{fixed_label}"));
        for fix in &report.fixed {
            text.push_str(&format!(
                "\n  {} [{}] {} → {}",
                fix.path, fix.field, fix.from, fix.to
            ));
        }
    }
    if !report.to_check.is_empty() {
        text.push_str("\n\n확인할 항목");
        for item in &report.to_check {
            let why = match item.reason {
                "glob" => format!("'{}'에 맞던 파일이 글롭 밖으로 옮겨졌습니다", item.entry),
                _ => format!("'{}' 항목의 모양을 고칠 수 없습니다", item.entry),
            };
            text.push_str(&format!(
                "\n  {} [{}] {why}: {} → {}",
                item.path, item.field, item.from, item.to
            ));
        }
    }
    if !report.specs_to_review.is_empty() {
        text.push_str("\n\n본문을 확인할 스펙");
        for review in &report.specs_to_review {
            text.push_str(&format!(
                "\n  {} ({}): {}",
                review.spec,
                review.path,
                review.files.join(", ")
            ));
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLOCK: &str = "---\nid: a\npaths:\n  - src/old.ts # 진입점\n  - 'src/x.ts'\ntests: []\n---\n\n본문 src/old.ts\n";

    #[test]
    fn 블록_목록의_항목만_바꾸고_주석과_본문은_그대로_둔다() {
        let edited = replace_item(BLOCK, "paths", "src/old.ts", "src/new.ts").unwrap();
        assert_eq!(
            edited,
            "---\nid: a\npaths:\n  - src/new.ts # 진입점\n  - 'src/x.ts'\ntests: []\n---\n\n본문 src/old.ts\n"
        );
        let edited = replace_item(BLOCK, "paths", "src/x.ts", "src/y.ts").unwrap();
        assert!(edited.contains("  - 'src/y.ts'\n"));
    }

    #[test]
    fn 한_줄_목록의_항목을_바꾼다() {
        let text = "---\nid: a\npaths: [src/a.ts, \"src/old.ts\"] # 둘\n---\n";
        assert_eq!(
            replace_item(text, "paths", "src/old.ts", "src/new.ts").unwrap(),
            "---\nid: a\npaths: [src/a.ts, \"src/new.ts\"] # 둘\n---\n"
        );
    }

    #[test]
    fn 들여쓰지_않은_블록_목록과_윈도우_줄바꿈도_고친다() {
        let text = "---\r\nid: a\r\ntests:\r\n- t/old.test.ts\r\npaths: []\r\n---\r\n";
        assert_eq!(
            replace_item(text, "tests", "t/old.test.ts", "t/new.test.ts").unwrap(),
            "---\r\nid: a\r\ntests:\r\n- t/new.test.ts\r\npaths: []\r\n---\r\n"
        );
    }

    #[test]
    fn 다른_항목의_같은_글자는_건드리지_않는다() {
        let text = "---\nid: a\ntests:\n  - src/old.ts\npaths:\n  - src/old.ts\n---\n";
        assert_eq!(
            replace_item(text, "paths", "src/old.ts", "src/new.ts").unwrap(),
            "---\nid: a\ntests:\n  - src/old.ts\npaths:\n  - src/new.ts\n---\n"
        );
        assert!(replace_item(text, "paths", "src/none.ts", "src/new.ts").is_none());
    }

    #[test]
    fn 주석의_괄호와_아포스트로피와_bom을_견딘다() {
        let text = "\u{feff}---\npaths: [src/old.ts] # see [x], src/old.ts\ntests:\n  - src/it's.ts # 메모\n  - 'src/o''k.ts'\n---\n";
        let edited = replace_item(text, "paths", "src/old.ts", "src/new.ts").unwrap();
        assert_eq!(edited, text.replacen("[src/old.ts]", "[src/new.ts]", 1));
        let edited = replace_item(text, "tests", "src/it's.ts", "src/it.ts").unwrap();
        assert!(edited.contains("  - src/it.ts # 메모\n"));
        let edited = replace_item(text, "tests", "src/o'k.ts", "src/ok.ts").unwrap();
        assert!(edited.contains("  - 'src/ok.ts'\n"));
    }

    #[test]
    fn 따옴표가_필요한_새_경로는_작은따옴표로_감싼다() {
        let text = "---\npaths:\n  - src/old.ts\n---\n";
        assert_eq!(
            replace_item(text, "paths", "src/old.ts", "src/a, b.ts").unwrap(),
            "---\npaths:\n  - 'src/a, b.ts'\n---\n"
        );
        for value in ["true", "123", "null", "dir/a:", "#a"] {
            assert!(needs_quote(value), "{value}");
        }
        assert!(!needs_quote("services/payment/index.ts"));
    }
}

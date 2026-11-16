//! 마크다운의 머리말(`---` 사이의 YAML)을 떼어 내고 항목을 읽는다.

use std::collections::BTreeSet;

use serde_yaml_ng::{Mapping, Value};

use crate::error::Problem;

/// 머리말과 본문을 나눈다. 첫 줄이 `---`가 아니거나 닫는 줄이 없으면 `None`이다.
pub fn split(text: &str) -> Option<(&str, &str)> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let rest = text.strip_prefix("---")?;
    let rest = rest
        .strip_prefix("\r\n")
        .or_else(|| rest.strip_prefix('\n'))?;

    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        if line.trim_end_matches(['\r', '\n']) == "---" {
            return Some((&rest[..offset], &rest[offset + line.len()..]));
        }
        offset += line.len();
    }
    None
}

/// 머리말 항목을 읽으며 문제를 모은다.
///
/// 읽은 항목을 기억해 두었다가 [`Fields::finish`]에서 모르는 항목을 보고한다.
pub struct Fields<'a> {
    map: &'a Mapping,
    path: &'a str,
    known: BTreeSet<&'static str>,
    problems: &'a mut Vec<Problem>,
}

impl<'a> Fields<'a> {
    pub fn new(map: &'a Mapping, path: &'a str, problems: &'a mut Vec<Problem>) -> Fields<'a> {
        Fields {
            map,
            path,
            known: BTreeSet::new(),
            problems,
        }
    }

    fn problem(&mut self, field: &str, code: &'static str, message: String) {
        self.problems
            .push(Problem::new(self.path, Some(field), code, message));
    }

    /// 값이 없거나 `null`이면 `None`이다.
    fn get(&mut self, key: &'static str) -> Option<&'a Value> {
        self.known.insert(key);
        self.map.get(key).filter(|value| !value.is_null())
    }

    fn missing(&mut self, key: &'static str) {
        self.problem(key, "missing_field", format!("필수 항목 {key}가 없습니다."));
    }

    pub fn string(&mut self, key: &'static str, required: bool) -> Option<String> {
        let value = match self.get(key) {
            Some(value) => value,
            None => {
                if required {
                    self.missing(key);
                }
                return None;
            }
        };
        match value.as_str().map(str::trim) {
            Some("") if required => {
                self.missing(key);
                None
            }
            Some("") => None,
            Some(text) => Some(text.to_string()),
            None => {
                self.problem(key, "invalid_type", format!("{key}는 문자열이어야 합니다."));
                None
            }
        }
    }

    /// 정해진 값 중 하나여야 하는 항목.
    pub fn choice(
        &mut self,
        key: &'static str,
        allowed: &[&str],
        required: bool,
    ) -> Option<String> {
        let value = self.string(key, required)?;
        if allowed.contains(&value.as_str()) {
            Some(value)
        } else {
            self.problem(
                key,
                "invalid_value",
                format!(
                    "{key}는 {} 중 하나여야 합니다. '{value}'는 받지 않습니다.",
                    allowed.join(", ")
                ),
            );
            None
        }
    }

    /// 문자열 목록. 없으면 빈 목록이다.
    pub fn list(&mut self, key: &'static str, required: bool) -> Vec<String> {
        let value = match self.get(key) {
            Some(value) => value,
            None => {
                if required && !self.map.contains_key(key) {
                    self.missing(key);
                }
                return Vec::new();
            }
        };
        let items = match value.as_sequence() {
            Some(items) => items,
            None => {
                self.problem(key, "invalid_type", format!("{key}는 목록이어야 합니다."));
                return Vec::new();
            }
        };

        let mut out = Vec::with_capacity(items.len());
        for item in items {
            match item.as_str().map(str::trim) {
                Some(text) if !text.is_empty() => out.push(text.to_string()),
                _ => self.problem(
                    key,
                    "invalid_type",
                    format!("{key}의 항목은 비어 있지 않은 문자열이어야 합니다."),
                ),
            }
        }
        out
    }

    /// 읽지 않은 항목을 모르는 항목으로 보고한다.
    pub fn finish(self) {
        let mut unknown = Vec::new();
        for key in self.map.keys() {
            match key.as_str() {
                Some(name) if self.known.contains(name) => {}
                Some(name) => unknown.push(name.to_string()),
                None => unknown.push(format!("{key:?}")),
            }
        }
        unknown.sort();
        for name in unknown {
            self.problems.push(Problem::new(
                self.path,
                Some(&name),
                "unknown_field",
                format!("모르는 항목 {name}입니다."),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 머리말과_본문을_나눈다() {
        let text = "---\nid: a\n---\n\n## 본문\n";
        assert_eq!(split(text), Some(("id: a\n", "\n## 본문\n")));
    }

    #[test]
    fn crlf와_bom도_받는다() {
        let text = "\u{feff}---\r\nid: a\r\n---\r\n본문";
        assert_eq!(split(text), Some(("id: a\r\n", "본문")));
    }

    #[test]
    fn 머리말이_없거나_닫히지_않으면_none이다() {
        assert_eq!(split("# 제목\n"), None);
        assert_eq!(split("---\nid: a\n"), None);
        assert_eq!(split("----\nid: a\n---\n"), None);
    }

    #[test]
    fn 항목_문제를_모으고_모르는_항목을_알린다() {
        let map: Mapping =
            serde_yaml_ng::from_str("title: 3\nstatus: done\nspecs: a\nowner: me\nid: x\n")
                .unwrap();
        let mut problems = Vec::new();
        let mut fields = Fields::new(&map, "a.md", &mut problems);
        assert_eq!(fields.string("id", true), Some("x".to_string()));
        assert_eq!(fields.string("title", true), None);
        assert_eq!(fields.choice("status", &["accepted"], true), None);
        assert_eq!(fields.list("specs", false), Vec::<String>::new());
        assert_eq!(fields.string("confidence", true), None);
        fields.finish();

        let codes: Vec<(&str, &str)> = problems
            .iter()
            .map(|p| (p.field.as_deref().unwrap(), p.code))
            .collect();
        assert_eq!(
            codes,
            [
                ("title", "invalid_type"),
                ("status", "invalid_value"),
                ("specs", "invalid_type"),
                ("confidence", "missing_field"),
                ("owner", "unknown_field"),
            ]
        );
    }
}

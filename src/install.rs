//! `lore install`: 프로젝트에 lore로 작업하는 방법을 심는다.
//!
//! `AGENTS.md`의 안내문 구간, 도구별 파일의 한 줄, 설정과 문서 뼈대를 만든다.
//! 여러 번 실행해도 결과가 같고, 구간 밖의 글은 건드리지 않는다.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::config::{self, CONFIG_FILE};
use crate::error::LoreError;
use crate::frontmatter;
use crate::git;
use crate::source::{Kind, sha256_hex};
use crate::why::display_width;

/// 표식 형식의 버전. 구간 본문이 바뀌어도 올리지 않고, 표식의 모양을 바꿀 때만 올린다.
pub const MARKER_VERSION: u32 = 1;
const BEGIN_PREFIX: &str = "<!-- lore:begin v";
const END: &str = "<!-- lore:end -->";

/// 안내문 본문.
pub const AGENTS_BODY: &str = include_str!("../templates/agents.md");
/// 도구별 파일에 넣는 한 줄.
pub const TOOL_POINTER: &str = include_str!("../templates/tool-pointer.md");
const DEFAULT_CONFIG: &str = include_str!("../templates/lore.config.yaml");

/// 내장 스킬. 이름과 템플릿이다.
pub const SKILLS: [(&str, &str); 3] = [
    (
        "lore-record",
        include_str!("../templates/skills/lore-record.md"),
    ),
    (
        "lore-source",
        include_str!("../templates/skills/lore-source.md"),
    ),
    (
        "lore-sync",
        include_str!("../templates/skills/lore-sync.md"),
    ),
];

/// 기존 프로젝트에 lore를 들일 때만 쓰는 `lore init`의 세 스킬. `lore install --init`으로 설치한다.
pub const INIT_SKILLS: [(&str, &str); 3] = [
    (
        "lore-init-channels",
        include_str!("../templates/skills/lore-init-channels.md"),
    ),
    (
        "lore-init-collect",
        include_str!("../templates/skills/lore-init-collect.md"),
    ),
    (
        "lore-init-draft",
        include_str!("../templates/skills/lore-init-draft.md"),
    ),
];
/// 스킬을 두는 폴더. Claude Code의 프로젝트 스킬 위치다.
pub const SKILL_DIR: &str = ".claude/skills";
const HASH_KEY: &str = "metadata:\n  lore-hash: ";

/// `lore check`를 PR마다 돌리는 GitHub Actions 워크플로.
pub const WORKFLOW_FILE: &str = ".github/workflows/lore.yml";
const WORKFLOW_TEMPLATE: &str = include_str!("../templates/lore-workflow.yml");
/// 워크플로가 바이너리를 내려받는 릴리스의 레포.
pub const RELEASE_REPO: &str = "Just-gomin/lore";
const WORKFLOW_HASH: &str = "# lore-hash: ";

pub const AGENTS_FILE: &str = "AGENTS.md";
/// 이미 있을 때만 한 줄을 넣는 도구별 파일.
pub const TOOL_FILES: [&str; 2] = ["CLAUDE.md", "GEMINI.md"];

/// 파일에 일어난 일. 걷어내기의 지움과 남김도 여기에 둔다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Change {
    Created,
    Updated,
    Unchanged,
    Removed,
    Kept,
}

impl Change {
    pub fn label(self) -> &'static str {
        match self {
            Change::Created => "만듦",
            Change::Updated => "바꿈",
            Change::Unchanged => "그대로",
            Change::Removed => "지움",
            Change::Kept => "남김",
        }
    }

    const ALL: [Change; 5] = [
        Change::Created,
        Change::Updated,
        Change::Unchanged,
        Change::Removed,
        Change::Kept,
    ];
}

#[derive(Debug, Clone, Serialize)]
pub struct FileChange {
    /// 프로젝트 홈 기준 경로. 폴더는 `/`로 끝난다.
    pub path: String,
    pub change: Change,
}

/// 설치나 걷어내기의 결과. JSON 출력이 된다.
#[derive(Debug, Serialize)]
pub struct InstallReport {
    pub dry_run: bool,
    pub files: Vec<FileChange>,
}

/// 설치할 프로젝트 홈. 위로 올라가며 설정 파일을 찾고, 없으면 git 레포의 최상위 폴더다.
pub fn project_root(cwd: &Path) -> Result<PathBuf, LoreError> {
    config::find_root(cwd)
        .or_else(|| git::toplevel(cwd))
        .ok_or_else(|| LoreError::NotAGitRepo {
            start: cwd.display().to_string(),
        })
}

/// 설치할 것. `--no-skills`, `--no-ci`로 끈다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Parts {
    pub skills: bool,
    pub ci: bool,
    /// `--init`: `lore init` 스킬을 설치하고, 새로 만드는 설정에 `check.scope: []`를 넣는다.
    pub init: bool,
}

pub fn install(cwd: &Path, dry_run: bool, parts: Parts) -> Result<InstallReport, LoreError> {
    let root = project_root(cwd)?;
    let mut plan = Plan::new(&root, dry_run);

    let config_path = root.join(CONFIG_FILE);
    let docs = if config_path.exists() {
        plan.record(CONFIG_FILE, Change::Unchanged);
        config::load(&root)?.docs
    } else {
        let text = if parts.init {
            init_config()
        } else {
            DEFAULT_CONFIG.to_string()
        };
        plan.write(CONFIG_FILE, None, &text)?;
        "docs".to_string()
    };

    for kind in Kind::ALL {
        let dir = format!("{docs}/{}", kind.folder());
        if root.join(&dir).is_dir() {
            plan.record(&format!("{dir}/"), Change::Unchanged);
        } else {
            plan.write(&format!("{dir}/.gitkeep"), None, "")?;
        }
    }

    let agents = plan.read(AGENTS_FILE)?;
    let text = put_section(agents.as_deref(), AGENTS_BODY, AGENTS_FILE)?;
    plan.write(AGENTS_FILE, agents.as_deref(), &text)?;

    for file in TOOL_FILES {
        if let Some(old) = plan.read(file)? {
            let text = put_section(Some(&old), TOOL_POINTER, file)?;
            plan.write(file, Some(&old), &text)?;
        }
    }

    // `--no-skills`면 init 스킬도 설치하지 않습니다. init 스킬이 lore-source, lore-record 스킬을 쓰기 때문입니다.
    // `--init` 없이 다시 설치해도 이미 있는 init 스킬은 새 버전으로 바꿉니다.
    let mut skills: Vec<(&str, &str)> = Vec::new();
    if parts.skills {
        skills.extend(SKILLS);
        for (name, template) in INIT_SKILLS {
            if parts.init || root.join(skill_path(name)).exists() {
                skills.push((name, template));
            }
        }
    }
    for (name, template) in skills {
        let path = skill_path(name);
        let text = installed_skill(template);
        match plan.read(&path)? {
            // 사람이 고친 스킬은 덮어쓰지 않는다.
            Some(old) if old != text && !skill_untouched(&old) => plan.record(&path, Change::Kept),
            old => plan.write(&path, old.as_deref(), &text)?,
        }
    }

    if parts.ci {
        let text = installed_workflow();
        match plan.read(WORKFLOW_FILE)? {
            // 사람이 고친 워크플로는 덮어쓰지 않는다.
            Some(old) if old != text && !workflow_untouched(&old) => {
                plan.record(WORKFLOW_FILE, Change::Kept)
            }
            old => plan.write(WORKFLOW_FILE, old.as_deref(), &text)?,
        }
    }

    Ok(plan.finish())
}

/// lore가 설치하는 파일. `lore check`의 매핑 누락 검사에서 뺀다.
pub fn installed_files() -> Vec<String> {
    let mut files: Vec<String> = std::iter::once(AGENTS_FILE)
        .chain(TOOL_FILES)
        .map(str::to_string)
        .collect();
    files.extend(
        SKILLS
            .iter()
            .chain(&INIT_SKILLS)
            .map(|(name, _)| skill_path(name)),
    );
    files.push(WORKFLOW_FILE.to_string());
    files
}

/// 설치할 워크플로. 이 lore와 같은 버전의 릴리스 바이너리를 받는다.
///
/// 첫 줄에 나머지 내용의 해시를 적어, 다시 설치하거나 걷어낼 때 사람이 고쳤는지 본다.
pub fn installed_workflow() -> String {
    let body = WORKFLOW_TEMPLATE
        .replace("{{LORE_VERSION}}", env!("CARGO_PKG_VERSION"))
        .replace("{{LORE_REPO}}", RELEASE_REPO);
    format!("{WORKFLOW_HASH}{}\n{body}", sha256_hex(body.as_bytes()))
}

/// 설치한 뒤 아무도 고치지 않은 워크플로인지. 예전 버전이 설치했어도 고치지 않았으면 참이다.
pub fn workflow_untouched(text: &str) -> bool {
    let Some(rest) = text.strip_prefix(WORKFLOW_HASH) else {
        return false;
    };
    let Some((recorded, body)) = rest.split_once('\n') else {
        return false;
    };
    sha256_hex(body.as_bytes()) == recorded
}

/// `lore install --init`이 새로 만드는 설정. `lore init`을 마치기 전에는 판단이 드는 검사가 아무 파일도 보지 않는다.
fn init_config() -> String {
    let marker = "  ignore: []\n";
    assert!(
        DEFAULT_CONFIG.contains(marker),
        "기본 설정에 check.ignore가 있다"
    );
    DEFAULT_CONFIG.replacen(
        marker,
        "  ignore: []\n  # 판단이 드는 검사를 켤 범위. lore init을 마친 모듈부터 server/**처럼 폴더 글롭으로 더합니다.\n  scope: []\n",
        1,
    )
}

pub fn skill_path(name: &str) -> String {
    format!("{SKILL_DIR}/{name}/SKILL.md")
}

/// 설치할 스킬 파일. 템플릿의 머리말 끝에 템플릿 전체의 해시를 더한다.
pub fn installed_skill(template: &str) -> String {
    let (yaml, body) = frontmatter::split(template).expect("내장 스킬에는 머리말이 있다");
    format!(
        "---\n{yaml}{HASH_KEY}{}\n---\n{body}",
        sha256_hex(template.as_bytes())
    )
}

/// 설치한 뒤 아무도 고치지 않은 스킬인지. 해시 줄을 빼고 되돌린 글의 해시가 적힌 해시와 같아야 한다.
///
/// 지금 버전과 달라도 예전 버전이 설치하고 아무도 고치지 않았으면 참이다.
pub fn skill_untouched(text: &str) -> bool {
    let Some(start) = text.find(HASH_KEY) else {
        return false;
    };
    let value_start = start + HASH_KEY.len();
    let Some(line_end) = text[value_start..].find('\n') else {
        return false;
    };
    let recorded = &text[value_start..value_start + line_end];
    let original = format!("{}{}", &text[..start], &text[value_start + line_end + 1..]);
    sha256_hex(original.as_bytes()) == recorded
}

/// 바꿀 파일을 모으고, `dry_run`이 아니면 실제로 쓴다.
pub(crate) struct Plan<'a> {
    root: &'a Path,
    dry_run: bool,
    files: Vec<FileChange>,
}

impl<'a> Plan<'a> {
    pub(crate) fn new(root: &'a Path, dry_run: bool) -> Plan<'a> {
        Plan {
            root,
            dry_run,
            files: Vec::new(),
        }
    }

    fn io_error(path: &str, error: std::io::Error) -> LoreError {
        LoreError::Io {
            path: path.to_string(),
            message: error.to_string(),
        }
    }

    /// 파일 내용. 없으면 `None`이다.
    pub(crate) fn read(&self, path: &str) -> Result<Option<String>, LoreError> {
        match std::fs::read_to_string(self.root.join(path)) {
            Ok(text) => Ok(Some(text)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(Self::io_error(path, error)),
        }
    }

    pub(crate) fn record(&mut self, path: &str, change: Change) {
        self.files.push(FileChange {
            path: path.to_string(),
            change,
        });
    }

    /// `old`가 지금 내용이다. 같으면 쓰지 않는다.
    pub(crate) fn write(
        &mut self,
        path: &str,
        old: Option<&str>,
        text: &str,
    ) -> Result<(), LoreError> {
        let change = match old {
            None => Change::Created,
            Some(old) if old == text => Change::Unchanged,
            Some(_) => Change::Updated,
        };
        if change != Change::Unchanged && !self.dry_run {
            let file = self.root.join(path);
            if let Some(dir) = file.parent() {
                std::fs::create_dir_all(dir).map_err(|error| Self::io_error(path, error))?;
            }
            std::fs::write(&file, text).map_err(|error| Self::io_error(path, error))?;
        }
        self.record(path, change);
        Ok(())
    }

    /// 파일을 지우고, 그 때문에 비게 된 폴더도 프로젝트 홈 바로 아래까지 함께 지운다.
    pub(crate) fn remove(&mut self, path: &str) -> Result<(), LoreError> {
        if !self.dry_run {
            let file = self.root.join(path);
            std::fs::remove_file(&file).map_err(|error| Self::io_error(path, error))?;
            let mut dir = file.parent();
            while let Some(current) = dir.filter(|current| *current != self.root) {
                // 비어 있지 않은 폴더에서 실패하고 멈춘다.
                if std::fs::remove_dir(current).is_err() {
                    break;
                }
                dir = current.parent();
            }
        }
        self.record(path, Change::Removed);
        Ok(())
    }

    pub(crate) fn finish(self) -> InstallReport {
        InstallReport {
            dry_run: self.dry_run,
            files: self.files,
        }
    }
}

/// 본문을 표식으로 감싼 구간.
pub fn section(body: &str) -> String {
    format!(
        "{BEGIN_PREFIX}{MARKER_VERSION} -->\n\n{}\n\n{END}\n",
        body.trim_end()
    )
}

/// 구간이 차지하는 바이트 범위. 끝 표식 줄의 줄바꿈까지 든다. 구간이 없으면 `None`이다.
///
/// 시작이나 끝 표식만 있거나, 구간이 둘 이상이거나, 순서가 뒤집혔으면 `broken_marker`다.
pub fn find_section(text: &str, path: &str) -> Result<Option<(usize, usize)>, LoreError> {
    let mut begins = Vec::new();
    let mut ends = Vec::new();
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim();
        let is_begin = trimmed
            .strip_prefix(BEGIN_PREFIX)
            .and_then(|rest| rest.strip_suffix(" -->"))
            .is_some_and(|version| version.parse::<u32>().is_ok());
        if is_begin {
            begins.push(offset);
        } else if trimmed == END {
            ends.push(offset + line.len());
        }
        offset += line.len();
    }
    match (begins.as_slice(), ends.as_slice()) {
        ([], []) => Ok(None),
        ([begin], [end]) if begin < end => Ok(Some((*begin, *end))),
        _ => Err(LoreError::BrokenMarker {
            path: path.to_string(),
        }),
    }
}

/// 구간을 넣거나 바꾼 글. 구간이 없으면 빈 줄 하나를 두고 파일 끝에 붙인다.
pub fn put_section(text: Option<&str>, body: &str, path: &str) -> Result<String, LoreError> {
    let section = section(body);
    let Some(text) = text else {
        return Ok(section);
    };
    Ok(match find_section(text, path)? {
        Some((start, end)) => format!("{}{section}{}", &text[..start], &text[end..]),
        None if text.is_empty() => section,
        None if text.ends_with('\n') => format!("{text}\n{section}"),
        None => format!("{text}\n\n{section}"),
    })
}

/// 구간을 지운 글. 구간이 없으면 `None`이다.
///
/// 설치가 구간 앞에 더한 빈 줄도 함께 지워, 끝에 붙였던 구간이면 설치 전의 글로 돌아간다.
pub fn remove_section(text: &str, path: &str) -> Result<Option<String>, LoreError> {
    let Some((start, end)) = find_section(text, path)? else {
        return Ok(None);
    };
    let mut before = &text[..start];
    let mut after = &text[end..];
    if before.ends_with("\n\n") {
        if after.is_empty() {
            before = &before[..before.len() - 1];
        } else if let Some(rest) = after.strip_prefix('\n') {
            after = rest;
        }
    }
    Ok(Some(format!("{before}{after}")))
}

/// `lore install`이 넣은 것을 걷어낸다. `docs/`의 기록은 지우지 않는다.
pub fn uninstall(cwd: &Path, dry_run: bool, with_config: bool) -> Result<InstallReport, LoreError> {
    let root = project_root(cwd)?;
    let mut plan = Plan::new(&root, dry_run);

    if let Some(old) = plan.read(AGENTS_FILE)? {
        match remove_section(&old, AGENTS_FILE)? {
            None => plan.record(AGENTS_FILE, Change::Unchanged),
            // 구간만 있던 파일은 설치가 만든 것이다.
            Some(text) if text.trim().is_empty() => plan.remove(AGENTS_FILE)?,
            Some(text) => plan.write(AGENTS_FILE, Some(&old), &text)?,
        }
    }

    // 도구별 파일은 사람이 만든 파일이므로 비어도 지우지 않는다.
    for file in TOOL_FILES {
        if let Some(old) = plan.read(file)? {
            match remove_section(&old, file)? {
                None => plan.record(file, Change::Unchanged),
                Some(text) => plan.write(file, Some(&old), &text)?,
            }
        }
    }

    for (name, _) in SKILLS.iter().chain(&INIT_SKILLS) {
        let path = skill_path(name);
        if let Some(text) = plan.read(&path)? {
            if skill_untouched(&text) {
                plan.remove(&path)?;
            } else {
                plan.record(&path, Change::Kept);
            }
        }
    }

    if let Some(text) = plan.read(WORKFLOW_FILE)? {
        if workflow_untouched(&text) {
            plan.remove(WORKFLOW_FILE)?;
        } else {
            plan.record(WORKFLOW_FILE, Change::Kept);
        }
    }

    if with_config && root.join(CONFIG_FILE).exists() {
        plan.remove(CONFIG_FILE)?;
    }

    Ok(plan.finish())
}

/// 사람용 설치 결과.
pub fn render(report: &InstallReport) -> String {
    let counts: Vec<String> = Change::ALL
        .into_iter()
        .filter_map(|change| {
            let count = report.files.iter().filter(|f| f.change == change).count();
            (count > 0).then(|| format!("{} {count}", change.label()))
        })
        .collect();
    let mut text = format!("lore: {}", counts.join(", "));
    for file in &report.files {
        let label = file.change.label();
        let pad = " ".repeat(6 - display_width(label));
        text.push_str(&format!("\n  {label}{pad}  {}", file.path));
    }
    if report.dry_run {
        text.push_str("\n\n--dry-run: 아무것도 쓰지 않았습니다.");
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 구간이_없으면_빈_줄을_두고_끝에_붙인다() {
        let text = put_section(Some("# 제목\n"), "본문\n", "A.md").unwrap();
        assert_eq!(
            text,
            "# 제목\n\n<!-- lore:begin v1 -->\n\n본문\n\n<!-- lore:end -->\n"
        );
        assert_eq!(put_section(Some(&text), "본문\n", "A.md").unwrap(), text);
    }

    #[test]
    fn 구간만_바꾸고_밖의_글은_그대로_둔다() {
        let old = "앞\n<!-- lore:begin v0 -->\n옛 본문\n<!-- lore:end -->\n뒤\n";
        assert_eq!(
            put_section(Some(old), "새 본문", "A.md").unwrap(),
            "앞\n<!-- lore:begin v1 -->\n\n새 본문\n\n<!-- lore:end -->\n뒤\n"
        );
    }

    #[test]
    fn 고치지_않은_스킬만_그대로로_본다() {
        for (_, template) in SKILLS {
            let installed = installed_skill(template);
            assert!(installed.contains("\nmetadata:\n  lore-hash: "));
            assert!(skill_untouched(&installed));
            assert!(!skill_untouched(&installed.replace("## 1.", "## 하나.")));
            assert!(!skill_untouched(template));
        }
    }

    #[test]
    fn 구간을_지우면_설치_전의_글로_돌아간다() {
        for original in ["# 제목\n", "", "앞\n\n뒤\n"] {
            let installed = put_section(Some(original), "본문", "A.md").unwrap();
            assert_eq!(
                remove_section(&installed, "A.md").unwrap().as_deref(),
                Some(original),
                "{original:?}"
            );
        }
        let middle = "앞\n\n<!-- lore:begin v1 -->\n\n본문\n\n<!-- lore:end -->\n\n뒤\n";
        assert_eq!(
            remove_section(middle, "A.md").unwrap().as_deref(),
            Some("앞\n\n뒤\n")
        );
        assert_eq!(remove_section("구간 없음\n", "A.md").unwrap(), None);
    }

    #[test]
    fn 깨진_표식은_고치지_않고_오류를_낸다() {
        for text in [
            "<!-- lore:begin v1 -->\n",
            "<!-- lore:end -->\n",
            "<!-- lore:end -->\n<!-- lore:begin v1 -->\n",
            "<!-- lore:begin v1 -->\n<!-- lore:end -->\n<!-- lore:begin v1 -->\n<!-- lore:end -->\n",
        ] {
            assert!(
                matches!(
                    put_section(Some(text), "본문", "A.md"),
                    Err(LoreError::BrokenMarker { .. })
                ),
                "{text}"
            );
        }
    }
}

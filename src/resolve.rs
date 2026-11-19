//! 모듈 경로를 레포 안의 파일로 해석한다. 상대 경로, `tsconfig.json`의 별칭, pnpm 워크스페이스 패키지.
//!
//! 파일 시스템과 git이 추적하는 파일 목록만 본다. 네트워크와 `node_modules`는 보지 않는다.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::rc::Rc;

use globset::{GlobBuilder, GlobSetBuilder};
use serde_json::Value;

use crate::analysis::{is_source, normalize, resolve_index, resolve_path};

/// 모듈 경로 하나를 해석한 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resolution {
    /// 분석하는 코드 파일. 프로젝트 홈 기준 경로다.
    File(String),
    /// CSS, JSON처럼 코드가 아닌 추적 파일. 엣지는 만들지 않는다.
    Asset,
    /// 레포 안을 가리키지만(상대 경로, 별칭, 워크스페이스 패키지) 파일에 잇지 못했다.
    Unresolved,
    /// 레포 밖 패키지. 엣지도 만들지 않고 세지도 않는다.
    External,
}

/// `tsconfig.json`에서 읽은 해석 설정. 경로는 모두 프로젝트 홈 기준이다.
#[derive(Debug, Clone, Default)]
struct TsConfig {
    /// `compilerOptions.baseUrl`
    base_url: Option<String>,
    /// `paths`의 대상이 기대는 폴더. `baseUrl`이 있으면 그것, 없으면 `paths`를 적은 파일의 폴더다.
    paths_base: String,
    /// (패턴, 대상들)
    paths: Vec<(String, Vec<String>)>,
}

/// pnpm 워크스페이스의 패키지 하나.
#[derive(Debug, Clone)]
struct Package {
    name: String,
    /// 패키지 폴더. 프로젝트 홈 기준이다.
    dir: String,
    exports: Option<Value>,
    main: Option<String>,
}

pub struct Resolver<'a> {
    root: &'a Path,
    known: HashSet<&'a str>,
    /// 폴더 → 그 폴더에서 가장 가까운 `tsconfig.json`
    nearest: RefCell<HashMap<String, Option<Rc<TsConfig>>>>,
    packages: Vec<Package>,
}

impl<'a> Resolver<'a> {
    pub fn new(root: &'a Path, tracked: &'a [String], workspace: bool) -> Resolver<'a> {
        let known: HashSet<&str> = tracked.iter().map(String::as_str).collect();
        let packages = if workspace {
            workspace_packages(root, tracked)
        } else {
            Vec::new()
        };
        Resolver {
            root,
            known,
            nearest: RefCell::new(HashMap::new()),
            packages,
        }
    }

    /// `from` 파일에서 `specifier`를 가져올 때 닿는 곳.
    pub fn resolve(&self, from: &str, specifier: &str) -> Resolution {
        let dir = parent(from);
        if is_relative(specifier) {
            return self.relative(&format!("{dir}/{specifier}"), names_folder(specifier));
        }
        if let Some(config) = self.tsconfig_for(dir) {
            if let Some(resolution) = self.alias(&config, specifier) {
                return resolution;
            }
            // `baseUrl`이 있으면 맨 이름도 그 폴더 기준으로 먼저 찾는다.
            if let Some(base) = &config.base_url
                && let Some(path) = normalize(&format!("{base}/{specifier}"))
                && let Some(file) = resolve_path(&path, &self.known)
            {
                return Resolution::File(file);
            }
        }
        if let Some(resolution) = self.package(specifier) {
            return resolution;
        }
        Resolution::External
    }

    fn relative(&self, joined: &str, folder: bool) -> Resolution {
        let Some(path) = normalize(joined) else {
            return Resolution::Unresolved;
        };
        let found = if folder {
            resolve_index(&path, &self.known)
        } else {
            resolve_path(&path, &self.known)
        };
        match found {
            Some(file) => Resolution::File(file),
            None if self.known.contains(path.as_str()) && !is_source(&path) => Resolution::Asset,
            None => Resolution::Unresolved,
        }
    }

    /// `paths`의 패턴 중 맞는 것. TypeScript처럼 글자 그대로 맞는 패턴을 먼저, 그다음 `*` 앞부분이 가장 긴 패턴을 쓴다.
    ///
    /// 패턴이 `*` 하나뿐이면 모든 이름에 맞으므로, 파일에 잇지 못하면 별칭이 아닌 것으로 보고 넘긴다.
    /// TypeScript도 `paths`에서 찾지 못하면 `node_modules`로 넘어간다.
    fn alias(&self, config: &TsConfig, specifier: &str) -> Option<Resolution> {
        let (pattern, targets) = config
            .paths
            .iter()
            .filter(|(pattern, _)| capture(pattern, specifier).is_some())
            .max_by_key(|(pattern, _)| {
                (
                    !pattern.contains('*'),
                    pattern.split('*').next().unwrap_or("").len(),
                )
            })?;
        let captured = capture(pattern, specifier)?;
        let mut asset = false;
        for target in targets {
            let joined = format!(
                "{}/{}",
                config.paths_base,
                target.replacen('*', captured, 1)
            );
            match self.relative(&joined, names_folder(&joined)) {
                Resolution::File(file) => return Some(Resolution::File(file)),
                Resolution::Asset => asset = true,
                _ => {}
            }
        }
        if asset {
            Some(Resolution::Asset)
        } else if pattern == "*" {
            None
        } else {
            Some(Resolution::Unresolved)
        }
    }

    /// 워크스페이스 패키지 이름으로 가져온 것. 패키지 이름이 아니면 `None`이다.
    fn package(&self, specifier: &str) -> Option<Resolution> {
        let package = self
            .packages
            .iter()
            .filter(|package| {
                specifier == package.name || specifier.starts_with(&format!("{}/", package.name))
            })
            .max_by_key(|package| package.name.len())?;
        let rest = &specifier[package.name.len()..];
        let subpath = if rest.is_empty() {
            ".".to_string()
        } else {
            format!(".{rest}")
        };
        let target = package
            .exports
            .as_ref()
            .and_then(|exports| export_target(exports, &subpath))
            .or_else(|| (subpath == ".").then(|| package.main.clone()).flatten())
            .unwrap_or_else(|| subpath.clone());
        let joined = format!("{}/{}", package.dir, target);
        Some(match self.relative(&joined, target.ends_with('/')) {
            Resolution::File(file) => Resolution::File(file),
            Resolution::Asset => Resolution::Asset,
            _ => Resolution::Unresolved,
        })
    }

    /// `dir`에서 위로 올라가며 가장 가까운 `tsconfig.json`. 프로젝트 홈 밖은 보지 않는다.
    fn tsconfig_for(&self, dir: &str) -> Option<Rc<TsConfig>> {
        if let Some(found) = self.nearest.borrow().get(dir) {
            return found.clone();
        }
        let path = if dir.is_empty() {
            "tsconfig.json".to_string()
        } else {
            format!("{dir}/tsconfig.json")
        };
        // git이 추적하는 설정만 읽는다. 읽지 못하면 위 폴더의 설정을 쓴다.
        let loaded = self
            .known
            .contains(path.as_str())
            .then(|| load_tsconfig(self.root, &self.known, &path, 0))
            .flatten();
        let found = match loaded {
            Some(config) => Some(Rc::new(config)),
            None if dir.is_empty() => None,
            None => self.tsconfig_for(parent(dir)),
        };
        self.nearest
            .borrow_mut()
            .insert(dir.to_string(), found.clone());
        found
    }
}

fn parent(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(dir, _)| dir)
}

/// 폴더를 가리키는 경로인지. `/`로 끝나거나 `.`, `..`로 끝나면 같은 이름의 파일이 아니라 폴더의 `index`를 찾는다.
fn names_folder(path: &str) -> bool {
    path.ends_with('/')
        || path == "."
        || path == ".."
        || path.ends_with("/.")
        || path.ends_with("/..")
}

pub(crate) fn is_relative(specifier: &str) -> bool {
    specifier.starts_with("./")
        || specifier.starts_with("../")
        || specifier == "."
        || specifier == ".."
}

/// `prefix*suffix` 꼴 패턴에 맞으면 `*` 자리의 글자. `*`가 없으면 글자 그대로 같을 때 빈 문자열이다.
fn capture<'s>(pattern: &str, specifier: &'s str) -> Option<&'s str> {
    match pattern.split_once('*') {
        None => (pattern == specifier).then_some(""),
        Some((prefix, suffix)) => specifier
            .strip_prefix(prefix)?
            .strip_suffix(suffix)
            .filter(|_| specifier.len() >= prefix.len() + suffix.len()),
    }
}

/// `tsconfig.json`을 읽고 `extends`를 따라간다. 주석과 끝 쉼표가 있어 JSON5로 읽는다.
///
/// `extends`는 레포 안의 상대 경로만 따라가고, 패키지 이름(`@tsconfig/node20` 등)은 무시한다.
/// TypeScript 5의 배열 꼴이면 앞에서부터 읽어 뒤의 것이 앞의 것을 덮는다. git이 추적하는 파일만 읽는다.
fn load_tsconfig(root: &Path, known: &HashSet<&str>, path: &str, depth: usize) -> Option<TsConfig> {
    if depth > 8 || !known.contains(path) {
        return None;
    }
    let text = std::fs::read_to_string(root.join(path)).ok()?;
    let value: Value = json5::from_str(&text).ok()?;
    let dir = parent(path);
    let config_path = |reference: &str, default_file: &str| -> Option<String> {
        let joined = normalize(&format!("{dir}/{reference}"))?;
        Some(if joined.ends_with(".json") {
            joined
        } else if default_file.is_empty() {
            format!("{joined}.json")
        } else {
            format!("{joined}/{default_file}")
        })
    };
    let extends: Vec<&str> = match value.get("extends") {
        Some(Value::String(one)) => vec![one.as_str()],
        Some(Value::Array(many)) => many.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    };
    let mut config = TsConfig::default();
    for parent_config in extends
        .into_iter()
        .filter(|extends| is_relative(extends))
        .filter_map(|extends| config_path(extends, ""))
        .filter_map(|file| load_tsconfig(root, known, &file, depth + 1))
    {
        if parent_config.base_url.is_some() {
            config.base_url = parent_config.base_url.clone();
        }
        if !parent_config.paths.is_empty() {
            config.paths = parent_config.paths.clone();
            config.paths_base = parent_config.paths_base.clone();
        }
    }
    let options = value.get("compilerOptions");
    if let Some(base) = options
        .and_then(|options| options.get("baseUrl"))
        .and_then(Value::as_str)
    {
        config.base_url = normalize(&format!("{dir}/{base}"));
    }
    if let Some(paths) = options
        .and_then(|options| options.get("paths"))
        .and_then(Value::as_object)
    {
        // 자식의 `paths`는 부모의 것을 통째로 바꾼다. TypeScript와 같다.
        config.paths = paths
            .iter()
            .map(|(pattern, targets)| {
                let targets = targets
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect();
                (pattern.clone(), targets)
            })
            .collect();
        config.paths.sort();
        config.paths_base = dir.to_string();
    }
    if let Some(base) = &config.base_url {
        config.paths_base = base.clone();
    }
    // Vite처럼 루트 설정이 `references`만 두고 `paths`를 다른 파일(`tsconfig.app.json` 등)에 둔 경우
    if config.paths.is_empty()
        && let Some(references) = value.get("references").and_then(Value::as_array)
        && let Some(referenced) = references
            .iter()
            .filter_map(|reference| reference.get("path").and_then(Value::as_str))
            .filter_map(|reference| config_path(reference, "tsconfig.json"))
            .filter_map(|file| load_tsconfig(root, known, &file, depth + 1))
            .find(|referenced| !referenced.paths.is_empty())
    {
        config.paths = referenced.paths;
        config.paths_base = referenced.paths_base;
        config.base_url = config.base_url.or(referenced.base_url);
    }
    Some(config)
}

/// `pnpm-workspace.yaml`의 `packages`에 맞는 폴더의 `package.json`.
fn workspace_packages(root: &Path, tracked: &[String]) -> Vec<Package> {
    let Ok(text) = std::fs::read_to_string(root.join("pnpm-workspace.yaml")) else {
        return Vec::new();
    };
    let Ok(yaml) = serde_yaml_ng::from_str::<serde_yaml_ng::Value>(&text) else {
        return Vec::new();
    };
    let patterns: Vec<&str> = yaml
        .get("packages")
        .and_then(|packages| packages.as_sequence())
        .into_iter()
        .flatten()
        .filter_map(|pattern| pattern.as_str())
        .collect();
    let build = |negated: bool| {
        let mut set = GlobSetBuilder::new();
        for pattern in &patterns {
            let (is_negated, pattern) = match pattern.strip_prefix('!') {
                Some(rest) => (true, rest),
                None => (false, *pattern),
            };
            if is_negated == negated
                && let Ok(glob) = GlobBuilder::new(pattern.trim_end_matches('/'))
                    .literal_separator(true)
                    .build()
            {
                set.add(glob);
            }
        }
        set.build().ok()
    };
    let (Some(include), Some(exclude)) = (build(false), build(true)) else {
        return Vec::new();
    };
    let mut packages: Vec<Package> = tracked
        .iter()
        .filter_map(|path| path.strip_suffix("/package.json"))
        .filter(|dir| include.is_match(dir) && !exclude.is_match(dir))
        .filter_map(|dir| {
            let text = std::fs::read_to_string(root.join(format!("{dir}/package.json"))).ok()?;
            let value: Value = serde_json::from_str(&text).ok()?;
            Some(Package {
                name: value.get("name")?.as_str()?.to_string(),
                dir: dir.to_string(),
                exports: value.get("exports").cloned(),
                main: value
                    .get("main")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            })
        })
        .collect();
    packages.sort_by(|a, b| a.name.cmp(&b.name));
    packages
}

/// `exports`에서 `subpath`(`.` 또는 `./x`)가 가리키는 파일. 조건은 `import`, `default`, `require` 순으로 고른다.
fn export_target(exports: &Value, subpath: &str) -> Option<String> {
    match exports {
        Value::String(target) if subpath == "." => Some(target.clone()),
        Value::Object(map) if map.keys().all(|key| !key.starts_with('.')) => {
            (subpath == ".").then(|| conditional(exports)).flatten()
        }
        Value::Object(map) => {
            if let Some(value) = map.get(subpath) {
                return conditional(value);
            }
            // Node처럼 `*` 앞부분이 가장 긴 패턴을 쓴다.
            map.iter()
                .filter_map(|(key, value)| Some((key, capture(key, subpath)?, value)))
                .max_by_key(|(key, _, _)| key.split('*').next().unwrap_or("").len())
                .and_then(|(_, captured, value)| {
                    Some(conditional(value)?.replacen('*', captured, 1))
                })
        }
        _ => None,
    }
}

/// 조건부 내보내기의 대상. 조건은 `import`, `default`, `require` 순으로 고르고 안쪽 조건까지 따라간다.
/// `types`는 선언 파일이라 쓰지 않고, `null`은 내보내지 않는다는 뜻이다. 배열이면 처음 것을 쓴다.
fn conditional(value: &Value) -> Option<String> {
    match value {
        Value::String(target) => Some(target.clone()),
        Value::Object(map) => ["import", "default", "require"]
            .iter()
            .filter_map(|key| map.get(*key))
            .find_map(conditional),
        Value::Array(items) => items.iter().find_map(conditional),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_all(root: &Path, files: &[(&str, &str)]) -> Vec<String> {
        for (path, text) in files {
            let file = root.join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, text).unwrap();
        }
        files.iter().map(|(path, _)| path.to_string()).collect()
    }

    #[test]
    fn references와_배열_extends의_paths를_읽고_별표_하나뿐인_별칭은_넘긴다() {
        let dir = tempfile::tempdir().unwrap();
        let tracked = write_all(
            dir.path(),
            &[
                (
                    "tsconfig.json",
                    r#"{"files": [], "references": [{"path": "./tsconfig.app.json"}]}"#,
                ),
                (
                    "tsconfig.app.json",
                    r#"{"extends": ["./base-a", "./base-b.json"]}"#,
                ),
                (
                    "base-a.json",
                    r#"{"compilerOptions": {"paths": {"@/*": ["./src/*"]}}}"#,
                ),
                (
                    "base-b.json",
                    r#"{"compilerOptions": {"paths": {"~/*": ["./src/*"], "*": ["./types/*"]}}}"#,
                ),
                ("src/lib/index.ts", ""),
                ("src/lib.ts", ""),
                ("src/main.ts", ""),
            ],
        );
        let resolver = Resolver::new(dir.path(), &tracked, false);
        // 배열의 뒤 설정(base-b)이 앞 설정의 paths를 덮는다.
        assert_eq!(
            resolver.resolve("src/main.ts", "~/lib"),
            Resolution::File("src/lib.ts".to_string())
        );
        assert_eq!(
            resolver.resolve("src/main.ts", "~/lib/"),
            Resolution::File("src/lib/index.ts".to_string())
        );
        assert_eq!(
            resolver.resolve("src/main.ts", "@/lib"),
            Resolution::External
        );
        // `*` 하나뿐인 패턴에 맞았지만 파일이 없으면 레포 밖 패키지로 본다.
        assert_eq!(
            resolver.resolve("src/main.ts", "react"),
            Resolution::External
        );
    }

    #[test]
    fn git이_추적하지_않거나_깨진_tsconfig는_위_폴더의_것을_쓴다() {
        let dir = tempfile::tempdir().unwrap();
        let tracked = write_all(
            dir.path(),
            &[
                (
                    "tsconfig.json",
                    r#"{"compilerOptions": {"paths": {"@/*": ["./src/*"]}}}"#,
                ),
                ("app/tsconfig.json", "{ 깨진 설정"),
                ("src/a.ts", ""),
                ("app/main.ts", ""),
            ],
        );
        let resolver = Resolver::new(dir.path(), &tracked, false);
        assert_eq!(
            resolver.resolve("app/main.ts", "@/a"),
            Resolution::File("src/a.ts".to_string())
        );
    }

    #[test]
    fn 패턴의_별표_자리를_잡는다() {
        assert_eq!(capture("@/*", "@/features/note"), Some("features/note"));
        assert_eq!(capture("@core", "@core"), Some(""));
        assert_eq!(capture("@core", "@core/x"), None);
        assert_eq!(capture("lib/*.js", "lib/a.js"), Some("a"));
        assert_eq!(capture("ab*ba", "aba"), None);
    }

    #[test]
    fn exports에서_하위_경로와_조건을_고른다() {
        let exports: Value = serde_json::json!({
            ".": {"types": "./dist/index.d.ts", "import": "./src/index.ts"},
            "./utils/*": "./src/utils/*.ts",
        });
        assert_eq!(
            export_target(&exports, ".").as_deref(),
            Some("./src/index.ts")
        );
        assert_eq!(
            export_target(&exports, "./utils/round").as_deref(),
            Some("./src/utils/round.ts")
        );
        assert_eq!(
            export_target(&serde_json::json!("./main.ts"), ".").as_deref(),
            Some("./main.ts")
        );
        assert_eq!(
            export_target(&serde_json::json!({"default": "./a.ts"}), ".").as_deref(),
            Some("./a.ts")
        );
        // 안쪽 조건과, 더 구체적인 패턴
        let exports: Value = serde_json::json!({
            ".": {"import": {"types": "./a.d.ts", "default": "./src/index.ts"}},
            "./*": "./src/*.ts",
            "./utils/*": "./src/utils/*.js",
        });
        assert_eq!(
            export_target(&exports, ".").as_deref(),
            Some("./src/index.ts")
        );
        assert_eq!(
            export_target(&exports, "./utils/u").as_deref(),
            Some("./src/utils/u.js")
        );
        assert_eq!(export_target(&serde_json::json!({".": null}), "."), None);
    }
}

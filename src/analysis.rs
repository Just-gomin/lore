//! 정적 분석: JavaScript와 TypeScript 파일의 import를 읽어 파일 → 파일 의존을 만든다.
//!
//! tree-sitter로 파싱하며 네트워크를 쓰지 않는다. 같은 파일 목록과 내용에서는 같은 결과를 낸다.
//! 분석은 추론이라, 문법 오류가 있는 파일과 해석하지 못한 경로는 빌드를 막지 않고 보고만 한다.

use std::collections::{BTreeSet, HashSet};
use std::path::Path;

use serde::Serialize;
use tree_sitter::{Language, Node, Parser};

/// 분석하는 확장자. `.d.ts`는 선언만 있어 뺀다.
const EXTENSIONS: [&str; 8] = ["ts", "tsx", "mts", "cts", "js", "jsx", "mjs", "cjs"];

/// 분석 결과. `lore build` 출력의 `analysis`가 된다.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct AnalysisReport {
    /// 분석한 파일 수.
    pub files: usize,
    /// 만든 파일 → 파일 의존 엣지 수.
    pub edges: usize,
    /// 문법 오류가 있거나 읽지 못한 파일. 오류가 난 곳 밖의 import는 그대로 쓴다.
    /// 문법 크레이트가 유효한 코드를 오류로 읽는 일(JSX 글 속의 `&` 등)이 있어, 파일 전체를 버리지 않는다.
    pub parse_errors: Vec<String>,
    /// 레포 안을 가리키지만 파일에 잇지 못한 import 수. 레포 밖 패키지는 세지 않는다.
    pub unresolved: usize,
}

/// 분석한 파일과 파일 → 파일 의존. 둘 다 정렬되어 있다.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Analysis {
    pub files: Vec<String>,
    /// (import하는 파일, import되는 파일)
    pub edges: Vec<(String, String)>,
    pub report: AnalysisReport,
}

/// 분석할 파일인지. 프로젝트 홈 기준 경로다.
pub fn is_source(path: &str) -> bool {
    if path.ends_with(".d.ts") || path.ends_with(".d.mts") || path.ends_with(".d.cts") {
        return false;
    }
    extension(path).is_some_and(|ext| EXTENSIONS.contains(&ext))
}

fn extension(path: &str) -> Option<&str> {
    let name = path.rsplit('/').next()?;
    let (_, ext) = name.rsplit_once('.')?;
    Some(ext)
}

/// git이 추적하는 파일 중 분석할 파일의 import를 읽는다.
pub fn analyze(root: &Path, tracked: &[String]) -> Analysis {
    let files: Vec<String> = tracked
        .iter()
        .filter(|path| is_source(path))
        .cloned()
        .collect();
    let known: HashSet<&str> = tracked.iter().map(String::as_str).collect();
    let mut report = AnalysisReport {
        files: files.len(),
        ..AnalysisReport::default()
    };
    let mut edges = BTreeSet::new();
    let mut parser = Parser::new();
    for file in &files {
        let Ok(text) = std::fs::read_to_string(root.join(file)) else {
            report.parse_errors.push(file.clone());
            continue;
        };
        let Some((specifiers, had_error)) = module_specifiers(&mut parser, file, &text) else {
            report.parse_errors.push(file.clone());
            continue;
        };
        if had_error {
            report.parse_errors.push(file.clone());
        }
        for specifier in specifiers {
            match classify(&specifier) {
                Specifier::Relative => match resolve_relative(file, &specifier, &known) {
                    Some(target) if target != *file => {
                        edges.insert((file.clone(), target));
                    }
                    Some(_) => {}
                    // CSS, JSON, 이미지처럼 코드가 아닌 추적 파일을 가져온 것은 해석한 것으로 보고 엣지는 만들지 않는다.
                    None if is_tracked_asset(file, &specifier, &known) => {}
                    None => report.unresolved += 1,
                },
                // 별칭과 워크스페이스 패키지는 2번에서 해석한다. 지금은 레포 밖 패키지와 같이 본다.
                Specifier::Bare => {}
            }
        }
    }
    report.edges = edges.len();
    Analysis {
        files,
        edges: edges.into_iter().collect(),
        report,
    }
}

fn language(path: &str) -> Language {
    match extension(path) {
        Some("tsx") => tree_sitter_typescript::LANGUAGE_TSX.into(),
        Some("ts" | "mts" | "cts") => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        _ => tree_sitter_javascript::LANGUAGE.into(),
    }
}

/// 파일이 가져오는 모듈 경로와 문법 오류가 있었는지. 파서가 아무것도 내지 못하면 `None`이다.
///
/// 정적 import, `export ... from`, 문자열 상수를 받는 `require()`와 `import()`를 읽는다.
/// 문법 오류로 읽은 곳(`ERROR` 노드) 안은 보지 않는다.
fn module_specifiers(parser: &mut Parser, path: &str, text: &str) -> Option<(Vec<String>, bool)> {
    parser.set_language(&language(path)).ok()?;
    let tree = parser.parse(text, None)?;
    let root = tree.root_node();
    let mut found = Vec::new();
    collect(root, text.as_bytes(), &mut found);
    Some((found, root.has_error()))
}

fn collect(node: Node, source: &[u8], found: &mut Vec<String>) {
    if node.is_error() {
        return;
    }
    match node.kind() {
        "import_statement" | "export_statement" => {
            if let Some(value) = node
                .child_by_field_name("source")
                .and_then(|string| string_value(string, source))
            {
                found.push(value);
            }
        }
        "call_expression" => {
            let callee = node.child_by_field_name("function");
            let is_loader = callee.is_some_and(|callee| {
                callee.kind() == "import"
                    || (callee.kind() == "identifier"
                        && callee.utf8_text(source).ok() == Some("require"))
            });
            if is_loader
                && let Some(args) = node.child_by_field_name("arguments")
                && args.named_child_count() == 1
                && let Some(value) = args
                    .named_child(0)
                    .and_then(|arg| string_value(arg, source))
            {
                found.push(value);
            }
        }
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect(child, source, found);
    }
}

/// 문자열 리터럴의 값. 템플릿 문자열이나 이어 붙인 식은 상수가 아니라 `None`이다.
fn string_value(node: Node, source: &[u8]) -> Option<String> {
    if node.kind() != "string" {
        return None;
    }
    let mut cursor = node.walk();
    let value: String = node
        .named_children(&mut cursor)
        .filter(|child| child.kind() == "string_fragment")
        .filter_map(|child| child.utf8_text(source).ok())
        .collect();
    (!value.is_empty()).then_some(value)
}

enum Specifier {
    /// `./`, `../`로 시작
    Relative,
    /// 그 밖의 것. 별칭, 워크스페이스 패키지, 레포 밖 패키지
    Bare,
}

fn classify(specifier: &str) -> Specifier {
    if specifier.starts_with("./") || specifier.starts_with("../") || specifier == "." {
        Specifier::Relative
    } else {
        Specifier::Bare
    }
}

/// 상대 경로를 git이 추적하는 파일에 잇는다. TypeScript의 `bundler` 해석(`allowJs`)과 같은 규칙이다.
fn resolve_relative(from: &str, specifier: &str, known: &HashSet<&str>) -> Option<String> {
    let dir = from.rsplit_once('/').map_or("", |(dir, _)| dir);
    let joined = normalize(&format!("{dir}/{specifier}"))?;
    resolve_path(&joined, known)
}

fn is_tracked_asset(from: &str, specifier: &str, known: &HashSet<&str>) -> bool {
    let dir = from.rsplit_once('/').map_or("", |(dir, _)| dir);
    normalize(&format!("{dir}/{specifier}")).is_some_and(|path| known.contains(path.as_str()))
}

/// 확장자를 보충하고 폴더의 `index`를 찾아 파일에 잇는다.
pub(crate) fn resolve_path(path: &str, known: &HashSet<&str>) -> Option<String> {
    const BARE: [&str; 4] = [".ts", ".tsx", ".js", ".jsx"];
    if known.contains(path) && is_source(path) {
        return Some(path.to_string());
    }
    // `.js`로 적은 경로가 실제로는 `.ts` 파일인 경우
    let swaps: &[(&str, &[&str])] = &[
        (".js", &[".ts", ".tsx"]),
        (".jsx", &[".tsx"]),
        (".mjs", &[".mts"]),
        (".cjs", &[".cts"]),
    ];
    for (written, actual) in swaps {
        if let Some(stem) = path.strip_suffix(written) {
            for ext in *actual {
                let candidate = format!("{stem}{ext}");
                if known.contains(candidate.as_str()) {
                    return Some(candidate);
                }
            }
        }
    }
    for ext in BARE {
        let candidate = format!("{path}{ext}");
        if known.contains(candidate.as_str()) {
            return Some(candidate);
        }
    }
    let base = path.trim_end_matches('/');
    for ext in BARE {
        let candidate = if base.is_empty() {
            format!("index{ext}")
        } else {
            format!("{base}/index{ext}")
        };
        if known.contains(candidate.as_str()) {
            return Some(candidate);
        }
    }
    None
}

/// `.`과 `..`를 풀어 프로젝트 홈 기준 경로로 만든다. 홈 밖으로 나가면 `None`이다.
fn normalize(path: &str) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            part => parts.push(part),
        }
    }
    Some(parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn known(files: &[&'static str]) -> HashSet<&'static str> {
        files.iter().copied().collect()
    }

    #[test]
    fn 상대_경로는_bundler_해석대로_파일에_잇는다() {
        let files = known(&[
            "src/a.ts",
            "src/b.tsx",
            "src/lib/index.ts",
            "src/util.js",
            "src/types.ts",
            "src/m.mts",
        ]);
        let resolve = |from: &str, spec: &str| resolve_relative(from, spec, &files);
        assert_eq!(resolve("src/x.ts", "./a").as_deref(), Some("src/a.ts"));
        assert_eq!(resolve("src/x.ts", "./b").as_deref(), Some("src/b.tsx"));
        assert_eq!(
            resolve("src/x.ts", "./lib").as_deref(),
            Some("src/lib/index.ts")
        );
        assert_eq!(resolve("src/lib/y.ts", "..").as_deref(), None);
        assert_eq!(
            resolve("src/x.ts", "./util.js").as_deref(),
            Some("src/util.js")
        );
        assert_eq!(
            resolve("src/x.ts", "./types.js").as_deref(),
            Some("src/types.ts")
        );
        assert_eq!(resolve("src/x.ts", "./m.mjs").as_deref(), Some("src/m.mts"));
        assert_eq!(resolve("src/lib/y.ts", "../a").as_deref(), Some("src/a.ts"));
        assert_eq!(resolve("src/x.ts", "../../../etc"), None);
        assert_eq!(resolve("src/x.ts", "./none"), None);
    }

    #[test]
    fn 선언_파일은_분석하지_않는다() {
        assert!(is_source("a/b.ts"));
        assert!(is_source("a/b.cjs"));
        assert!(!is_source("a/b.d.ts"));
        assert!(!is_source("a/b.css"));
        assert!(!is_source("Makefile"));
    }

    #[test]
    fn import와_require와_동적_import의_상수_경로만_읽는다() {
        let mut parser = Parser::new();
        let text = "import a from './a';\nimport type { T } from './t';\nexport * from './b';\nexport { c } from './c';\nconst d = require('./d');\nconst e = await import('./e');\nconst f = import(`./${x}`);\nrequire(name);\nexport const g = 1;\n";
        let (found, had_error) = module_specifiers(&mut parser, "x.ts", text).unwrap();
        assert_eq!(found, ["./a", "./t", "./b", "./c", "./d", "./e"]);
        assert!(!had_error);
    }

    #[test]
    fn 문법_오류가_있어도_오류_밖의_import는_읽는다() {
        let mut parser = Parser::new();
        // JSX 글 속의 맨 `&`를 이 문법은 오류로 읽는다.
        let text = "import { A } from './a';\nexport const B = () => <p>WHEN & WHERE</p>;\n";
        let (found, had_error) = module_specifiers(&mut parser, "x.tsx", text).unwrap();
        assert_eq!(found, ["./a"]);
        assert!(had_error);
    }
}

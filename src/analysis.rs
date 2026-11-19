//! 정적 분석: JavaScript와 TypeScript 파일의 import를 읽어 파일 → 파일 의존을 만든다.
//!
//! tree-sitter로 파싱하며 네트워크를 쓰지 않는다. 같은 파일 목록과 내용에서는 같은 결과를 낸다.
//! 분석은 추론이라, 문법 오류가 있는 파일과 해석하지 못한 경로는 빌드를 막지 않고 보고만 한다.

use std::collections::{BTreeSet, HashSet};
use std::path::Path;

use serde::Serialize;
use tree_sitter::{Language, Node, Parser};

use crate::resolve::{Resolution, Resolver};

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

/// git이 추적하는 파일 중 분석할 파일의 import를 읽는다. `workspace`면 pnpm 워크스페이스 패키지도 해석한다.
pub fn analyze(root: &Path, tracked: &[String], workspace: bool) -> Analysis {
    let files: Vec<String> = tracked
        .iter()
        .filter(|path| is_source(path))
        .cloned()
        .collect();
    let resolver = Resolver::new(root, tracked, workspace);
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
            match resolver.resolve(file, &specifier) {
                Resolution::File(target) if target != *file => {
                    edges.insert((file.clone(), target));
                }
                // CSS, JSON, 이미지처럼 코드가 아닌 추적 파일과 레포 밖 패키지는 엣지를 만들지 않는다.
                Resolution::File(_) | Resolution::Asset | Resolution::External => {}
                Resolution::Unresolved => report.unresolved += 1,
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

/// 트리를 돌며 모듈 경로를 모은다. 깊이 중첩된 코드(긴 문자열 이어 붙이기 등)에서 스택이 넘치지 않게
/// 재귀 대신 커서로 돈다. 문법 오류로 읽은 곳(`ERROR`) 안으로는 들어가지 않는다.
fn collect(root: Node, source: &[u8], found: &mut Vec<String>) {
    let mut cursor = root.walk();
    loop {
        let node = cursor.node();
        let enter = !node.is_error();
        if enter {
            found.extend(specifier_of(node, source));
        }
        if enter && cursor.goto_first_child() {
            continue;
        }
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                return;
            }
        }
    }
}

/// 노드 하나가 가져오는 모듈 경로.
fn specifier_of(node: Node, source: &[u8]) -> Option<String> {
    match node.kind() {
        "import_statement" | "export_statement" => {
            // TypeScript의 `import x = require("y")`는 경로가 `import_require_clause` 안에 있다.
            let string = node.child_by_field_name("source").or_else(|| {
                let mut cursor = node.walk();
                node.named_children(&mut cursor)
                    .find(|child| child.kind() == "import_require_clause")
                    .and_then(|clause| clause.child_by_field_name("source"))
            });
            string.and_then(|string| string_value(string, source))
        }
        "call_expression" => {
            let callee = node.child_by_field_name("function")?;
            let is_loader = callee.kind() == "import"
                || (callee.kind() == "identifier"
                    && callee.utf8_text(source).ok() == Some("require"));
            let args = node.child_by_field_name("arguments")?;
            if !is_loader || args.named_child_count() != 1 {
                return None;
            }
            args.named_child(0)
                .and_then(|arg| string_value(arg, source))
        }
        _ => None,
    }
}

/// 문자열 리터럴의 값. 템플릿 문자열이나 이어 붙인 식은 상수가 아니라 `None`이다.
fn string_value(node: Node, source: &[u8]) -> Option<String> {
    if node.kind() != "string" {
        return None;
    }
    let mut cursor = node.walk();
    let children: Vec<Node> = node.named_children(&mut cursor).collect();
    // 이스케이프가 든 경로는 풀지 않고 버린다. 글자 그대로 이으면 다른 파일을 가리키게 된다.
    if children
        .iter()
        .any(|child| child.kind() != "string_fragment")
    {
        return None;
    }
    let value: String = children
        .iter()
        .filter_map(|child| child.utf8_text(source).ok())
        .collect();
    (!value.is_empty()).then_some(value)
}

const BARE: [&str; 4] = [".ts", ".tsx", ".js", ".jsx"];

/// 폴더의 `index` 파일.
pub(crate) fn resolve_index(dir: &str, known: &HashSet<&str>) -> Option<String> {
    BARE.iter()
        .map(|ext| {
            if dir.is_empty() {
                format!("index{ext}")
            } else {
                format!("{dir}/index{ext}")
            }
        })
        .find(|candidate| known.contains(candidate.as_str()))
}

/// 확장자를 보충하고 폴더의 `index`를 찾아 파일에 잇는다.
pub(crate) fn resolve_path(path: &str, known: &HashSet<&str>) -> Option<String> {
    // 분석하는 파일만 잇는다. `.d.ts`처럼 분석하지 않는 파일로 가면 노드가 없는 엣지가 된다.
    let found = |candidate: &str| known.contains(candidate) && is_source(candidate);
    if found(path) {
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
                if found(&candidate) {
                    return Some(candidate);
                }
            }
        }
    }
    for ext in BARE {
        let candidate = format!("{path}{ext}");
        if found(&candidate) {
            return Some(candidate);
        }
    }
    resolve_index(path, known)
}

/// `.`과 `..`를 풀어 프로젝트 홈 기준 경로로 만든다. 홈 밖으로 나가면 `None`이다.
pub(crate) fn normalize(path: &str) -> Option<String> {
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

    fn resolve_in(tracked: &[&str], from: &str, spec: &str) -> Option<String> {
        let tracked: Vec<String> = tracked.iter().map(|path| path.to_string()).collect();
        let resolver = Resolver::new(Path::new("/nonexistent"), &tracked, false);
        match resolver.resolve(from, spec) {
            Resolution::File(file) => Some(file),
            _ => None,
        }
    }

    #[test]
    fn 상대_경로는_bundler_해석대로_파일에_잇는다() {
        let files = [
            "src/a.ts",
            "src/b.tsx",
            "src/lib/index.ts",
            "src/util.js",
            "src/types.ts",
            "src/m.mts",
        ];
        let resolve = |from: &str, spec: &str| resolve_in(&files, from, spec);
        assert_eq!(resolve("src/x.ts", "./a").as_deref(), Some("src/a.ts"));
        assert_eq!(resolve("src/x.ts", "./b").as_deref(), Some("src/b.tsx"));
        assert_eq!(
            resolve("src/x.ts", "./lib").as_deref(),
            Some("src/lib/index.ts")
        );
        assert_eq!(resolve("src/lib/y.ts", "..").as_deref(), None);
        assert_eq!(
            resolve("src/lib/y.ts", ".").as_deref(),
            Some("src/lib/index.ts")
        );
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
        // `/`로 끝나면 같은 이름의 파일보다 폴더의 index를 찾는다.
        let files = ["src/lib.ts", "src/lib/index.ts", "src/types.d.ts"];
        assert_eq!(
            resolve_in(&files, "src/x.ts", "./lib/").as_deref(),
            Some("src/lib/index.ts")
        );
        assert_eq!(
            resolve_in(&files, "src/x.ts", "./lib").as_deref(),
            Some("src/lib.ts")
        );
        // 분석하지 않는 선언 파일로는 잇지 않는다.
        assert_eq!(resolve_in(&files, "src/x.ts", "./types.d"), None);
        // `..`도 상대 경로다. `src/lib/inner`에서 `..`은 `src/lib`이고, 같은 이름의 파일이 먼저다.
        assert_eq!(
            resolve_in(&files, "src/lib/inner/y.ts", "..").as_deref(),
            Some("src/lib.ts")
        );
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

        let text = "import x = require('./x');\nconst y = require('./li\\x62');\n";
        let (found, _) = module_specifiers(&mut parser, "y.ts", text).unwrap();
        assert_eq!(found, ["./x"]);
    }

    #[test]
    fn 깊이_중첩된_코드에서도_스택이_넘치지_않는다() {
        let mut parser = Parser::new();
        let mut text = String::from("import a from './a';\nconst s = \"x\"");
        for _ in 0..50_000 {
            text.push_str(" + \"x\"");
        }
        text.push_str(";\n");
        let (found, _) = module_specifiers(&mut parser, "deep.js", &text).unwrap();
        assert_eq!(found, ["./a"]);
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

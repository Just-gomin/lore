//! 심볼: 파일이 내보내는 이름과, 최상위 선언이 가져다 쓰는 이름.
//!
//! 한 파일의 구문 트리에서 정보를 뽑는 일([`extract`])과, 파일들을 이어 심볼 → 심볼 의존을 만드는 일([`link`])로 나뉜다.
//! 타입 검사는 하지 않는다. 가져온 이름을 어느 최상위 선언이 쓰는지까지만 본다.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use tree_sitter::Node;

/// 한 파일에서 뽑은 심볼 정보.
#[derive(Debug, Clone, Default)]
pub struct FileSymbols {
    /// 이 파일에서 정의해 내보내는 이름. 내보낸 이름 → 그 선언의 지역 이름(같은 선언을 다른 이름으로 내보낼 수 있다).
    pub defined: BTreeMap<String, String>,
    /// 최상위 클래스의 멤버. `클래스의 지역 이름.method`. 클래스를 내보냈을 때만 심볼이 된다.
    pub members: BTreeSet<String>,
    /// `export { a as b } from "./x"`: 내보낸 이름 → (모듈 경로, 가져온 이름). `export * as ns`는 가져온 이름이 `*`
    pub reexports: BTreeMap<String, (String, String)>,
    /// `export * from "./x"`의 모듈 경로
    pub stars: Vec<String>,
    /// `export { a as b }`처럼 가져온 이름을 그대로 다시 내보낸 것: 내보낸 이름 → 지역 이름
    pub local_exports: BTreeMap<String, String>,
    /// 가져온 이름: 지역 이름 → (모듈 경로, 가져온 이름). 이름 전체(`* as ns`)는 `*`
    pub imports: BTreeMap<String, (String, String)>,
    /// 최상위 선언마다 쓰는 지역 이름. 선언의 지역 이름(없으면 파일 수준) → 쓰는 이름들
    pub uses: BTreeMap<Option<String>, BTreeSet<Use>>,
}

/// 쓰인 이름 하나. `ns.member`처럼 이름 전체를 가져와 쓴 경우 `member`가 있다.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Use {
    pub local: String,
    pub member: Option<String>,
}

fn text<'a>(node: Node, source: &'a [u8]) -> &'a str {
    node.utf8_text(source).unwrap_or("")
}

fn string_of(node: Option<Node>, source: &[u8]) -> Option<String> {
    let node = node?;
    if node.kind() != "string" {
        return None;
    }
    let mut cursor = node.walk();
    let children: Vec<Node> = node.named_children(&mut cursor).collect();
    if children
        .iter()
        .any(|child| child.kind() != "string_fragment")
    {
        return None;
    }
    let value: String = children.iter().map(|child| text(*child, source)).collect();
    (!value.is_empty()).then_some(value)
}

/// 구문 트리의 최상위 문장을 읽는다.
pub fn extract(root: Node, source: &[u8]) -> FileSymbols {
    let mut symbols = FileSymbols::default();
    let mut cursor = root.walk();
    for statement in root.named_children(&mut cursor) {
        if statement.is_error() {
            continue;
        }
        match statement.kind() {
            "import_statement" => read_import(statement, source, &mut symbols),
            "export_statement" => read_export(statement, source, &mut symbols),
            _ => {
                let statement = unwrap_ambient(statement);
                let names = declared_names(statement, source);
                if let Some(class) = names.first() {
                    record_members(statement, source, class, &mut symbols);
                }
                record_uses(statement, source, &names, &mut symbols);
            }
        }
    }
    // 가져온 이름만 쓴 것으로 남긴다.
    let imports = symbols.imports.clone();
    for uses in symbols.uses.values_mut() {
        uses.retain(|used| imports.contains_key(&used.local));
    }
    symbols.uses.retain(|_, uses| !uses.is_empty());
    symbols
}

fn read_import(statement: Node, source: &[u8], symbols: &mut FileSymbols) {
    let Some(module) = string_of(statement.child_by_field_name("source"), source) else {
        return;
    };
    let mut cursor = statement.walk();
    let Some(clause) = statement
        .named_children(&mut cursor)
        .find(|child| child.kind() == "import_clause")
    else {
        return;
    };
    let mut cursor = clause.walk();
    for part in clause.named_children(&mut cursor) {
        match part.kind() {
            "identifier" => {
                symbols.imports.insert(
                    text(part, source).to_string(),
                    (module.clone(), "default".to_string()),
                );
            }
            "namespace_import" => {
                let mut inner = part.walk();
                if let Some(name) = part
                    .named_children(&mut inner)
                    .find(|child| child.kind() == "identifier")
                {
                    symbols.imports.insert(
                        text(name, source).to_string(),
                        (module.clone(), "*".to_string()),
                    );
                }
            }
            "named_imports" => {
                let mut inner = part.walk();
                for spec in part
                    .named_children(&mut inner)
                    .filter(|child| child.kind() == "import_specifier")
                {
                    let Some(name) = spec.child_by_field_name("name") else {
                        continue;
                    };
                    let local = spec.child_by_field_name("alias").unwrap_or(name);
                    symbols.imports.insert(
                        text(local, source).to_string(),
                        (module.clone(), text(name, source).to_string()),
                    );
                }
            }
            _ => {}
        }
    }
}

fn read_export(statement: Node, source: &[u8], symbols: &mut FileSymbols) {
    let mut cursor = statement.walk();
    let children: Vec<Node> = statement.children(&mut cursor).collect();
    let is_default = children.iter().any(|child| child.kind() == "default");
    let module = string_of(statement.child_by_field_name("source"), source);
    let clause = children
        .iter()
        .find(|child| child.kind() == "export_clause")
        .copied();

    if let Some(module) = module {
        match clause {
            // export { a as b } from "./x"
            Some(clause) => {
                for (name, exported) in export_specifiers(clause, source) {
                    symbols.reexports.insert(exported, (module.clone(), name));
                }
            }
            None => match children
                .iter()
                .find(|child| child.kind() == "namespace_export")
            {
                // export * as ns from "./x"
                Some(namespace) => {
                    let mut inner = namespace.walk();
                    if let Some(name) = namespace.named_children(&mut inner).next() {
                        let name = string_of(Some(name), source)
                            .unwrap_or_else(|| text(name, source).to_string());
                        symbols.reexports.insert(name, (module, "*".to_string()));
                    }
                }
                // export * from "./x"
                None => symbols.stars.push(module),
            },
        }
        return;
    }
    if let Some(clause) = clause {
        // export { a as b }
        for (name, exported) in export_specifiers(clause, source) {
            symbols.local_exports.insert(exported, name);
        }
        return;
    }
    if let Some(declaration) = statement.child_by_field_name("declaration") {
        let declaration = unwrap_ambient(declaration);
        let mut names = declared_names(declaration, source);
        if is_default {
            // 이름 없는 `export default function () {}`는 지역 이름도 `default`로 둔다.
            if names.is_empty() {
                names.push("default".to_string());
            }
            symbols
                .defined
                .insert("default".to_string(), names[0].clone());
        } else {
            for name in &names {
                symbols.defined.insert(name.clone(), name.clone());
            }
        }
        if let Some(class) = names.first() {
            record_members(declaration, source, class, symbols);
        }
        record_uses(declaration, source, &names, symbols);
        return;
    }
    // export default <식>, 그리고 TypeScript의 `export = <식>`도 기본 내보내기로 본다.
    let value = statement.child_by_field_name("value").or_else(|| {
        children
            .iter()
            .any(|child| child.kind() == "=")
            .then(|| {
                children
                    .iter()
                    .find(|child| child.is_named() && child.kind() != "decorator")
                    .copied()
            })
            .flatten()
    });
    if let Some(value) = value {
        // export default <식>. 식이 지역 이름 하나면 그 선언을 `default`로도 내보낸다.
        if value.kind() == "identifier" {
            symbols
                .local_exports
                .insert("default".to_string(), text(value, source).to_string());
        } else {
            symbols
                .defined
                .insert("default".to_string(), "default".to_string());
            // 이름 없는 `export default class {}`
            record_members(value, source, "default", symbols);
            record_uses(value, source, &["default".to_string()], symbols);
        }
    }
}

fn export_specifiers(clause: Node, source: &[u8]) -> Vec<(String, String)> {
    let mut cursor = clause.walk();
    clause
        .named_children(&mut cursor)
        .filter(|child| child.kind() == "export_specifier")
        .filter_map(|spec| {
            let name = text(spec.child_by_field_name("name")?, source).to_string();
            let exported = spec
                .child_by_field_name("alias")
                .map_or_else(|| name.clone(), |alias| text(alias, source).to_string());
            Some((name, exported))
        })
        .collect()
}

/// `declare`로 감싼 선언은 안의 선언을 본다.
fn unwrap_ambient(node: Node) -> Node {
    if node.kind() != "ambient_declaration" {
        return node;
    }
    let mut cursor = node.walk();
    let inner = node.named_children(&mut cursor).next();
    inner.unwrap_or(node)
}

/// 최상위 선언이 만드는 지역 이름. 함수, 클래스, 변수(구조 분해로 여럿일 수 있음), 인터페이스, 타입, 열거형, 네임스페이스.
fn declared_names(declaration: Node, source: &[u8]) -> Vec<String> {
    match declaration.kind() {
        "function_declaration"
        | "generator_function_declaration"
        | "function_signature"
        | "class_declaration"
        | "abstract_class_declaration"
        | "interface_declaration"
        | "type_alias_declaration"
        | "enum_declaration"
        | "internal_module"
        | "module" => declaration
            .child_by_field_name("name")
            .filter(|name| matches!(name.kind(), "identifier" | "type_identifier"))
            .map(|name| vec![text(name, source).to_string()])
            .unwrap_or_default(),
        "lexical_declaration" | "variable_declaration" => {
            let mut names = Vec::new();
            let mut cursor = declaration.walk();
            for declarator in declaration
                .named_children(&mut cursor)
                .filter(|child| child.kind() == "variable_declarator")
            {
                if let Some(pattern) = declarator.child_by_field_name("name") {
                    pattern_names(pattern, source, &mut names);
                }
            }
            names
        }
        _ => Vec::new(),
    }
}

/// 변수 선언의 이름 자리(`{ a, b: [c] }` 같은 구조 분해 포함)가 만드는 지역 이름.
fn pattern_names(pattern: Node, source: &[u8], names: &mut Vec<String>) {
    match pattern.kind() {
        "identifier" | "shorthand_property_identifier_pattern" => {
            names.push(text(pattern, source).to_string());
        }
        "pair_pattern" => {
            if let Some(value) = pattern.child_by_field_name("value") {
                pattern_names(value, source, names);
            }
        }
        "assignment_pattern" | "object_assignment_pattern" => {
            if let Some(left) = pattern.child_by_field_name("left") {
                pattern_names(left, source, names);
            }
        }
        "object_pattern" | "array_pattern" | "rest_pattern" => {
            let mut cursor = pattern.walk();
            for child in pattern.named_children(&mut cursor) {
                pattern_names(child, source, names);
            }
        }
        _ => {}
    }
}

/// 클래스의 공개 메서드. `#private`, `private`, 계산된 이름(`[Symbol.iterator]`)은 뺀다.
fn record_members(declaration: Node, source: &[u8], class: &str, symbols: &mut FileSymbols) {
    if !matches!(
        declaration.kind(),
        "class_declaration" | "abstract_class_declaration" | "class"
    ) {
        return;
    }
    let Some(body) = declaration.child_by_field_name("body") else {
        return;
    };
    let mut cursor = body.walk();
    for member in body.named_children(&mut cursor) {
        if !matches!(
            member.kind(),
            "method_definition" | "abstract_method_signature" | "method_signature"
        ) {
            continue;
        }
        let Some(method) = member.child_by_field_name("name") else {
            continue;
        };
        let mut inner = member.walk();
        let private = member.named_children(&mut inner).any(|child| {
            child.kind() == "accessibility_modifier" && text(child, source) == "private"
        });
        if method.kind() == "property_identifier" && !private {
            symbols
                .members
                .insert(format!("{class}.{}", text(method, source)));
        }
    }
}

/// 선언 안에서 쓰인 이름을 그 선언의 이름(여럿이면 각각, 없으면 파일 수준)으로 모은다.
/// 깊이 중첩된 코드에서 스택이 넘치지 않게 커서로 돈다.
fn record_uses(node: Node, source: &[u8], owners: &[String], symbols: &mut FileSymbols) {
    // 변수 선언은 선언자마다 따로 모은다. 구조 분해로 이름이 여럿이면 그 이름들이 모두 같은 것을 쓴다.
    if matches!(node.kind(), "lexical_declaration" | "variable_declaration") {
        let mut cursor = node.walk();
        for declarator in node
            .named_children(&mut cursor)
            .filter(|child| child.kind() == "variable_declarator")
        {
            let mut names = Vec::new();
            if let Some(pattern) = declarator.child_by_field_name("name") {
                pattern_names(pattern, source, &mut names);
            }
            let found = used_names(declarator, source);
            if names.is_empty() {
                symbols
                    .uses
                    .entry(None)
                    .or_default()
                    .extend(found.iter().cloned());
            }
            for name in names {
                symbols
                    .uses
                    .entry(Some(name))
                    .or_default()
                    .extend(found.iter().cloned());
            }
        }
        return;
    }
    let found = used_names(node, source);
    let owner = owners.first().cloned();
    symbols.uses.entry(owner).or_default().extend(found);
}

fn used_names(node: Node, source: &[u8]) -> BTreeSet<Use> {
    let mut found = BTreeSet::new();
    let mut cursor = node.walk();
    loop {
        let current = cursor.node();
        let mut descend = !current.is_error();
        match current.kind() {
            "member_expression" | "nested_type_identifier" | "member_expression_type" => {
                let object = current
                    .child_by_field_name("object")
                    .or_else(|| current.child_by_field_name("module"));
                let property = current
                    .child_by_field_name("property")
                    .or_else(|| current.child_by_field_name("name"));
                if let (Some(object), Some(property)) = (object, property)
                    && matches!(object.kind(), "identifier" | "type_identifier")
                {
                    found.insert(Use {
                        local: text(object, source).to_string(),
                        member: Some(text(property, source).to_string()),
                    });
                }
            }
            "identifier" | "type_identifier" | "shorthand_property_identifier" => {
                found.insert(Use {
                    local: text(current, source).to_string(),
                    member: None,
                });
                descend = false;
            }
            _ => {}
        }
        if descend && cursor.goto_first_child() {
            continue;
        }
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() || cursor.node() == node {
                return found;
            }
        }
    }
}

/// 파일들의 심볼 정보를 이어 심볼 노드와 의존을 만든다.
pub struct Linked {
    /// 심볼 ID(`파일경로#이름`) → 담은 파일
    pub symbols: BTreeMap<String, String>,
    /// (출발, 도착). 출발은 심볼 ID나 파일 경로, 도착은 심볼 ID다.
    pub edges: BTreeSet<(String, String)>,
}

/// `resolve(파일, 모듈 경로)`는 그 import가 닿는 분석 파일이다.
pub fn link(
    files: &BTreeMap<String, FileSymbols>,
    resolve: &dyn Fn(&str, &str) -> Option<String>,
) -> Linked {
    let mut symbols = BTreeMap::new();
    for (file, info) in files {
        for name in info.defined.keys() {
            symbols.insert(format!("{file}#{name}"), file.clone());
        }
        // 지역 선언을 다른 이름으로 내보낸 것도 이 파일의 심볼이다.
        for (exported, local) in &info.local_exports {
            if !info.imports.contains_key(local) {
                symbols.insert(format!("{file}#{exported}"), file.clone());
            }
        }
        // 멤버는 클래스를 어떤 이름으로든 내보냈을 때만 심볼이 된다. ID는 클래스의 지역 이름을 쓴다.
        let exported: HashSet<&str> = info
            .defined
            .values()
            .chain(
                info.local_exports
                    .values()
                    .filter(|local| !info.imports.contains_key(*local)),
            )
            .map(String::as_str)
            .collect();
        for member in &info.members {
            let class = member
                .split_once('.')
                .map_or(member.as_str(), |(class, _)| class);
            if exported.contains(class) {
                symbols.insert(format!("{file}#{member}"), file.clone());
            }
        }
    }

    let mut edges = BTreeSet::new();
    for (file, info) in files {
        // 선언의 지역 이름 → 그 선언을 내보낸 이름
        let mut exported_as: HashMap<&str, &str> = HashMap::new();
        for (exported, local) in info.defined.iter().chain(info.local_exports.iter()) {
            exported_as
                .entry(local.as_str())
                .or_insert(exported.as_str());
        }
        for (owner, uses) in &info.uses {
            let src = owner
                .as_deref()
                .and_then(|local| exported_as.get(local))
                .map(|exported| format!("{file}#{exported}"))
                .filter(|id| symbols.contains_key(id))
                .unwrap_or_else(|| file.clone());
            for used in uses {
                let Some((module, name)) = info.imports.get(&used.local) else {
                    continue;
                };
                let Some(target) = resolve(file, module) else {
                    continue;
                };
                let lookup = |file: &str, name: &str| {
                    definition(files, resolve, &symbols, file, name, &mut HashSet::new())
                };
                // 이름 전체(`* as ns`, `export * as ns`)를 가져왔으면 `ns.member`의 멤버를 찾는다.
                let dst = match (lookup(&target, name), used.member.as_deref()) {
                    (Some(Definition::Symbol(id)), None) => Some(id),
                    (Some(Definition::Namespace(namespace)), Some(member)) => {
                        match lookup(&namespace, member) {
                            Some(Definition::Symbol(id)) => Some(id),
                            _ => None,
                        }
                    }
                    _ => None,
                };
                if let Some(dst) = dst
                    && dst != src
                {
                    edges.insert((src.clone(), dst));
                }
            }
        }
    }
    Linked { symbols, edges }
}

/// 내보낸 이름이 닿는 곳: 심볼, 또는 파일의 이름 전체.
enum Definition {
    Symbol(String),
    Namespace(String),
}

/// `file`이 `name`으로 내보내는 것의 정의. re-export와 `export *`를 따라간다.
/// `export *`가 서로를 가리켜도 같은 (파일, 이름)은 한 번만 보므로 파일 수에 비례해 끝난다.
fn definition(
    files: &BTreeMap<String, FileSymbols>,
    resolve: &dyn Fn(&str, &str) -> Option<String>,
    symbols: &BTreeMap<String, String>,
    file: &str,
    name: &str,
    visited: &mut HashSet<(String, String)>,
) -> Option<Definition> {
    if !visited.insert((file.to_string(), name.to_string())) {
        return None;
    }
    let info = files.get(file)?;
    if name == "*" {
        return Some(Definition::Namespace(file.to_string()));
    }
    let id = format!("{file}#{name}");
    if symbols.contains_key(&id) {
        return Some(Definition::Symbol(id));
    }
    if let Some((module, original)) = info.reexports.get(name) {
        let target = resolve(file, module)?;
        return definition(files, resolve, symbols, &target, original, visited);
    }
    // 가져온 이름을 그대로 다시 내보낸 것
    if let Some(local) = info.local_exports.get(name)
        && let Some((module, original)) = info.imports.get(local)
    {
        let target = resolve(file, module)?;
        return definition(files, resolve, symbols, &target, original, visited);
    }
    if name == "default" {
        return None;
    }
    info.stars.iter().find_map(|module| {
        let target = resolve(file, module)?;
        definition(files, resolve, symbols, &target, name, visited)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tree_sitter::Parser;

    fn parse(text: &str) -> FileSymbols {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
            .unwrap();
        let tree = parser.parse(text, None).unwrap();
        extract(tree.root_node(), text.as_bytes())
    }

    /// `./x`를 `x.ts`로 해석하는 레포를 이어 심볼과 의존을 만든다.
    fn link_files(files: &[(&str, &str)]) -> Linked {
        let files: BTreeMap<String, FileSymbols> = files
            .iter()
            .map(|(path, text)| (path.to_string(), parse(text)))
            .collect();
        let known: BTreeSet<String> = files.keys().cloned().collect();
        let resolve = move |_: &str, module: &str| {
            let path = format!("{}.ts", module.trim_start_matches("./"));
            known.contains(&path).then_some(path)
        };
        link(&files, &resolve)
    }

    fn ids(linked: &Linked) -> Vec<&str> {
        linked.symbols.keys().map(String::as_str).collect()
    }

    #[test]
    fn 구조_분해와_declare와_네임스페이스와_export_등호를_심볼로_만든다() {
        let linked = link_files(&[(
            "a.ts",
            "export const { da, db: [dc] } = obj;\nexport declare function df(): void;\nexport namespace NS { export const x = 1; }\nexport module Mod {}\nfunction f() {}\nexport = f;\n",
        )]);
        assert_eq!(
            ids(&linked),
            [
                "a.ts#Mod",
                "a.ts#NS",
                "a.ts#da",
                "a.ts#dc",
                "a.ts#default",
                "a.ts#df"
            ]
        );
    }

    #[test]
    fn 나중에_내보낸_클래스와_이름_없는_기본_클래스의_공개_메서드만_멤버가_된다() {
        let linked = link_files(&[
            (
                "late.ts",
                "class Late { m() {} #hidden() {} private secret() {} [Symbol.iterator]() {} }\nclass Inner { n() {} }\nexport { Late };\n",
            ),
            ("anon.ts", "export default class { send() {} }\n"),
        ]);
        assert_eq!(
            ids(&linked),
            [
                "anon.ts#default",
                "anon.ts#default.send",
                "late.ts#Late",
                "late.ts#Late.m"
            ]
        );
    }

    #[test]
    fn export_star_as로_내보낸_이름_전체의_멤버를_찾는다() {
        let linked = link_files(&[
            ("money.ts", "export function round() {}\n"),
            ("barrel.ts", "export * as money from './money';\n"),
            (
                "use.ts",
                "import { money } from './barrel';\nexport const total = money.round();\n",
            ),
        ]);
        assert_eq!(
            linked.edges,
            BTreeSet::from([("use.ts#total".to_string(), "money.ts#round".to_string())])
        );
    }

    #[test]
    fn export_star가_서로를_가리켜도_없는_이름을_빨리_포기한다() {
        // 배럴 여섯 개가 서로를 모두 export *로 내보낸다. 경로마다 따라가면 끝나지 않는다.
        let names: Vec<String> = (0..6).map(|i| format!("b{i}")).collect();
        let texts: Vec<(String, String)> = names
            .iter()
            .map(|name| {
                let text: String = names
                    .iter()
                    .filter(|other| *other != name)
                    .map(|other| format!("export * from './{other}';\n"))
                    .collect();
                (format!("{name}.ts"), text)
            })
            .chain([(
                "use.ts".to_string(),
                "import { nope } from './b0';\nexport const x = nope;\n".to_string(),
            )])
            .collect();
        let files: Vec<(&str, &str)> = texts
            .iter()
            .map(|(path, text)| (path.as_str(), text.as_str()))
            .collect();
        let linked = link_files(&files);
        assert!(linked.edges.is_empty());
    }
}

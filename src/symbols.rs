//! 심볼: 파일이 내보내는 이름과, 최상위 선언이 가져다 쓰는 이름.
//!
//! 한 파일의 구문 트리에서 정보를 뽑는 일([`extract`])과, 파일들을 이어 심볼 → 심볼 의존을 만드는 일([`link`])로 나뉜다.
//! 타입 검사는 하지 않는다. 가져온 이름을 어느 최상위 선언이 쓰는지까지만 본다.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use tree_sitter::Node;

/// 한 파일에서 뽑은 심볼 정보.
#[derive(Debug, Clone, Default)]
pub struct FileSymbols {
    /// 이 파일에서 정의해 내보내는 이름. 내보낸 이름 → 그 선언의 지역 이름(같은 선언을 다른 이름으로 내보낼 수 있다).
    pub defined: BTreeMap<String, String>,
    /// 클래스 멤버 심볼. `Class.method`
    pub members: BTreeSet<String>,
    /// `export { a as b } from "./x"`: 내보낸 이름 → (모듈 경로, 가져온 이름)
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
                let names = declared_names(statement, source);
                record_uses(statement, source, &names, &mut symbols);
            }
        }
    }
    // 가져온 이름만 쓴 것으로 남긴다. 지역 변수와 같은 이름은 빼고, 이름 전체를 가져온 것은 멤버가 있어야 한다.
    let imports = symbols.imports.clone();
    for uses in symbols.uses.values_mut() {
        uses.retain(|used| match imports.get(&used.local) {
            Some((_, name)) if name == "*" => used.member.is_some(),
            Some(_) => used.member.is_none(),
            None => false,
        });
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
            // export * from "./x"
            None if !children
                .iter()
                .any(|child| child.kind() == "namespace_export") =>
            {
                symbols.stars.push(module);
            }
            // export * as ns from "./x"
            None => {}
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
        let names = declared_names(declaration, source);
        if is_default {
            let local = names
                .first()
                .cloned()
                .unwrap_or_else(|| "default".to_string());
            symbols.defined.insert("default".to_string(), local);
        } else {
            for name in &names {
                symbols.defined.insert(name.clone(), name.clone());
            }
        }
        record_members(declaration, source, symbols);
        record_uses(declaration, source, &names, symbols);
        // 이름 없는 `export default function () {}`
        if names.is_empty() && is_default {
            record_uses(declaration, source, &["default".to_string()], symbols);
        }
        return;
    }
    if let Some(value) = statement.child_by_field_name("value") {
        // export default <식>. 식이 지역 이름 하나면 그 선언을 `default`로도 내보낸다.
        if value.kind() == "identifier" {
            symbols
                .local_exports
                .insert("default".to_string(), text(value, source).to_string());
        } else {
            symbols
                .defined
                .insert("default".to_string(), "default".to_string());
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

/// 최상위 선언이 만드는 지역 이름. 함수, 클래스, 변수(여럿일 수 있음), 인터페이스, 타입, 열거형.
fn declared_names(declaration: Node, source: &[u8]) -> Vec<String> {
    match declaration.kind() {
        "function_declaration"
        | "generator_function_declaration"
        | "class_declaration"
        | "abstract_class_declaration"
        | "interface_declaration"
        | "type_alias_declaration"
        | "enum_declaration" => declaration
            .child_by_field_name("name")
            .map(|name| vec![text(name, source).to_string()])
            .unwrap_or_default(),
        "lexical_declaration" | "variable_declaration" => {
            let mut cursor = declaration.walk();
            declaration
                .named_children(&mut cursor)
                .filter(|child| child.kind() == "variable_declarator")
                .filter_map(|child| child.child_by_field_name("name"))
                .filter(|name| name.kind() == "identifier")
                .map(|name| text(name, source).to_string())
                .collect()
        }
        _ => Vec::new(),
    }
}

fn record_members(declaration: Node, source: &[u8], symbols: &mut FileSymbols) {
    if !matches!(
        declaration.kind(),
        "class_declaration" | "abstract_class_declaration"
    ) {
        return;
    }
    let (Some(name), Some(body)) = (
        declaration.child_by_field_name("name"),
        declaration.child_by_field_name("body"),
    ) else {
        return;
    };
    let class = text(name, source);
    let mut cursor = body.walk();
    for member in body.named_children(&mut cursor) {
        if matches!(
            member.kind(),
            "method_definition" | "abstract_method_signature" | "method_signature"
        ) && let Some(method) = member.child_by_field_name("name")
        {
            symbols
                .members
                .insert(format!("{class}.{}", text(method, source)));
        }
    }
}

/// 선언 안에서 쓰인 이름을 그 선언의 이름(여럿이면 각각, 없으면 파일 수준)으로 모은다.
/// 깊이 중첩된 코드에서 스택이 넘치지 않게 커서로 돈다.
fn record_uses(node: Node, source: &[u8], owners: &[String], symbols: &mut FileSymbols) {
    // 변수 선언이 여럿이면 선언자마다 따로 모은다.
    if matches!(node.kind(), "lexical_declaration" | "variable_declaration") && owners.len() > 1 {
        let mut cursor = node.walk();
        for declarator in node
            .named_children(&mut cursor)
            .filter(|child| child.kind() == "variable_declarator")
        {
            let owner = declarator
                .child_by_field_name("name")
                .map(|name| text(name, source).to_string());
            let found = used_names(declarator, source);
            symbols.uses.entry(owner).or_default().extend(found);
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
        for name in info.defined.keys().chain(info.members.iter()) {
            symbols.insert(format!("{file}#{name}"), file.clone());
        }
        // 지역 선언을 다른 이름으로 내보낸 것도 이 파일의 심볼이다.
        for (exported, local) in &info.local_exports {
            if !info.imports.contains_key(local) {
                symbols.insert(format!("{file}#{exported}"), file.clone());
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
                let name = used.member.as_deref().unwrap_or(name);
                if let Some(dst) = definition(files, resolve, &symbols, &target, name, 0)
                    && dst != src
                {
                    edges.insert((src.clone(), dst));
                }
            }
        }
    }
    Linked { symbols, edges }
}

/// `file`이 `name`으로 내보내는 것의 정의. re-export와 `export *`를 따라간다.
fn definition(
    files: &BTreeMap<String, FileSymbols>,
    resolve: &dyn Fn(&str, &str) -> Option<String>,
    symbols: &BTreeMap<String, String>,
    file: &str,
    name: &str,
    depth: usize,
) -> Option<String> {
    if depth > 16 {
        return None;
    }
    let info = files.get(file)?;
    let id = format!("{file}#{name}");
    if symbols.contains_key(&id) {
        return Some(id);
    }
    if let Some((module, original)) = info.reexports.get(name) {
        let target = resolve(file, module)?;
        return definition(files, resolve, symbols, &target, original, depth + 1);
    }
    // 가져온 이름을 그대로 다시 내보낸 것
    if let Some(local) = info.local_exports.get(name)
        && let Some((module, original)) = info.imports.get(local)
    {
        let target = resolve(file, module)?;
        return definition(files, resolve, symbols, &target, original, depth + 1);
    }
    if name == "default" {
        return None;
    }
    info.stars.iter().find_map(|module| {
        let target = resolve(file, module)?;
        definition(files, resolve, symbols, &target, name, depth + 1)
    })
}

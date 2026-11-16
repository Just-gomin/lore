//! `lore build`가 고정 예제로 디비를 만드는 과정을 확인한다.

mod common;

use std::path::Path;

use common::{git, json, lore_in, repo};
use rusqlite::Connection;
use serde_json::json;

fn dump(db: &Path) -> Vec<String> {
    let conn = Connection::open(db).unwrap();
    let mut rows = Vec::new();
    for sql in [
        "SELECT key || '=' || value FROM meta ORDER BY key",
        "SELECT id || '|' || kind || '|' || ifnull(title,'') || '|' || ifnull(summary,'') || '|' || ifnull(status,'') || '|' || source_path || '|' || content_hash || '|' || ifnull(frontmatter,'') FROM nodes ORDER BY id",
        "SELECT src || '|' || dst || '|' || kind || '|' || origin FROM edges ORDER BY src, dst, kind",
        "SELECT id || '|' || ifnull(title,'') || '|' || body FROM content ORDER BY id",
    ] {
        let mut stmt = conn.prepare(sql).unwrap();
        let values = stmt.query_map([], |row| row.get::<_, String>(0)).unwrap();
        rows.extend(values.map(Result::unwrap));
    }
    rows
}

#[test]
fn 고정_예제의_노드와_엣지_수가_기대값과_같다() {
    let dir = repo("payment");
    let output = lore_in(dir.path(), &["build"]);
    assert!(output.status.success(), "{}", common::stdout(&output));

    let report = json(&output);
    assert_eq!(report["ok"], true);
    assert_eq!(report["db"], ".lore/lore.db");
    assert_eq!(report["index"], "docs/INDEX.md");
    assert_eq!(
        report["nodes"],
        json!({"decision": 4, "feature": 4, "file": 7, "incident": 1, "source": 2})
    );
    assert_eq!(
        report["edges"],
        json!({
            "belongs_to": 2, "decides": 4, "depends_on": 2, "implements": 5,
            "occurred_in": 1, "supersedes": 1, "supports": 3, "triggers": 1, "verifies": 2
        })
    );
    assert_eq!(report["commit"].as_str().unwrap().len(), 40);
}

#[test]
fn 테스트_파일은_구현함이_아니라_검증함으로_잇는다() {
    let dir = repo("payment");
    assert!(lore_in(dir.path(), &["build"]).status.success());
    let conn = Connection::open(dir.path().join(".lore/lore.db")).unwrap();
    let kinds: Vec<String> = conn
        .prepare("SELECT kind FROM edges WHERE src = 'services/payment/retry/queue.test.ts'")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(kinds, ["verifies"]);
}

#[test]
fn 같은_커밋에서_두_번_빌드한_디비의_내용이_같다() {
    let dir = repo("payment");
    let db = dir.path().join(".lore/lore.db");
    assert!(lore_in(dir.path(), &["build"]).status.success());
    let first = dump(&db);
    assert!(lore_in(dir.path(), &["build"]).status.success());
    assert_eq!(first, dump(&db));
    assert!(first.iter().any(|row| row.starts_with("schema_version=")));
}

#[test]
fn 디비_폴더는_git이_무시해_작업_트리를_더럽히지_않는다() {
    let dir = repo("payment");
    assert!(lore_in(dir.path(), &["build"]).status.success());
    assert_eq!(git(dir.path(), &["status", "--porcelain"]), "");
}

#[test]
fn 본문은_trigram으로_조사가_붙은_말도_찾는다() {
    let dir = repo("payment");
    assert!(lore_in(dir.path(), &["build"]).status.success());
    let conn = Connection::open(dir.path().join(".lore/lore.db")).unwrap();
    let ids: Vec<String> = conn
        .prepare("SELECT id FROM content WHERE content MATCH '재시도' ORDER BY id")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert!(
        ids.contains(&"ADR-20261105-retry-queue".to_string()),
        "{ids:?}"
    );
    assert!(
        ids.contains(&"INC-20261020-duplicate-charge".to_string()),
        "{ids:?}"
    );
}

#[test]
fn git이_추적하지_않는_파일에만_맞는_경로는_오류다() {
    let dir = repo("payment");
    let spec = dir.path().join("docs/specs/payment-notification.md");
    let text = std::fs::read_to_string(&spec).unwrap();
    std::fs::write(
        &spec,
        text.replace("services/notification/**", "services/sms/**"),
    )
    .unwrap();
    std::fs::create_dir_all(dir.path().join("services/sms")).unwrap();
    std::fs::write(dir.path().join("services/sms/send.ts"), "").unwrap();

    let output = lore_in(dir.path(), &["build"]);
    assert_eq!(output.status.code(), Some(1));
    let report = json(&output);
    assert_eq!(report["ok"], false);
    assert_eq!(report["error"]["code"], "invalid_sources");
    assert_eq!(
        report["error"]["problems"],
        json!([{
            "path": "docs/specs/payment-notification.md",
            "field": "paths",
            "code": "no_matching_files",
            "message": "'services/sms/**'에 맞는 파일이 git이 추적하는 파일 중에 없습니다."
        }])
    );
}

#[test]
fn 잘못된_소스는_문제를_모두_담아_종료_코드_1로_실패한다() {
    let dir = repo("invalid");
    let output = lore_in(dir.path(), &["build"]);
    assert_eq!(output.status.code(), Some(1));
    let report = json(&output);
    assert_eq!(report["error"]["code"], "invalid_sources");
    assert_eq!(report["error"]["problems"].as_array().unwrap().len(), 17);
    assert!(!dir.path().join(".lore/lore.db").exists());
}

#[test]
fn 하위_폴더에서_실행해도_프로젝트_홈을_찾는다() {
    let dir = repo("payment");
    let output = lore_in(
        &dir.path().join("services/payment"),
        &["build", "--for-human"],
    );
    assert!(output.status.success());
    assert!(common::stdout(&output).starts_with("lore build: 노드 18개, 엣지 21개"));
    assert!(dir.path().join(".lore/lore.db").exists());
}

#[test]
fn 설정_파일이_없으면_config_not_found다() {
    let dir = tempfile::TempDir::new().unwrap();
    let output = lore_in(dir.path(), &["build"]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(json(&output)["error"]["code"], "config_not_found");
}

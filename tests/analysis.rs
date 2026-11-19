//! 정적 분석이 고정 예제의 import로 파일 → 파일 의존을 만드는지 확인한다.

mod common;

use std::path::Path;

use common::{json, lore_in, repo};
use rusqlite::Connection;

fn analysis_edges(dir: &Path) -> Vec<String> {
    let conn = Connection::open(dir.join(".lore/lore.db")).unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT src || ' -> ' || dst FROM edges WHERE origin = 'analysis' ORDER BY src, dst",
        )
        .unwrap();
    stmt.query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

#[test]
fn import를_파일_의존으로_만들고_문법_오류와_미해석은_보고만_한다() {
    let dir = repo("analysis");
    let output = lore_in(dir.path(), &["build"]);
    assert!(output.status.success(), "{}", common::stdout(&output));
    let report = json(&output);
    assert_eq!(
        report["analysis"],
        serde_json::json!({
            "files": 6,
            "edges": 6,
            "parse_errors": ["services/notification/broken.ts"],
            "unresolved": 1,
        })
    );
    // 레포 밖 패키지(node:fs)는 엣지도 만들지 않고 세지도 않는다.
    assert_eq!(
        analysis_edges(dir.path()),
        [
            "scripts/seed.mjs -> services/notification/alert.ts",
            "services/notification/alert.ts -> services/payment/core/index.ts",
            // 문법 오류가 난 곳 밖의 import는 그대로 쓴다.
            "services/notification/broken.ts -> services/notification/alert.ts",
            "services/payment/core/charge.ts -> services/payment/core/types.ts",
            "services/payment/core/index.ts -> services/payment/core/charge.ts",
            "services/payment/core/index.ts -> services/payment/core/types.ts",
        ]
    );
    // 스펙에 속하지 않은 scripts/seed.mjs도 노드가 된다. 분석한 파일 6개에 스펙이 맡는 alert.css를 더해 7개다.
    assert_eq!(report["nodes"]["file"], 7);
}

#[test]
fn 같은_커밋에서_두_번_빌드하면_분석_엣지가_같다() {
    let dir = repo("analysis");
    assert!(lore_in(dir.path(), &["build"]).status.success());
    let first = analysis_edges(dir.path());
    std::fs::remove_dir_all(dir.path().join(".lore")).unwrap();
    assert!(lore_in(dir.path(), &["build"]).status.success());
    assert_eq!(analysis_edges(dir.path()), first);
}

#[test]
fn analysis가_없는_설정은_분석하지_않는다() {
    let dir = repo("payment");
    let report = json(&lore_in(dir.path(), &["build"]));
    assert!(report.get("analysis").is_none());
    assert!(analysis_edges(dir.path()).is_empty());
}

#[test]
fn 사람용_출력에_분석_결과를_보여_준다() {
    let dir = repo("analysis");
    let text = common::stdout(&lore_in(dir.path(), &["build", "--for-human"]));
    assert!(
        text.contains("  분석    파일 6개, 의존 6개, 해석하지 못한 import 1개\n    문법 오류  services/notification/broken.ts"),
        "{text}"
    );
}

#[test]
fn 디비를_만든_lore_버전이_다르면_조회_전에_다시_만든다() {
    let dir = repo("analysis");
    assert!(lore_in(dir.path(), &["build"]).status.success());
    let conn = Connection::open(dir.path().join(".lore/lore.db")).unwrap();
    conn.execute(
        "UPDATE meta SET value = '0.0.0' WHERE key = 'lore_version'",
        [],
    )
    .unwrap();
    drop(conn);
    let report = json(&lore_in(dir.path(), &["why", "payment-core"]));
    assert_eq!(report["rebuilt"], true);
}

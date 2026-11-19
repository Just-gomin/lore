//! 정적 분석이 고정 예제의 import로 파일 → 파일 의존을 만드는지 확인한다.

mod common;

use std::path::Path;

use common::{json, lore_in, repo};
use rusqlite::Connection;

fn analysis_edges(dir: &Path) -> Vec<String> {
    let conn = Connection::open(dir.join(".lore/lore.db")).unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT src || ' -> ' || dst FROM edges WHERE origin = 'analysis' AND kind = 'depends_on' AND src NOT LIKE '%#%' AND dst NOT LIKE '%#%' ORDER BY src, dst",
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
            "files": 10,
            "edges": 12,
            "parse_errors": ["services/notification/broken.ts"],
            "unresolved": 2,
            "symbols": 8,
            "symbol_edges": 8,
        })
    );
    // 레포 밖 패키지(node:fs)는 엣지도 만들지 않고 세지도 않는다.
    assert_eq!(
        analysis_edges(dir.path()),
        [
            // 워크스페이스 패키지 안에서 패키지의 tsconfig 별칭(baseUrl 기준)을 따른다.
            "packages/money/src/index.ts -> packages/money/src/lib/round.ts",
            "scripts/seed.mjs -> services/notification/alert.ts",
            "services/notification/alert.ts -> services/payment/core/index.ts",
            // 문법 오류가 난 곳 밖의 import는 그대로 쓴다.
            "services/notification/broken.ts -> services/notification/alert.ts",
            // 루트 tsconfig의 `@/*`와 `@core` 별칭
            "services/notification/format.ts -> services/payment/core/index.ts",
            "services/notification/format.ts -> services/payment/core/types.ts",
            "services/notification/sender.ts -> services/notification/alert.ts",
            "services/notification/sender.ts -> services/payment/core/index.ts",
            // 워크스페이스 패키지 이름. exports의 import 조건을 따른다.
            "services/payment/core/charge.ts -> packages/money/src/index.ts",
            "services/payment/core/charge.ts -> services/payment/core/types.ts",
            "services/payment/core/index.ts -> services/payment/core/charge.ts",
            "services/payment/core/index.ts -> services/payment/core/types.ts",
        ]
    );
    // 스펙에 속하지 않은 scripts/, packages/의 파일도 노드가 된다. 분석한 파일 10개에 스펙이 맡는 alert.css를 더해 11개다.
    assert_eq!(report["nodes"]["file"], 11);
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
        text.contains("  분석    파일 10개, 의존 12개, 해석하지 못한 import 2개, 심볼 8개, 심볼 의존 8개\n    문법 오류  services/notification/broken.ts"),
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

#[test]
fn 스펙에_속하지_않은_분석_파일을_물으면_스펙이_없다고_알린다() {
    let dir = repo("analysis");
    let output = lore_in(dir.path(), &["why", "scripts/seed.mjs"]);
    assert!(output.status.success());
    let report = json(&output);
    assert_eq!(report["target"]["kind"], "file");
    assert_eq!(report["features"], serde_json::json!([]));
    let text = common::stdout(&lore_in(
        dir.path(),
        &["why", "scripts/seed.mjs", "--for-human"],
    ));
    assert!(
        text.contains("scripts/seed.mjs 에 연결된 스펙이 없습니다."),
        "{text}"
    );
}

fn query(dir: &Path, sql: &str) -> Vec<String> {
    let conn = Connection::open(dir.join(".lore/lore.db")).unwrap();
    let mut stmt = conn.prepare(sql).unwrap();
    stmt.query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .map(Result::unwrap)
        .collect()
}

#[test]
fn 내보낸_이름을_심볼로_만들고_re_export를_거쳐_정의에_잇는다() {
    let dir = repo("analysis");
    assert!(lore_in(dir.path(), &["build"]).status.success());
    assert_eq!(
        query(
            dir.path(),
            "SELECT id FROM nodes WHERE kind = 'symbol' ORDER BY id"
        ),
        [
            "packages/money/src/lib/round.ts#round",
            "services/notification/alert.ts#alert",
            "services/notification/format.ts#format",
            "services/notification/sender.ts#Sender.send",
            "services/notification/sender.ts#default",
            "services/notification/sender.ts#run",
            "services/payment/core/charge.ts#charge",
            "services/payment/core/types.ts#Charge",
        ]
    );
    assert_eq!(
        query(
            dir.path(),
            "SELECT src || ' -> ' || dst FROM edges WHERE kind = 'depends_on' AND dst LIKE '%#%' ORDER BY src, dst"
        ),
        [
            // core/index.ts의 export *를 거쳐 charge.ts의 정의에 닿는다.
            "services/notification/alert.ts#alert -> services/payment/core/charge.ts#charge",
            "services/notification/format.ts#format -> services/payment/core/charge.ts#charge",
            "services/notification/format.ts#format -> services/payment/core/types.ts#Charge",
            // 기본 내보내기 클래스가 별칭으로 가져온 이름과 이름 전체의 멤버(core.charge)를 쓴다.
            "services/notification/sender.ts#default -> services/notification/alert.ts#alert",
            "services/notification/sender.ts#default -> services/payment/core/charge.ts#charge",
            // 내보내지 않은 선언을 다른 이름(run)으로 내보낸 것
            "services/notification/sender.ts#run -> services/payment/core/charge.ts#charge",
            // 워크스페이스 패키지의 export { round } from "#lib/round"를 거친다.
            "services/payment/core/charge.ts#charge -> packages/money/src/lib/round.ts#round",
            "services/payment/core/charge.ts#charge -> services/payment/core/types.ts#Charge",
        ]
    );
    // 심볼은 담은 파일에 속한다.
    assert_eq!(
        query(
            dir.path(),
            "SELECT dst FROM edges WHERE kind = 'belongs_to' AND src = 'services/notification/sender.ts#Sender.send'"
        ),
        ["services/notification/sender.ts"]
    );
}

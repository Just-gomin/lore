//! 추출기와 코어를 잇는 통합 확인: 엑셀 파일 → 추출기 → `lore add source` → 결정 → `lore why --depth source`.
//!
//! 추출기와 코어 크레이트는 서로 의존하지 않으므로, `cargo test --workspace`가 함께 빌드한 추출기 실행 파일을
//! 이 테스트 실행 파일 옆 폴더(`target/<프로필>/`)에서 찾아 부른다.

mod common;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use common::{json, lore_in, lore_with_input, repo};

/// `target/<프로필>/lore-extractor-xlsx`. 이 테스트 실행 파일은 `target/<프로필>/deps/`에 있다.
fn extractor() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let path = exe.parent()?.parent()?.join("lore-extractor-xlsx");
    path.is_file().then_some(path)
}

#[test]
fn 엑셀_회의_자료가_소스_노트를_거쳐_결정에_연결된다() {
    let Some(extractor) = extractor() else {
        // `cargo test -p lore`처럼 추출기를 빌드하지 않은 경우. CI는 `--workspace`로 돌린다.
        assert!(
            std::env::var_os("CI").is_none(),
            "CI는 cargo test --workspace로 추출기를 함께 빌드해야 합니다."
        );
        eprintln!(
            "lore-extractor-xlsx가 빌드되지 않아 건너뜁니다. cargo test --workspace로 돌리세요."
        );
        return;
    };
    let dir = repo("payment");
    let file =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("extractors/xlsx/tests/fixtures/meeting.xlsx");

    // 1. 추출기
    let mut child = Command::new(&extractor)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let input = serde_json::json!({
        "file": file.display().to_string(),
        "channel": "github",
        "origin": "https://drive.google.com/file/d/meeting-sheet",
        "kind": "client-doc",
        "occurred_at": "2026-11-03",
    });
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.to_string().as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let extracted: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let mut candidate = extracted["candidates"][0].clone();
    assert!(candidate["text"].as_str().unwrap().contains("재시도 횟수"));

    // 2. 정제: 스킬이 할 일을 테스트가 정한 글로 대신한다.
    let object = candidate.as_object_mut().unwrap();
    object.remove("text");
    object.insert("slug".into(), "retry-sheet".into());
    object.insert(
        "body".into(),
        "- 회의 자료의 표에서 재시도 횟수를 3회로 적음\n".into(),
    );
    let added = lore_with_input(
        dir.path(),
        &["add", "source"],
        &serde_json::Value::Object(object.clone()).to_string(),
    );
    assert!(added.status.success(), "{}", common::stdout(&added));
    let source = json(&added)["id"].as_str().unwrap().to_string();
    assert_eq!(source, "SRC-20261103-retry-sheet");

    // 3. 이 소스 노트를 근거로 한 결정
    let decision = lore_in(
        dir.path(),
        &[
            "add",
            "decision",
            "--slug",
            "retry-three",
            "--title",
            "재시도는 3회로 둔다",
            "--specs",
            "payment-retry",
            "--sources",
            &source,
            "--date",
            "2026-11-05",
        ],
    );
    assert!(decision.status.success(), "{}", common::stdout(&decision));
    let decision = json(&decision)["id"].as_str().unwrap().to_string();

    // 4. 결정에서 근거까지 조회한다.
    let why = json(&lore_in(
        dir.path(),
        &["why", &decision, "--depth", "source"],
    ));
    let record = &why["records"][0];
    assert_eq!(record["sources"][0]["id"], source);
    assert!(
        record["sources"][0]["body"]
            .as_str()
            .unwrap()
            .contains("재시도 횟수를 3회로")
    );
}

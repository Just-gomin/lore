# AGENTS.md

이 레포에서 일하는 에이전트가 가장 먼저 읽는 문서입니다. lore 자체를 만드는 레포입니다.

## 먼저 읽을 문서

- [`docs/plan/plan.md`](docs/plan/plan.md): 기획서. 무엇을 왜 만드는지와 확정한 결정 서른두 가지가 있습니다. 설계에 관한 질문은 여기서 먼저 찾습니다.
- [`docs/plan/stage-4.md`](docs/plan/stage-4.md): 지금 하고 있는 4단계의 작업 순서와 완료 기준입니다. 끝난 단계는 [`stage-2.md`](docs/plan/stage-2.md), [`stage-3.md`](docs/plan/stage-3.md)에 있습니다.

기획서와 다르게 만들어야 할 이유가 생기면 코드만 바꾸지 말고, 사람에게 묻고 기획서 9장의 결정 기록을 같은 PR에서 고칩니다.

## 지켜야 할 것

- lore 코어는 네트워크와 LLM 없이 동작합니다. 같은 입력에는 같은 출력을 냅니다. 요약이나 판단처럼 LLM이 필요한 일은 코어에 넣지 않고 에이전트 스킬에 둡니다.
- 모든 명령의 기본 출력은 JSON입니다. 사람용 텍스트는 `--for-human`일 때만 냅니다. JSON일 때는 오류도 표준 출력으로 냅니다.
- 디비는 소스에서 다시 만들 수 있는 빌드 산출물입니다. 커밋하지 않습니다.
- 채널에서 자료를 꺼내는 커넥터와 비텍스트 추출기는 코어에 넣지 않습니다. 표준 입출력으로 JSON을 주고받는 별도 실행 파일로 둡니다.

## 구조

| 경로                 | 내용                                                 |
| -------------------- | ---------------------------------------------------- |
| `src/main.rs`        | 진입점. 인자를 넘기고 결과를 출력만 합니다           |
| `src/lib.rs`         | `run`: 인자를 받아 출력과 종료 코드를 돌려줍니다     |
| `src/cli.rs`         | 명령 이름과 공통 옵션 해석                           |
| `src/output.rs`      | JSON과 사람용 텍스트 출력                            |
| `src/error.rs`       | 명령의 실패 이유와 소스 문제(`Problem`)              |
| `src/config.rs`      | `lore.config.yaml` 찾기와 읽기                       |
| `src/frontmatter.rs` | 머리말 분리와 항목 읽기                              |
| `src/source.rs`      | 소스 폴더를 훑어 종류별로 읽고 참조를 검사           |
| `src/git.rs`         | 추적 파일 목록, 현재 커밋, 작업 트리 상태            |
| `src/graph.rs`       | 소스와 파일로 노드와 엣지 계산                       |
| `src/db.rs`          | SQLite 스키마와 쓰기                                 |
| `src/index.rs`       | `docs/INDEX.md` 본문 만들기                          |
| `src/build.rs`       | `lore build`: 디비와 인덱스 만들기                   |
| `src/add.rs`         | `lore add`: 결정과 사고 기록 만들기                  |
| `src/add_source.rs`  | `lore add source`: 등급을 적용해 소스 노트 만들기    |
| `src/install.rs`     | `lore install`, `lore uninstall`: 안내문, 뼈대, 스킬 |
| `src/check.rs`       | `lore check`: 소스와 코드가 어긋났는지 검사          |
| `src/query.rs`       | 조회 준비: 디비를 최신으로 맞추고 대상 해석          |
| `src/why.rs`         | `lore why`: ID나 파일 경로로 결정과 사고 조회        |
| `src/impact.rs`      | `lore impact`: 영향 범위의 다섯 묶음                 |
| `templates/`         | 바이너리에 넣는 템플릿, 안내문, 기본 설정, 스킬      |
| `tests/cli.rs`       | 바이너리를 실행해 출력 계약을 확인하는 테스트        |
| `tests/sources.rs`   | 고정 예제로 소스 읽기를 확인하는 테스트              |
| `tests/build.rs`     | 고정 예제로 `lore build`를 확인하는 테스트           |
| `tests/index.rs`     | 고정 예제로 `docs/INDEX.md` 생성을 확인하는 테스트   |
| `tests/why.rs`       | 고정 예제로 `lore why` 조회를 확인하는 테스트        |
| `tests/add.rs`       | 고정 예제로 `lore add`를 확인하는 테스트             |
| `tests/add_source.rs` | 고정 예제로 `lore add source`를 확인하는 테스트     |
| `tests/install.rs`   | 빈 레포와 기존 파일로 `lore install`을 확인하는 테스트 |
| `tests/uninstall.rs` | 설치한 것만 걷어내는지 확인하는 테스트               |
| `tests/skills.rs`    | 스킬과 안내문의 명령 예시가 해석되는지 확인          |
| `tests/check.rs`     | 고정 예제로 `lore check`를 확인하는 테스트           |
| `tests/impact.rs`    | 고정 예제로 `lore impact`를 확인하는 테스트          |
| `tests/common/`      | 고정 예제를 임시 git 레포로 만드는 도우미            |
| `tests/fixtures/`    | 고정 예제. `payment`는 정상, `invalid`는 잘못된 입력 |

## 작업 방법

- 바꾸기 전후로 `cargo fmt --all`, `cargo clippy --all-targets -- -D warnings`, `cargo test`를 돌립니다. CI가 같은 세 가지를 검사합니다.
- 출력 형식을 바꾸면 `tests/cli.rs`의 기대값도 같은 PR에서 바꿉니다.
- 테스트 이름은 무엇을 보장하는지 한국어 문장으로 씁니다.

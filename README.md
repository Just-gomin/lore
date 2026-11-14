# lore

결정 기록을 코드 옆에 두고 조회하는 AI Native SDLC 도구입니다. 코드를 왜 이렇게 만들었는지를 레포의 `docs/`에 마크다운으로 남기고, `lore`가 그것을 디비로 컴파일해 에이전트와 사람이 조회합니다.

아직 뼈대 단계입니다. 명령 이름만 등록되어 있고, 실행하면 `not_implemented` 오류를 냅니다.

## 명령

| 명령             | 역할                                                           |
| ---------------- | -------------------------------------------------------------- |
| `lore install`   | 설정 파일, 문서 뼈대, 에이전트 안내문, 스킬, CI 워크플로 설치  |
| `lore uninstall` | 설치한 안내문 구간, 스킬, CI 워크플로 제거                     |
| `lore init`      | 기존 이력 확보와 첫 디비 구축                                  |
| `lore add`       | 결정, 사고, 소스 노트 추가                                     |
| `lore why`       | 코드나 기능의 배경 조회                                        |
| `lore impact`    | 변경의 영향 범위 파악                                          |
| `lore build`     | 소스를 읽어 디비로 컴파일                                      |
| `lore check`     | 신선도 검사                                                    |
| `lore sync`      | 코드 변경분에 맞춰 소스 갱신                                   |

모든 명령은 JSON을 출력합니다. 사람이 읽을 때는 `--for-human`을 붙입니다.

## 개발

Rust로 만듭니다. `Cargo.toml`은 Rust의 패키지 명세 파일로, Node.js의 `package.json`에 해당합니다. lore를 쓰는 프로젝트의 설정 파일인 `lore.config.yaml`과는 다른 파일입니다.

```sh
cargo build          # 빌드
cargo test           # 테스트
cargo fmt --all      # 포맷
cargo clippy --all-targets -- -D warnings   # 린트
cargo run -- help --for-human               # 실행
```

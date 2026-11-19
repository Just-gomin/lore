---
name: lore-init-channels
description: 기존 코드와 이력이 있는 프로젝트에 lore를 들일 때 1단계로, 팀이 결정에 쓰는 도구를 리드 개발자에게 묻고 포함할 채널, 민감도 등급, 수집 경로를 lore.config.yaml에 적는다. lore init이 next로 channels를 낼 때 쓴다.
---

# lore 도입 1단계: 채널 고르기

결정이 오간 곳을 채널로 등록합니다. 고르지 않은 곳은 읽지 않습니다. 질문에는 프로젝트의 리드 개발자가 답합니다.

## 1. 지금 상태 보기

```sh
lore init
```

`next`가 `channels`가 아니면 이 단계는 끝났습니다. `next`에 맞는 스킬로 넘어갑니다.

`lore.config.yaml`에 `check.scope`가 없으면, 이미 `lore install`을 실행했던 레포라 판단이 드는 검사가 모든 파일을 보고 있습니다. 사람에게 알리고, 동의하면 `check:` 아래에 `scope: []`를 더해 `lore init`을 마친 모듈부터 검사를 켭니다.

## 2. 도구 묻기

리드 개발자에게 아래를 묻습니다. 답을 짐작해 채우지 않습니다.

- 결정을 어디서 이야기했나요? 예: GitHub PR과 이슈, Slack, 카카오톡, 회의, Google Drive 문서, 위키
- 채널마다 어디까지 포함할까요? 대화방, 폴더, 기간
- 채널마다 민감한 내용이 섞이나요? 개인 대화, 계약, 인사, 정산

## 3. 채널마다 정할 것

- `id`: 영문 소문자로 짧게 짓습니다. `lore add source` 입력의 `channel`과 같아야 합니다.
- `sensitivity`
  - `normal`: 정제한 내용과 원본 링크를 남깁니다.
  - `sensitive`: 원본 링크와 한 줄 제목만 남깁니다. 개인 대화가 섞이거나 계약을 다루는 곳입니다.
  - `excluded`: 읽지 않습니다. 인사, 정산 관련 채널입니다.
- `tool`: 수집 경로입니다.
  - 커넥터 실행 파일 이름: git 이력은 `lore-connector-git`, GitHub PR과 이슈는 `lore-connector-github`
  - `agent`: 에이전트에 연결된 도구(MCP 등)로 읽는 곳. 예: Slack, Google Drive, 위키
  - `file`: 사람이 내보낸 파일로 가져오는 곳. 예: 카카오톡 대화 내보내기, 회의 녹음 전사
- `scope`: 포함할 대화방, 폴더, 기간을 사람이 읽는 글로 적습니다. 코어는 읽지 않고 2단계 스킬이 읽습니다.

같은 도구라도 수집 경로가 다르면 채널을 나눕니다. 예: GitHub에서 커넥터로 병합된 PR을 모으는 채널과, 에이전트 도구로 열린 이슈의 논의를 읽는 채널

## 4. 설정에 적기

`lore.config.yaml`의 `channels`에 적습니다.

```yaml
channels:
  - id: github
    tool: lore-connector-github
    sensitivity: normal
    scope: 2025년 이후 병합된 PR
  - id: kakaotalk
    tool: file
    sensitivity: sensitive
    scope: 고객사 공지방. 개인 대화방은 넣지 않음
```

## 5. 확인

```sh
lore init
```

`channels`에 적은 채널이 모두 나오고 `next`가 `collect`이면 `lore-init-collect` 스킬로 넘어갑니다. 설정 변경은 PR로 올려 리드 개발자가 확인합니다.

---
name: lore-init-collect
description: 기존 프로젝트에 lore를 들일 때 2단계로, lore.config.yaml의 채널마다 정한 수집 경로(커넥터, 에이전트 도구, 파일 가져오기)로 결정에 관련된 자료 후보를 모으고, 사람이 고른 것만 정제해 소스 노트로 등록한다. lore init이 next로 collect를 낼 때 쓴다.
---

# lore 도입 2단계: 자료 모으기

채널에서 결정에 영향을 준 자료를 찾아 소스 노트로 만듭니다. 무엇을 남길지는 사람이 고르고, 만든 소스 노트는 PR에서 승인받습니다.

## 1. 채널 확인

```sh
lore init
```

`channels`의 `tool`과 `lore.config.yaml`의 `scope`를 읽습니다. `excluded` 채널은 읽지 않습니다.

## 2. 후보 모으기

채널의 `tool`에 맞는 경로로 후보를 모읍니다. 도입 범위(`lore init --scope`)가 정해져 있으면 그 경로를 바꾼 것부터 봅니다.

커넥터 채널은 커넥터를 실행합니다. 출력의 `candidates`가 후보이고 `text`가 원문입니다.

```text
lore-connector-github --channel github --since 2025-01-01
lore-connector-git --channel git --paths 'server/**' --exclude-pr-commits
```

- squash merge 레포에서 GitHub 채널이 있으면 git 커넥터에 `--exclude-pr-commits`를 붙입니다. 같은 결정이 PR과 커밋으로 두 번 나오지 않게 하기 위해서입니다. 모든 커밋이 PR을 거쳤다면 git 채널의 후보는 비어 있는 것이 정상입니다.
- 출력의 `ok`가 `false`면 `error`를 사람에게 알리고 그 채널은 건너뜁니다.

`agent` 채널은 연결된 도구로 `scope`에 적힌 범위를 읽습니다. 도구가 없으면 사람에게 알리고 건너뜁니다.

`file` 채널은 사람에게 내보낸 파일을 달라고 합니다. 파일의 원본(내보내기 전의 대화방이나 녹음)은 레포에 두지 않고, 지정된 Drive 폴더에 올린 링크를 `origin`으로 씁니다.

## 3. 사람이 고르기

후보가 많으면 먼저 거릅니다. 둘 이상의 방법 중 하나를 고른 논의, 요구사항, 장애와 그 조치처럼 결정의 근거가 될 만한 것만 남깁니다. 그 목록을 제목, 날짜, 링크로 보여 주고 사람이 소스 노트로 만들 것을 고르게 합니다.

## 4. 정제와 등록

고른 후보마다 `lore-source` 스킬의 기준으로 정제합니다. 결정에 영향을 준 내용만 `body`에 적고, 민감 채널은 `body`를 뺍니다. 커넥터 후보는 `title`, `kind`, `channel`, `origin`, `origin_version`, `occurred_at`을 그대로 쓰고 `slug`만 영문으로 짓습니다.

```sh
lore add source --from source.json
```

입력 JSON 파일은 레포에 커밋하지 않습니다.

커넥터가 판을 매기는 원본(GitHub PR과 이슈 주소, 커밋 주소)은 에이전트 도구로 읽었더라도 판을 커넥터 규칙으로 적습니다. 그렇지 않으면 `lore check --origins`가 그 노트를 낡은 것으로 봅니다. 노트를 만든 뒤 커넥터의 `--origins` 출력에서 그 원본의 `origin_version`을 찾아 입력에 넣고 `lore add source`를 다시 실행합니다. 같은 `origin`이면 새로 만들지 않고 그 노트를 갱신합니다.

```text
lore-connector-github --origins docs
```

## 5. 확인

```sh
lore init
```

`sources`가 늘었고 `next`가 `draft`이면 `lore-init-draft` 스킬로 넘어갑니다. 소스 노트는 PR로 올려 리드 개발자가 확인합니다.

---
name: lore-init-draft
description: 기존 프로젝트에 lore를 들일 때 3단계로, lore init이 알려 준 스펙에 속하지 않은 코드 파일을 묶어 스펙 초안을 만들고, 소스 노트에서 결정 기록 초안을 쓴 뒤 lore build를 처음 실행해 PR로 올린다. lore init이 next로 draft를 낼 때 쓴다.
---

# lore 도입 3단계: 스펙과 결정 초안

도입 범위의 코드 파일이 모두 스펙에 속하고, 주요 결정이 그 코드에서 조회되게 합니다. 결정 초안은 모두 `confidence: inferred`이고, 스펙과 결정 초안은 PR에서 리드 개발자가 확인합니다.

## 1. 속하지 않은 파일 보기

```sh
lore init --scope <폴더>
```

`coverage.unmapped`가 스펙에 속하지 않은 코드 파일입니다. 코드가 아닌 파일(문서, 설정, 이미지)은 스펙에 넣지 않고 `lore.config.yaml`의 `check.ignore`에 글롭으로 더합니다.

## 2. 스펙 초안

바뀌는 이유가 같은 파일을 한 스펙으로 묶습니다. 처음에는 크게 묶고, 나눌지는 나중에 `lore check --report`의 후보를 보고 정합니다. 유틸과 인프라는 기능에 억지로 넣지 않고 플랫폼 스펙으로 묶습니다.

```sh
lore add spec --slug <영문 슬러그> --title <제목> --paths <글롭> --tests <글롭> --parent <상위 스펙 ID>
```

- `--paths`, `--tests`는 글롭 하나씩 여러 번 줍니다.
- 본문의 "지금 동작"에는 코드에서 확인한 동작만 적습니다. 코드에 없는 동작은 짐작해 적지 않습니다.
- 새 파일을 `git add`해야 `lore build`가 글롭을 파일에 맞춥니다.

## 3. 결정 초안

소스 노트마다 그 자료로 내린 결정을 찾아 기록합니다. 결정의 이유는 소스 노트에 있는 것만 씁니다.

```sh
lore add decision --slug retry-queue --title "재시도를 큐로 옮긴다" --specs payment-retry --sources SRC-20250612-retry-meeting --date 2025-06-12
```

- `--date`는 결정을 내린 날입니다. 소스 노트의 `occurred_at`을 씁니다.
- 본문은 `lore-record` 스킬의 기준으로 채웁니다.
- 소스 노트에 이유가 없으면(민감 채널처럼 본문이 없는 경우 포함) "이유 불명"이라고 적고, PR 설명의 질문 목록에 그 결정을 넣습니다. 리드 개발자도 이유를 모르면 "이유 불명"으로 남깁니다. 모른다는 사실도 다음 사람에게는 정보입니다.
- 확인이 필요한 것은 PR 설명의 질문 목록에 적습니다.

## 4. 빌드와 검사 범위

```sh
lore build
lore init --scope <폴더>
lore check
```

- `coverage.ratio`가 1이고 `next`가 `done`이면 범위의 코드 파일이 모두 스펙에 속합니다.
- `lore check`의 실패를 모두 고칩니다.
- `lore.config.yaml`의 `check.scope`에 마친 범위를 글롭으로 더합니다. 그 범위부터 PR의 판단이 드는 검사가 켜집니다. `check.scope`가 아예 없으면 지금은 모든 파일을 보고 있으므로, 범위를 적으면 나머지 파일의 검사가 꺼진다는 것을 사람에게 알리고 정합니다.

```yaml
check:
  scope:
    - server/**
```

## 5. PR

스펙, 결정, 소스 노트, `docs/INDEX.md`, 설정을 한 PR에 담습니다. PR 설명에는 확인할 질문 목록을 적고 리드 개발자에게 답을 받습니다. PR이 병합되기 전에 답을 받으면 초안의 본문을 고치고, 사람이 이유를 확인한 결정은 `confidence: stated`로 바꿉니다. 병합한 뒤에는 기존 결정을 고치지 않고 새 결정으로 대체합니다.

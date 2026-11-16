---
id: ADR-20261105-retry-queue
title: 큐 기반 재시도로 전환, 최대 3회
status: accepted
supersedes: ADR-20260918-retry-inline
specs: [payment-retry]
sources: [SRC-20261103-retry-meeting, https://github.com/org/repo/pull/231]
confidence: stated
---

## 배경

요청 안의 재시도가 응답을 느리게 했습니다.

## 결정

실패한 결제를 큐에 넣고 최대 3회까지 다시 시도합니다. 3회는 카드사의 일시 장애가 대부분 풀리는 시간에 맞춘 값입니다.

## 검토한 대안

## 결과와 감수한 비용

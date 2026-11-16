---
id: ADR-20261022-idempotency-key
title: 결제 요청에 멱등 키를 붙인다
status: accepted
specs: [payment-core, payment-retry]
sources: [SRC-20261020-incident-thread]
confidence: stated
---

## 배경

재시도 중 같은 결제가 두 번 승인된 사고가 있었습니다.

## 결정

결제 요청마다 멱등 키를 붙여 카드사가 중복 요청을 거르게 합니다.

## 검토한 대안

## 결과와 감수한 비용

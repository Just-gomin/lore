---
id: ADR-20260918-retry-inline
title: 요청 안에서 즉시 재시도
status: accepted
specs: [payment-retry]
sources: []
confidence: inferred
---

## 배경

결제 실패가 드물어 별도 인프라 없이 처리하고 싶었습니다.

## 결정

결제 요청 안에서 실패하면 바로 한 번 더 시도합니다.

## 검토한 대안

## 결과와 감수한 비용

- 응답이 느려집니다.

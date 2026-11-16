---
id: payment-retry
title: 결제 재시도
paths:
  - services/payment/retry/**
tests:
  - services/payment/retry/**/*.test.ts
depends_on: [payment-core]
parent: payment
---

## 지금 동작

실패한 결제를 큐에 넣고 최대 3회까지 다시 시도합니다.

# INDEX

모든 기록의 한 줄 목록입니다. `lore build`가 만드는 파일이니 손으로 고치지 마세요. ID 순으로 정렬하고 스펙별로 묶습니다.

## payment

- `payment` · 결제 · [스펙](specs/payment.md)

## payment-core

- `payment-core` · 결제 처리 · [스펙](specs/payment-core.md)
- `ADR-20261022-idempotency-key` · 결제 요청에 멱등 키를 붙인다 · [결정](decisions/ADR-20261022-idempotency-key.md)
- `SRC-20261020-incident-thread` · 중복 결제 대응 스레드 · [소스 노트](sources/SRC-20261020-incident-thread.md)

## payment-notification

- `payment-notification` · 결제 알림 · [스펙](specs/payment-notification.md)

## payment-retry

- `payment-retry` · 결제 재시도 · [스펙](specs/payment-retry.md)
- `ADR-20260918-retry-inline` · 요청 안에서 즉시 재시도 · [결정](decisions/ADR-20260918-retry-inline.md) · [대체됨: ADR-20261105-retry-queue]
- `ADR-20261022-idempotency-key` · 결제 요청에 멱등 키를 붙인다 · [결정](decisions/ADR-20261022-idempotency-key.md)
- `ADR-20261105-retry-queue` · 큐 기반 재시도로 전환, 최대 3회 · [결정](decisions/ADR-20261105-retry-queue.md)
- `INC-20261020-duplicate-charge` · 재시도 중 중복 결제 발생 · [사고](incidents/INC-20261020-duplicate-charge.md)
- `SRC-20261020-incident-thread` · 중복 결제 대응 스레드 · [소스 노트](sources/SRC-20261020-incident-thread.md)
- `SRC-20261103-retry-meeting` · 결제 재시도 방식 회의 · [소스 노트](sources/SRC-20261103-retry-meeting.md)

## 스펙에 연결되지 않은 기록

- `ADR-20261110-log-format` · 로그를 JSON 한 줄로 남긴다 · [결정](decisions/ADR-20261110-log-format.md)

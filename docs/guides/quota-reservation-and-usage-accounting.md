# How Aster Team member quota and usage accounting work

[简体中文](../zh-CN/guides/quota-reservation-and-usage-accounting.md) · [Deployment guides](README.md) · [Member guide](../member-guide.md)

Aster Team assigns quota to individual members and accounts for each model request through reservation and settlement. This prevents concurrent requests from all spending the same available balance while keeping usage attributable to the member API key that initiated the request.

## Quota lifecycle

```text
Member grant
    │
    ▼
Available quota ── request accepted ──► Reserved quota
                                           │
                         ┌─────────────────┴─────────────────┐
                         ▼                                   ▼
                Trusted usage received              Request never executes
                         │                                   │
                         ▼                                   ▼
                   Usage settled                      Reservation released
```

1. An administrator grants quota to a member.
2. Before routing a request, Control reserves enough available quota for that in-flight request.
3. If no eligible Runner or account can accept the request, the reservation is released and the request is not charged.
4. After execution, Aster settles the request from trusted usage returned by the execution path.
5. Usage records and quota ledger entries retain the member, key, model, protocol, request identifier, and accounting result needed for review.

## Granted, reserved, and consumed values

- **Granted** is quota added by an administrator, approved request, or supported redemption flow.
- **Reserved** is temporarily unavailable while requests are in flight or awaiting recovery.
- **Consumed** is usage successfully settled to the ledger.
- **Available** is the spendable balance after grants, consumption, and current reservations.

Because reservations protect concurrent usage, the displayed available balance can temporarily be lower than granted minus consumed.

## Interrupted requests

If Control restarts after execution, it first attempts to recover persisted results and settle the original request. A reservation without a recoverable result becomes eligible for periodic reconciliation after 30 minutes. Aster releases such an expired reservation without inventing token usage and records that the final upstream execution result was uncertain.

Do not delete settlement state or edit the database to release quota. If a reservation remains after the reconciliation window, preserve the request ID and collect Control logs with `aster-team-cli trace`.

## Investigate a quota problem

- `34001 INSUFFICIENT_QUOTA` means available quota is too low after current reservations.
- Review the member's grant, consumed usage, and in-flight reservations in Admin or Member UI.
- Confirm that the client is using the intended member key rather than a key copied from another person or automation identity.
- Use `X-Aster-Request-ID` to correlate the API error, usage record, and Control trace.
- Grant additional quota only after checking whether the request pattern and selected model are expected.

Prompt and response bodies are not used as the business usage ledger. Runner and infrastructure operators must still apply their own log-access and retention controls.

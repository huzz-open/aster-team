# Upgrades and persistence

Read when changing schema migrations, Customer upgrades/recovery, request draining or Operations environment upgrades.

- Published migrations are immutable and checksummed. Use forward-only expand/contract changes and resumable backfills; do not restore an old database snapshot as automatic program rollback. MariaDB DDL requires statement-level idempotency and precise interrupted-migration recovery.
- SQLite/SQLCipher supports maintenance upgrades: confirm old writers have stopped before migrating/starting the candidate. Two retained slots do not imply supported concurrent business instances.
- External databases are necessary but insufficient for blue-green support. Backend capabilities must check platform, signed releases, migration compatibility, Runner chains, readiness and shared-state protocols. Do not enable a mode from a frontend inference or silently downgrade a requested mode.
- Preserve installation identity, licenses and history, database keys/data and logical Runner ownership across upgrades. Two slot instances of one logical Runner share quota only through verified installation/task/instance ownership; never bypass registration quotas.
- Freeze source/candidate release identities in durable preparation/recovery records. Revalidate signatures, actual paths, process identities and proxy state at the relevant transition; timeout does not prove that a database commit or service start never happened.
- Online coexistence requires compatible signed settlement-intent and execution-lock protocols on both releases. Preserve phase-specific recovery: a commit phase with durable retirement evidence must not be confused with an earlier phase still allowing coexistence.
- Account for requests from admission through upstream work, response completion, settlement/release and required child work. Disconnects, empty in-memory state and Runner completion alone do not prove safe retirement. New work must not race with drain-to-zero.
- Retry, refresh and transport phases consume the same remaining request budget. Cancellation and unknown upstream outcomes must not cause duplicate generation, billing or silent cross-account replay.
- Preserve old Control/Runner chains while their requests drain. Verify identity and termination before reusing a slot; recover or cancel from the persisted phase instead of guessing from the last command exit.
- Operations upgrades use authenticated normal target APIs and durable background jobs; closing the UI must not stop execution. Customer operation and manual maintenance remain independent of Operations availability.
- Report upgrade execution, continuity, recovery and coverage gaps separately. Health checks, component tests and candidate starts do not prove uninterrupted real business traffic. Platform release checks remain those selected by repository commands.

Remaining support and acceptance requirements require separate implementation and verification; a design alone does not enable a product capability.

# a3s-runtime

`a3s-runtime` defines the provider-neutral execution contract used by A3S
control planes. It separates stable execution semantics from providers such as
local Docker, `a3s-box`, and A3S OS.

The crate owns:

- immutable execution request and result types;
- Candidate and Judge role invariants;
- Runtime capability discovery;
- idempotent submit, inspect, and cancel operations;
- typed checkpoint, submission, protected-result, usage, and evidence records.

Provider implementations live behind `A3sRuntimeClient`. Callers must never
branch on provider names to weaken execution semantics.

This initial contract does not yet publish a production provider. Docker and
`a3s-box` adapters will be added only when they implement the same lifecycle
and evidence contract.

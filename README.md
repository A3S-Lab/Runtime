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

Provider selection is shared as well. An explicit operator provider takes
precedence over authenticated session policy; when neither exists, a signed-out
local caller selects Docker. Provider IDs are normalized, portable identifiers,
not executable paths or shell commands. Selecting an unavailable explicit
provider must fail rather than fall back to Docker.

`RuntimeClientRegistry` maps those IDs to typed `RuntimeProviderFactory`
objects. Duplicate registrations are rejected without replacement, and a
missing explicit provider fails without consulting the Docker factory. A
factory owns all provider-specific configuration and dependencies and can only
return the common `A3sRuntimeClient` interface.

`FileOperationStore` provides the durable idempotency boundary shared by local
providers. It publishes owner-only records atomically under a cross-process
file lock, rejects symlink state roots, returns the original queued handle for
an identical repeated reservation, and reports `OperationConflict` when an
existing operation ID is reused with another canonical spec digest. Updates
preserve execution/spec/role identity and allow only forward lifecycle
transitions; terminal records cannot be replaced.

This initial contract does not yet publish a production provider. Docker and
`a3s-box` adapters will be added only when they implement the same lifecycle
and evidence contract.

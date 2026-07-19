# ADR 0002: Complete Protocol and Operation Semantics

- Status: Accepted
- Date: 2026-07-17
- Decision owners: A3S Runtime maintainers

## Context

ADR 0001 established the general Task and Service model, immutable generations,
durable request identity, and provider-neutral ownership boundary. Source and
test-plan review found several places where the public API does not yet provide
enough information to prove those guarantees:

- some top-level wire types have no schema identifier;
- capability provider identity uses a different grammar from `ProviderId` and
  is not checked against the selected driver;
- a newer generation can overwrite durable identity before the old provider
  resource has been reconciled;
- file locking protects individual writes but not the asynchronous provider
  operation between reservation and completion;
- Task outputs have no matching capability and no reported byte size;
- exec has a request ID but no durable replay receipt;
- request receipts are embedded in one record with a hard 10,000-entry limit;
- deadlines are checked before reservation but do not bound lock waiting and
  provider dispatch;
- log, exec, stop-after-loss, and operation-result state semantics are not
  explicit enough to form complete test oracles.

These are pre-1.0 breaking changes. They are resolved together so provider and
consumer integrations migrate once to one coherent contract.

## Decision

### 1. Every top-level wire type is explicitly versioned

The following top-level values carry a `schema` field and reject unknown
fields:

- capabilities;
- unit specifications and apply/action requests;
- observations, inspections, and removals;
- log queries and log chunks;
- exec requests and exec results;
- durable unit records and request receipts.

Nested value objects remain versioned by their enclosing top-level schema.
Making `ephemeral_storage_bytes` optional changes unit specifications to
`a3s.runtime.unit-spec.v2`; v1 required a numeric quota. An omitted quota is
encoded explicitly as `null` in v2 and requires no ephemeral-storage
capability. Adding `size_bytes` to output artifacts changes observations to
`a3s.runtime.observation.v2`. Adding typed provider identity and output
capability changes capabilities to `a3s.runtime.capabilities.v3`. The durable
request-journal layout uses `a3s.runtime.unit-record.v2`. Persisting the
effective request deadline changes request receipts to
`a3s.runtime.request-receipt.v2`.

The Runtime core does not silently reinterpret legacy records or receipts. A
caller that needs old state owns an explicit archival decoder or migration
before starting the new client.

### 2. Provider identity is one typed value

`RuntimeCapabilities.provider_id` uses `ProviderId`, serialized as its existing
lowercase string representation. `RuntimeDriver` reports its stable
`ProviderId`, and `ManagedRuntimeClient` rejects capabilities whose ID differs
from the selected driver.

`RuntimeProviderFactory` construction becomes asynchronous. The registry
validates the created client's capabilities and verifies that the reported ID
matches the registered factory before returning the client. Observations do
not repeat the provider ID: the validated selected client is the authority,
while `provider_resource_id` remains provider-scoped identity.

### 3. One unit ID has one converged provider generation

Applying generation N durably makes N the desired generation before provider
dispatch. A driver must reconcile provider resources so that, when apply
returns successfully, exactly one resource generation for that unit remains.
Older generations are retired idempotently by the driver.

A transient overlap is permitted while the provider performs a handoff, but a
crash and exact retry must converge it to one resource. A caller that needs two
generations alive simultaneously, such as a rolling deployment, uses distinct
unit IDs and owns traffic switching above Runtime.

This rule lets a provider discover stale resources from provider labels or
equivalent metadata even though the core has already persisted the new desired
record.

### 4. A state store supplies a cross-process operation lease

`RuntimeStateStore` exposes an owned, asynchronous, per-unit operation lease.
`ManagedRuntimeClient` holds it across reservation, provider dispatch, and
durable completion for apply, inspect, stop, remove, logs, and exec.

The file store implements the lease with a separate owner-only advisory-lock
file. Record writes continue to use a shorter record lock, so completion does
not recursively acquire the operation lock. A process crash or cancelled
future releases the lease through operating-system file-handle cleanup.

Same-unit operations are serialized across tasks and processes. Different unit
IDs remain parallel. Distributed stores must implement an equivalent fenced
lease; a no-op implementation is not conformant.

### 5. Recovery identity changes only after confirmed loss

An ambiguous acknowledgement leaves a pending receipt. Retrying it must
discover the same provider resource and may not substitute identity.

If inspect proves that a previously observed provider resource is absent, the
core durably records `unknown`. A later same-generation apply may adopt one new
provider resource ID. Once that observation is completed, exact replay returns
the replacement without another provider create.

`unknown` is a confirmed-loss state, not a second `accepted` state. Recovery
may move from `unknown` to a valid provider-backed intermediate or operation
result and may adopt a replacement provider identity, but it cannot regress to
`accepted`.

### 6. Operation postconditions are explicit

Apply returns a provider-backed observation that has advanced beyond
`accepted`:

- a Task returns `succeeded` or `failed`;
- a Service returns `running`, `stopped`, `failed`, or `unknown`;
- `preparing`, `starting`, and `stopping` are inspectable intermediate states,
  not successful apply results.

A Service may be `running` but unhealthy. That is a truthful result and does
not satisfy `converges`.

Stop returns `stopped`, an already terminal observation, or durable `unknown`
when the provider resource is confirmed lost. It may not report a still-active
state as a successful stop result. Stop never recreates an unknown resource.

The shared conformance successful fixtures must converge; negative fixtures
also verify failed and unhealthy results.

### 7. Task outputs are capability-gated and exact

`RuntimeFeature::OutputArtifacts` advertises Task output collection. A Task
with nonempty `outputs` is rejected before reservation unless the feature is
present.

`RuntimeOutputArtifact` includes `size_bytes`. A succeeded Task that requested
outputs must report exactly the requested names. Every artifact media type must
match its output specification, every artifact is digest-bound, and
`size_bytes` must not exceed `max_bytes`. A provider without output collection,
including the current Docker driver, does not advertise the feature.

`IsolationLevel::Confidential` additionally requires the attestation feature.
Usage remains optional observation data unless a future specification requests
it explicitly.

### 8. Logs and exec have different state policies

Logs are readable for the current, non-removed generation in any lifecycle
state, including terminal and unknown. This permits postmortem logs. The
provider may return `NotFound` when retained provider logs no longer exist.
Removal closes the Runtime log surface for that unit generation.

Exec requires the current, non-removed generation to be `running`. It is a
mutating request and therefore participates in durable request replay:

- reserve an exec receipt before dispatch;
- an exact completed retry returns the stored result without re-execution;
- an ambiguous failure leaves the receipt pending;
- retrying a pending exec uses the same request ID, and an Exec-capable driver
  must deduplicate or reattach that request;
- conflicting reuse of the request ID fails before provider dispatch.

Exec results are stored in the request journal rather than embedded in the
unit record because bounded output may still be large.

### 9. Request receipts use a durable per-request journal

The v2 file layout separates the active unit record from receipts:

```text
state root/
├── locks/
├── operations/
└── units/
    └── <unit-key>/
        ├── record.json
        └── requests/
            └── <request-key>.json
```

Pending and completed receipts are atomically written and owner-only. Exact
replay is guaranteed until an explicitly removed unit is purged through a
future administrative retention operation. Normal lifecycle methods never
silently discard receipts, and there is no 10,000-operation availability
cliff.

The public unit record does not need to load every historical result to perform
an operation. Reservations return the one relevant receipt. Tests can enumerate
the journal through a test-only or administrative inspection surface.

### 10. Deadlines cover queueing and provider work

Managed operations validate the deadline before capability work, again after
acquiring the operation lease, and use the remaining duration to bound the
provider future. A timeout returns `DeadlineExceeded` and leaves a dispatched
mutating request pending because provider acknowledgement is ambiguous.

An exact replay whose receipt is already `completed` returns that durable
result before capability or deadline checks, including after the original
absolute deadline and after later lifecycle operations. The core reacquires
the unit lease and lets the state store reconcile a receipt-first crash before
returning. Deadlines constrain unfinished work; they do not invalidate an
already committed response. A pending replay remains subject to its original
deadline and is never redispatched after that deadline expires.

The request receipt stores the effective absolute deadline captured on first
reservation. For Exec this is the smaller of the first attempt's relative
timeout and optional absolute deadline; a retry cannot restart that relative
timeout window. Before provider dispatch, `ManagedRuntimeClient` replaces the
driver-bound exec request's optional caller deadline with that persisted
effective absolute deadline. The driver therefore receives the same non-null
`deadline_at_ms` on the first dispatch and every pending replay.

Drivers may enforce a shorter provider-specific timeout. They must never extend
the caller deadline. Exec uses the smaller of its relative `timeout_ms` and an
optional absolute request deadline.

Logs and inspect remain read operations without a request deadline in this
version; provider adapters must still have a configured transport timeout.

## Conformance consequences

The shared conformance suite is split into Base, Recovery, Networking, Mounts,
Health, Resources, Logs, Exec, Security, Outputs, and Evidence profiles.

- Base and Recovery are mandatory for every production provider.
- An advertised optional capability activates its corresponding profile.
- A provider job fails if a required fixture or provider prerequisite is
  absent; it does not silently pass by returning early.
- Generation advancement, operation cancellation, and all mutating crash
  windows include provider inventory checks.
- Successful cleanup must return provider and state inventory to the declared
  baseline.

## Consequences

- The public protocol and durable state schema break once before 1.0.
- Provider adapters must add typed identity, generation reconciliation,
  operation timeout handling, and idempotent exec when advertised.
- State-store implementations gain a cross-process lease and separate receipt
  journal.
- Exact replay no longer has an arbitrary embedded-record limit.
- Docker truthfully rejects requested outputs and exec until those capabilities
  are implemented.
- A3S Box can implement `IsolationLevel::Sandbox` behind the same contract
  without adding Box-specific fields.
- Tests can now derive a deterministic oracle for every public operation.

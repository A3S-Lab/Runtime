# A3S Runtime

<p align="center">
  <strong>Provider-Neutral Task and Service Runtime for A3S</strong>
</p>

<p align="center">
  <em>Converge immutable workload generations across local, container, sandbox, and remote execution providers</em>
</p>

<p align="center">
  <a href="#overview">Overview</a> •
  <a href="#features">Features</a> •
  <a href="#runtime-model">Runtime Model</a> •
  <a href="#operations">Operations</a> •
  <a href="#durable-state">Durable State</a> •
  <a href="#provider-integration">Providers</a> •
  <a href="#conformance">Conformance</a> •
  <a href="#development">Development</a>
</p>

---

## Overview

**A3S Runtime** defines a general execution contract for finite Tasks and
long-running Services. A caller submits an immutable unit generation, a provider
materializes it, and observations show whether provider state has converged with
the requested specification.

Runtime owns provider-neutral lifecycle semantics, capability discovery,
durable unit identity, and idempotent request handling. Scheduling, deployment
workflows, routing, product profiles, and provider-selection policy remain in
their owning applications.

## Features

- **One General Unit Model**: Run finite Tasks and long-running Services through
  the same client and provider interface
- **Immutable Generations**: Bind every operation and observation to a unit ID,
  positive generation, and canonical specification digest
- **Idempotent Lifecycle**: Apply, inspect, stop, and remove units with durable
  request receipts and deterministic conflict detection
- **Structured Capabilities**: Match artifact, isolation, network, mount,
  health, resource, and optional feature requirements before provider dispatch
- **Provider-Neutral Inputs**: Describe processes, artifacts, mounts, secret
  references, resources, networking, health checks, restart policy, and outputs
- **Observed Convergence**: Keep desired specifications separate from provider
  observations, usage, evidence, output artifacts, and failure details
- **Durable Local State**: Persist owner-only records atomically under
  cross-process locks without following symbolic-link state boundaries
- **Logs and Exec**: Expose generation-bound log and exec surfaces only when a
  provider reports the corresponding capability
- **Conformance Suite**: Exercise the common Task and Service lifecycle against
  provider-owned disposable resources

## Runtime Model

### Unit classes

| Class | Lifecycle | Typical work |
| --- | --- | --- |
| `Task` | Finite; converges at `succeeded` | Build, migration, evaluation, backup |
| `Service` | Long-running; converges at `running` and healthy when configured | Application, Agent, MCP server |

`RuntimeUnitSpec` is immutable for a `(unit_id, generation)` pair. Changing any
field requires the next generation. Reusing a generation with different content
fails with `GenerationConflict`; submitting an older generation fails with
`StaleGeneration`.

### Specification

A unit specification includes:

- a digest-bound artifact reference and media type;
- command, arguments, working directory, and environment;
- artifact, volume, and temporary-filesystem mounts;
- provider-resolved secret references and delivery targets;
- network mode, named ports, and transport protocols;
- CPU, memory, process, ephemeral-storage, and optional execution limits;
- isolation level, health probe, restart policy, and Task outputs;
- an optional digest binding caller-owned execution semantics.

All wire records use explicit schema identifiers and reject unknown fields.
Protocol validation occurs before state reservation or provider work.

### Observations

`RuntimeObservation` binds provider state to the exact unit ID, generation,
class, and specification digest. It can carry stable provider identity, health,
resource usage, output artifacts, evidence, attestation, and structured failure.

Terminal observations are immutable. If a previously observed provider resource
cannot be found, inspection records `unknown`; it does not silently report
success or erase the last provider identity.

## Operations

The `RuntimeClient` contract exposes:

| Operation | Semantics |
| --- | --- |
| `capabilities` | Return and validate the provider's structured capabilities |
| `apply` | Create, reattach, or converge one immutable unit generation |
| `inspect` | Return the latest observation or a generation-aware absence |
| `stop` | Stop the active generation without deleting durable identity |
| `remove` | Remove the provider resource and persist an absence tombstone |
| `logs` | Read strictly ordered, cursor-addressed log chunks |
| `exec` | Execute a bounded command against the exact active generation |

Each mutating request carries its own request ID and optional absolute deadline.
An exact retry returns or reconstructs the same logical result. Reusing a
request ID with different content fails with `RequestConflict`. A deadline is
checked independently before provider dispatch.

## Capabilities

Providers report supported unit classes, artifact media types, isolation
levels, network modes, mount kinds, health probes, resource controls, and
optional features. `ManagedRuntimeClient` validates the complete specification
against those capabilities before reserving state, so unsupported work cannot
leave a pending record or partially created provider resource.

The registry maps explicit `ProviderId` values to typed factories. It does not
choose a provider, infer login state, or fall back to a default. Callers own that
policy and pass the selected ID to `RuntimeClientRegistry::connect`.

## Durable State

`FileRuntimeStateStore` provides the local durable boundary for a managed
provider integration:

```text
state root/
├── locks/                   # per-unit cross-process locks
└── units/                   # atomic JSON records and request receipts
```

The store uses a SHA-256 storage key derived from the validated unit ID. Records
are written through an owner-only temporary file, synchronized, atomically
published, and followed by a directory sync. State directories, lock files, and
records reject symbolic-link boundaries; Unix permissions are tightened to
`0700` for directories and `0600` for files.

Pending receipts deliberately survive ambiguous transport failures. Retrying
the same apply request dispatches the same durable unit identity so an
idempotent provider driver can discover or converge the existing resource.

## Provider Integration

A provider implements `RuntimeDriver` and reports a stable `ProviderId` through
a `RuntimeProviderFactory`. The driver receives validated specifications and
durable unit records; it never owns request conflict or generation policy.

`ManagedRuntimeClient` composes three replaceable boundaries:

```text
RuntimeClient
    |
    v
ManagedRuntimeClient
    ├── RuntimeStateStore   durable identities and receipts
    ├── RuntimeDriver       provider resource lifecycle
    └── RuntimeClock        deadline and observation time source
```

Provider `apply` must be idempotent for the supplied unit ID and generation.
After an ambiguous acknowledgement, a repeated call must discover or converge
the same resource rather than create another one. Provider-specific labels,
SDK handles, container fields, and transport details stay behind the driver.

## Conformance

Provider repositories should run `verify_runtime_provider` against real,
disposable infrastructure:

```rust,ignore
use a3s_runtime::{verify_runtime_provider, RuntimeConformanceCase};

let case = RuntimeConformanceCase {
    task_apply,
    task_remove,
    service_apply,
    service_stop,
    service_remove,
};

let report = verify_runtime_provider(client.as_ref(), &case).await?;
assert!(report.task.converges(&task_spec));
assert!(report.service.converges(&service_spec));
```

The shared suite validates capability matching, exact apply replay, inspection,
stop replay, removal replay, and generation-aware absence for both lifecycle
classes. Provider repositories remain responsible for crash injection,
reconstruction, provider-specific security, and resource-leak tests.

See the [deep test plan](docs/deep-test-plan.md) for the full contract,
durability, real-provider, fault, performance, soak, and A3S OS release gates.

## Architecture

The contract and managed lifecycle are intentionally independent of product and
provider concerns:

```text
caller policy and workflow
          |
          v
provider-neutral Runtime contract
          |
          v
managed durability and validation
          |
          v
provider driver and external runtime
```

See [ADR 0001](docs/adr/0001-general-runtime-contract.md) for the general
ownership model, [ADR 0002](docs/adr/0002-complete-protocol-and-operation-semantics.md)
for the completed protocol and operation semantics, and the
[implementation plan](docs/implementation-plan.md) for the dependency-ordered
delivery tasks.

## Development

Run validation from this crate repository, not from the A3S monorepo root:

```bash
cargo fmt --all --check
cargo test --all-targets
cargo clippy --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps
```

The integration suite covers Task and Service behavior, request and generation
conflicts, ambiguous retry, capability rejection, provider identity,
generation-bound logs and exec, independent deadlines, provider disappearance,
terminal immutability, concurrent file reservations, symbolic-link rejection,
registry behavior, and the exported provider conformance path.

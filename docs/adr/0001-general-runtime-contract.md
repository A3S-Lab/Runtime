# ADR 0001: General Task and Service Runtime Contract

- Status: Accepted
- Date: 2026-07-14
- Decision owners: A3S Runtime maintainers

## Context

The initial Runtime API encoded product-specific evaluation roles, result
shapes, provider precedence, and a default local provider. That made the core
unsuitable for application services, build tasks, migrations, Agents, MCP
servers, and other callers with different policies.

A3S Cloud needs one execution boundary that can converge finite and
long-running work across multiple providers. It also needs to recover after a
lost acknowledgement without changing provider identity or creating duplicate
resources.

## Decision

Runtime exposes one immutable `RuntimeUnitSpec` with two lifecycle classes:
`Task` and `Service`. The same `RuntimeClient` supports capability discovery,
apply, inspect, stop, remove, logs, and exec.

The identity tuple is:

```text
(unit_id, generation, canonical_spec_digest)
```

Every mutating command also carries a caller-generated request ID. Exact replay
returns or reconstructs the same logical result. Reusing a request ID with
different content, reusing a generation with different content, or submitting a
stale generation fails deterministically.

`ManagedRuntimeClient` owns validation, capability matching, receipt
reservation, transition validation, and durable completion. `RuntimeDriver`
owns only provider resource operations. A driver must treat apply as idempotent
for the supplied unit identity and must preserve a stable provider resource ID.

Provider selection belongs to callers. Runtime retains typed provider IDs,
factories, and a registry, but the registry never chooses a default or falls
back when an explicit provider is unavailable.

Product profiles may bind extra semantics through an immutable digest, but
their domain fields and validation do not enter the Runtime protocol.

## State and failure semantics

The file store persists one versioned unit record containing the current
specification, latest observation, removal tombstone, and bounded request
receipts. Atomic owner-only writes and per-unit cross-process locks protect the
local durability boundary.

A transport error after dispatch is ambiguous. The receipt remains pending and
an exact retry re-dispatches the same unit identity so the provider can discover
or converge it. A missing resource that was previously observed becomes
`unknown`; absence is definitive only for a never-recorded or explicitly
removed unit. Terminal observations cannot be mutated.

## Capability semantics

Capabilities are structured sets rather than closed product predicates. A
provider reports supported unit classes, artifact media types, isolation,
networking, mounts, health probes, resource controls, and optional features.
The complete requirement set is checked before durable reservation and provider
dispatch.

## Migration

Version 0.2 is a breaking pre-1.0 contract. General Runtime records use the
`a3s.runtime.unit-record.v1` schema and are not an in-place reinterpretation of
records written by the earlier product-shaped API. Callers that retain older
terminal records own their archival decoder and presentation. New operations
must create general unit records; the Runtime core does not silently rewrite or
resume an older record as a general unit.

## Consequences

- Cloud, Bench, and future callers can share lifecycle and durability semantics
  without sharing product policy.
- Providers must implement stable discovery and idempotent apply, not only
  one-shot process launch.
- Callers must make provider selection and product validation explicit.
- Provider repositories can use one exported conformance suite while retaining
  provider-specific crash, security, and leak tests.
- The pre-1.0 API break requires known consumers to migrate before release.

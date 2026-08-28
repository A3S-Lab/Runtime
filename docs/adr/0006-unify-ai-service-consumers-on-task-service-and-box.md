# ADR 0006: Unify AI Service Consumers on Task, Service, and A3S Box

- Status: Accepted
- Date: 2026-08-28
- Decision owners: A3S Runtime maintainers

## Context

A3S Cloud exposes Agent as a Service, Workflow as a Service, and Function as a
Service, supports Durable Cells, and hosts sessionless MCP services. Each has
different product invariants, but their executable processes need the same
identity, generation, replay, health, endpoint, resource, recovery, and cleanup
mechanics.

Adding `Agent`, `Workflow`, `Function`, `Mcp`, or `Cell` Runtime unit classes
would couple the provider protocol to Cloud domains and create parallel
lifecycle implementations. Collapsing the product aggregates into one generic
execution aggregate would instead erase their authorization and recovery
boundaries.

A3S Box is the production process and sandbox provider. A3S Gateway is the
public traffic boundary. Neither should acquire Cloud product semantics.

## Decision

### 1. Runtime keeps exactly two executable classes

A finite executable is a `Task`. A continuously available executable is a
`Service`. Product identity is bound through an opaque immutable semantics
profile digest, not a product-specific wire field.

A Workflow is not itself a Runtime unit. A3S Flow owns Workflow durability;
Agent and Function nodes delegate only their executable child to Runtime.

### 2. Consumers compose one generic requirements abstraction

Runtime exports `RuntimeConsumerRequirements` outside the wire-contract module.
It admits a specification against provider capabilities and accepts ready
observations only from exact semantics, health, and endpoint evidence selected
by the consumer.

The abstraction reuses `RuntimeCapabilities`, `RuntimeUnitSpec`, and
`RuntimeObservation`. It introduces no registry, scheduler, receipt, endpoint
store, or second validation protocol.

### 3. A3S Box is the production provider baseline

The supported production chain is Cloud to Runtime to Box. Cloud may depend on
the Runtime client and contracts, but product domains must not invoke Box
process APIs directly. Box implements provider mechanics behind
`RuntimeDriver` and must pass Base, Recovery, and every advertised conformance
profile before a product gate can close.

Runtime remains provider-neutral so behavior can be certified independently
and alternate providers can be evaluated without changing Cloud's domains.
Provider neutrality is an isolation boundary, not a second execution engine.

### 4. Gateway remains the only public traffic boundary

Runtime Service endpoints are internal, generation-bound evidence. Cloud admits
them into Edge snapshots; Gateway authenticates, applies protocol and traffic
policy, routes, streams, and drains. Runtime and Box never publish a public
route.

### 5. Product mappings remain explicit

- stateful Agents use a fenced Service pool; bounded batch Agents may use Task;
- finite hosted Functions use Task;
- low-latency stateless Functions and sessionless MCP use Service;
- external FaaS uses a Cloud Connector attempt and no local Runtime unit;
- a Durable Cell application replica uses Service, while an individual named
  Cell is not a Runtime unit; and
- Workflow uses no unit for its orchestration and composes child owners.

## Consequences

- AaaS, WaaS, FaaS, MCP, and Durable Cell share lifecycle mechanics without
  sharing or weakening their domain models.
- Providers implement and certify one Task/Service contract.
- Cloud and Gateway can fail closed on exact provider evidence through one
  admission abstraction.
- A product cannot claim availability from a fixture alone; real Box, Cloud,
  Gateway, recovery, cleanup, and product gates remain mandatory.
- New product Runtime classes, public endpoint registries, and direct
  Cloud-to-Box control paths are architectural violations.

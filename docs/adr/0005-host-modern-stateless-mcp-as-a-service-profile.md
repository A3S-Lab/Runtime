# ADR 0005: Host Modern Stateless MCP as a Service Profile

- Status: Proposed
- Date: 2026-07-30
- Decision owners: A3S Runtime maintainers

## Context

MCP revision `2026-07-28` removes the initialization handshake and
protocol-level sessions. Each Streamable HTTP request carries its protocol
version, client capabilities, and routing headers; `clientInfo` is recommended
self-reported metadata, not an authenticated identity. A server must implement
`server/discover`. This makes requests protocol-stateless, but it does not make
the server process ephemeral, side-effect-free, or safe to replay.

A3S Cloud must deploy and reconcile hosted MCP services, and A3S Gateway must
expose their public protocol. Runtime already has a durable Service lifecycle,
opaque product-profile binding, typed TCP endpoints, health, logs, resources,
and provider conformance. Adding MCP methods or product fields to that core
would duplicate Gateway and Cloud responsibilities and make the general
contract protocol-specific.

## Decision

### 1. A hosted MCP replica is an ordinary Runtime Service

Each replica uses `UnitClass::Service`, one immutable specification, one
positive generation, and one distinct Unit ID. Replicas never share a Unit ID.
Rolling updates use distinct Unit identities or an explicit generation
transition under the existing Runtime rules.

Protocol statelessness does not change durable Runtime receipts, observations,
provider discovery, or reconciliation.

### 2. The MCP profile stays outside the Runtime wire protocol

A3S Cloud owns an immutable Service-profile ACL for the MCP release and a
separate mutable Gateway route-policy ACL. The Service profile covers protocol
versions, server endpoint path, capability expectations, and server bounds;
the route policy covers public Origin, authorization, grants, effective
limits, telemetry, and expiry. Cloud compiles only generic process, network,
health, resource, mount, Secret, and port requirements into
`RuntimeUnitSpec`.

Runtime binds the immutable MCP profile through
`semantics_profile_digest`. It does not decode, default, or validate the
profile's domain fields. The mutable route policy never enters Runtime or
forces a Service restart. The same Service-profile digest must be present in
the exact observation Cloud uses to publish a target.

### 3. Runtime owns the provider lifecycle, not the MCP request path

The provider creates, discovers, fences, recovers, and removes the Service and
its typed loopback endpoint. Runtime never parses MCP JSON-RPC, negotiates a
protocol version, answers `server/discover`, authenticates a request, selects a
replica, frames SSE, or persists tool state.

Cloud coordinates target removal and Gateway drain before Runtime stop.
Runtime does not implement a second drain or routing controller.

### 4. Certification uses a real MCP server as a black-box consumer

The Runtime consumer gate launches a pinned modern MCP fixture through A3S Box
and proves that the typed endpoint reaches the expected fixture, including a
`server/discover` request. The MCP protocol assertions belong to the joint
Gateway/Cloud conformance harness; Runtime assertions remain lifecycle,
identity, endpoint, recovery, resource, and cleanup assertions.

The fixture does not introduce MCP types into `src/contract` or make Runtime
authoritative for tools, resources, prompts, or application state.

## Consequences

- Existing Runtime providers can host MCP without a new provider API or unit
  class.
- Cloud has one place to validate assets, profiles, replicas, and desired
  state.
- Gateway has one place to enforce modern MCP transport and request policy.
- A semantics or protocol change creates a new immutable Cloud profile/release
  without reinterpreting a Runtime record.
- Runtime can certify the substrate independently while the complete product
  claim still requires pinned Cloud, Gateway, Box, and fixture evidence.
- Legacy session-based MCP, if required, needs a separate compatibility gate;
  it is not inferred from the modern Service profile.

## References

- [MCP versioning and compatibility, revision 2026-07-28](https://modelcontextprotocol.io/specification/2026-07-28/basic/versioning)
- [MCP Streamable HTTP, revision 2026-07-28](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/streamable-http)
- [MCP server discovery](https://modelcontextprotocol.io/specification/2026-07-28/server/discover)

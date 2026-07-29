# ADR 0004: Type Service Endpoints and Protocol Capabilities

- Status: Accepted
- Date: 2026-07-29
- Decision owners: A3S Runtime maintainers

## Context

`NetworkMode::Service` previously activated both TCP and UDP conformance cases
even when a provider could safely publish only one transport. Consumers also
encoded provider endpoints in product-specific evidence keys. That duplicated
wire grammar, allowed synthetic endpoint publication outside the provider, and
made a provider either overstate UDP support or reject useful TCP Services.

A3S Box already has one generation-fenced execution port connector. The Runtime
contract needs to expose the resulting node-local endpoint without importing a
Box type, adding another lifecycle store, or making a control-plane consumer
own a forwarding process.

## Decision

### 1. Capabilities v4 advertises Service transports exactly

`RuntimeFeature::ServiceTcp` and `RuntimeFeature::ServiceUdp` independently
advertise the transports a provider can publish for `NetworkMode::Service`.
Capabilities schema `a3s.runtime.capabilities.v4` requires Service mode and at
least one Service transport feature to appear together. Specification matching
rejects every declared port whose protocol is not advertised before state
reservation or provider work.

The Networking conformance profile activates `NETWORK-PROTOCOL-TCP` and
`NETWORK-PROTOCOL-UDP` only for the corresponding feature. Service mode still
requires exact loopback publication and collision behavior.

### 2. Runtime owns one typed endpoint evidence grammar

`RuntimeServiceEndpoint` contains the declared port name, transport protocol,
literal loopback address, and positive host port. Its canonical evidence key is
`a3s.runtime.service-endpoint.<port-name>` and its canonical value is a TCP or
UDP socket URI. Constructors, parsing, insertion, and observation lookup live
in Runtime; providers and consumers must not define another prefix or parser.

Endpoint data uses the existing `RuntimeEvidence.claims` extension boundary.
That keeps it bound to the observation's provider build, specification digest,
and optional semantics profile without changing the v2 observation envelope or
durable record schema. Capabilities change to v4 because protocol support
changes admission behavior and is not an opaque evidence extension.

### 3. Lifecycle validation is exact

A running Runtime Service with `NetworkMode::Service` reports exactly one typed
endpoint for every declared port and the transport must match. Endpoint sockets
must be unique. Tasks, non-running Services, and non-Service network modes may
not report endpoints. Transitioning to an unknown or terminal state removes
endpoint claims while preserving unrelated evidence claims.

The provider owns listener creation, recovery, generation fencing, and cleanup.
A consumer may compile routing or health policy from the typed observation, but
it may not invent an endpoint, start a second forwarding daemon, or persist an
alternate endpoint registry.

## Consequences

- TCP-only providers can truthfully advertise Service networking without
  claiming UDP behavior.
- Endpoint publication is provider-neutral and reusable by health, routing, and
  other consumers.
- Existing arbitrary evidence claims remain available, but the Runtime endpoint
  namespace is validated and has one implementation.
- Provider conformance must prove real forwarding, collision handling,
  generation replacement, restart recovery, and endpoint cleanup for every
  advertised transport.

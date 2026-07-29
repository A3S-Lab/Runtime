# A3S Runtime Roadmap

## 1. Scope and authority

**Status as of 2026-07-30.**

This roadmap describes work owned by the A3S Runtime repository. The
[implementation plan](docs/implementation-plan.md) owns task order, and the
[deep test plan](docs/deep-test-plan.md) owns release evidence. Cross-product
ordering and the public state of `MCP0` are owned by the
[A3S Cloud product roadmap](https://github.com/A3S-Lab/Cloud/blob/main/ROADMAP.md).

The roadmap is gate-driven. Code, a mock, or an environment-skipped test does
not make a capability available.

## 2. Product responsibility

**A3S Runtime is the provider-neutral, durable lifecycle boundary for one Task
or Service unit.**

Runtime owns:

- immutable Unit identity, generation, specification digest, and request
  replay;
- apply, inspect, stop, remove, logs, and bounded unary exec;
- capability admission and provider-neutral health, resource, network, mount,
  output, and evidence contracts;
- typed, generation-bound Service endpoint observations;
- provider recovery, fencing, conformance, and cleanup evidence; and
- opaque binding of a product semantics profile through its immutable digest.

Runtime does not own:

- organizations, assets, releases, desired replicas, placement, rollout, or
  autoscaling;
- domains, TLS, authentication grants, route policy, or endpoint selection;
- MCP JSON-RPC parsing, protocol-version negotiation, `server/discover`, tool
  schemas, request authorization, or SSE framing;
- product-profile fields or validation in the Runtime wire protocol; or
- application state held by a hosted MCP server.

Those boundaries produce this ownership map:

| Concern | Runtime | A3S Cloud | A3S Gateway |
| --- | --- | --- | --- |
| One process or sandbox | Own lifecycle and provider evidence | Declare and reconcile desired Unit | Never create or stop it |
| MCP contract | Bind only the immutable Service-profile digest | Own the immutable Service-profile ACL, separate route-policy ACL, release, and validation | Consume both compiled projections |
| Replica set | Give every replica a distinct Unit identity | Own count, placement, rollout, and fencing | Balance only across the published healthy set |
| Public MCP request | No request-path role | Stay off the request path | Validate, authorize, route, stream, and drain |
| Tools/resources/prompts | No protocol semantics | Admit and pin the server release | Relay; do not invent server capabilities |

## 3. Current capability truth

| Area | Current state | Decision |
| --- | --- | --- |
| General Task and Service contract | Implemented foundation; the complete `R00`-`R22` release audit remains authoritative | Preserve one generic lifecycle contract |
| Typed TCP Service endpoints | Implemented by ADR 0004 and bound to provider build, specification digest, and generation | Reuse for hosted MCP; do not add an MCP endpoint registry |
| Product semantics binding | Opaque `semantics_profile_digest` is part of the existing contract | Cloud owns the MCP profile content; Runtime verifies only the binding |
| A3S Box provider certification | Incomplete until the advertised Box profiles and clean-host evidence pass | `MCP0` cannot bypass Box certification |
| Hosted MCP Service substrate | Consumer-profile foundation implemented: ADR boundary, canonical Runtime Unit fixture, semantics-profile digest equality, typed endpoint/generation acceptance, and stale evidence rejection pass focused tests. Real Box/Linux workload and recovery certification remain open | Keep MCP a black-box Service consumer and proceed to `RMCP-03`; do not claim `MCP0.2` before real provider evidence |
| Native MCP protocol handling | Not implemented by design | Belongs to Gateway and the hosted server |

## 4. `MCP0` contract

The first hosted MCP baseline is the modern, stateless protocol revision
`2026-07-28`. It has no initialization handshake or protocol session. Every
request declares its version and capabilities; `clientInfo` is recommended
request metadata rather than an authenticated identity. Every conforming
server implements `server/discover`.

This protocol change does not require a new Runtime unit class:

```text
Cloud MCP AssetRelease + ACL profile
  -> immutable Runtime Service spec + semantics profile digest
  -> Box starts one generation-fenced Service replica
  -> Runtime publishes one typed loopback TCP endpoint
  -> Cloud admits the exact healthy endpoint into a Gateway snapshot
  -> Gateway serves the public modern MCP transport
```

An MCP replica is long-running and therefore maps to `UnitClass::Service`.
Stateless MCP describes request protocol state, not Runtime lifecycle state.
Runtime receipts, generations, observations, logs, and provider reconciliation
remain durable.

The shared sequence is:

| Sub-gate | Owner and result |
| --- | --- |
| `MCP0.1` | Cloud freezes the cross-repository profile and fixture with Runtime and Gateway review |
| `MCP0.2` | Runtime and Box certify the generic hosted-Service substrate |
| `MCP0.3` | Cloud implements release admission, orchestration, replicas, rollout, policy compilation, and audit |
| `MCP0.4` | Gateway implements the modern MCP request data plane |
| `MCP0.5` | All repositories pass the exact-revision single-node release gate |
| `MCP0.6` | All repositories pass the multi-node production, recovery, load, and operations gate |

## 5. Runtime delivery plan

Runtime owns cross-product sub-gate `MCP0.2`.

| Task | Status | Work | Dependency | Exit evidence |
| --- | --- | --- | --- | --- |
| `RMCP-01` | Foundation complete (2026-07-30) | Accept the MCP Service boundary in ADR 0005 and freeze the exact Runtime-to-Cloud projection | ADR 0001 and ADR 0004 | No MCP field, parser, route, or session enters `src/contract` |
| `RMCP-02` | Foundation complete (2026-07-30) | Define a consumer certification profile from existing Service TCP, health, logs, resources, Secrets, and semantics-digest capabilities | `RMCP-01` | Canonical fixture manifest and stable case IDs |
| `RMCP-03` | Pending Linux/Box evidence | Run one real modern MCP server as a Box-hosted Runtime Service | `RMCP-02`, Box Base/Recovery/Networking/Health certification | Exact endpoint answers a black-box `server/discover` probe and provider inventory returns to baseline |
| `RMCP-04` | Planned | Prove generation replacement, process loss, host restart, external deletion, stop, and remove | `RMCP-03` | One final provider resource, no stale endpoint, deterministic `unknown`, and zero leak |
| `RMCP-05` | Planned | Prove distinct Unit identity and endpoint evidence for multiple replicas | `RMCP-04` | No shared Unit ID, port collision, profile mismatch, or cross-generation observation |
| `RMCP-06` | Planned | Run the pinned Runtime/Box/Cloud/Gateway consumer gate | `RMCP-03`-`RMCP-05`, Cloud `MCP0.3`, Gateway `MCP0.4` | Runtime evidence is accepted into joint `MCP0.5`; exact SHAs, fixture digests, observations, traffic results, and cleanup bundle pass |

`RMCP-03` uses MCP only as a real black-box workload. The probe verifies that
the endpoint reaches the expected fixture; it does not make Runtime an MCP
implementation or the authority for the fixture's advertised tools.

### Dependency order

1. Review and land the implemented `RMCP-01`/`RMCP-02` contract and fixtures
   with Cloud and Gateway at exact revisions.
2. Complete the Runtime core and Box capability gates required by the profile.
3. Land the smallest single-replica Box slice.
4. Add crash, generation, and multi-replica evidence.
5. Close `MCP0.2` only through the joint exact-revision gate.

## 6. Exit criteria

Runtime may claim an **MCP-ready Service substrate** only when:

- the Unit is an ordinary immutable Runtime Service;
- the MCP semantics profile is opaque to Runtime and digest-bound to the
  observation;
- Box advertises and passes every capability used by the Service;
- apply replay, agent loss, provider loss, and host restart do not create a
  second provider resource for one Unit generation;
- each replica has a distinct Unit ID and one exact typed endpoint;
- replacement removes stale endpoint evidence before Cloud can publish it;
- stop and remove restore provider inventory and release all listeners;
- Cloud and Gateway consume only exact-generation typed observations; and
- the evidence bundle records compatible Runtime, Box, Cloud, Gateway, and
  fixture revisions.

## 7. Non-goals

- Adding `McpTask`, `McpService`, or MCP protocol fields to the Runtime wire
  schema.
- Running an MCP router, authorization server, or protocol proxy in Runtime.
- Treating stateless requests as idempotent or retryable.
- Sharing one Runtime Unit ID across replicas.
- Letting Runtime discover assets, choose releases, schedule replicas, or
  publish public routes.
- Claiming support for legacy initialization/session-based MCP as part of the
  modern baseline.

## 8. Protocol references

- [MCP 2026-07-28 versioning and compatibility](https://modelcontextprotocol.io/specification/2026-07-28/basic/versioning)
- [MCP 2026-07-28 Streamable HTTP](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/streamable-http)
- [MCP server discovery](https://modelcontextprotocol.io/specification/2026-07-28/server/discover)

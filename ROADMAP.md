# A3S Runtime Roadmap

## 1. Scope and authority

**Status as of 2026-08-30.**

This roadmap describes work owned by the A3S Runtime repository. The
[implementation plan](docs/implementation-plan.md) owns task order, and the
[deep test plan](docs/deep-test-plan.md) owns release evidence. Cross-product
ordering and the public state of AaaS, WaaS, FaaS, Durable Cells, and `MCP0`
are owned by the
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
- opaque binding of a product semantics profile through its immutable digest;
- opaque binding of caller-owned identity attachment intent to exact provider
  evidence and attestation without parsing product policy;
- one composable `RuntimeConsumerRequirements` admission/readiness boundary;
  and
- the certified Runtime-to-A3S-Box provider boundary for the production path.

Runtime does not own:

- organizations, assets, releases, desired replicas, placement, rollout, or
  autoscaling;
- domains, TLS, authentication grants, route policy, or endpoint selection;
- Agent, Workflow, Function, MCP, model, or Durable Cell product semantics;
- JSON-RPC parsing, protocol-version negotiation, request authorization, or
  streaming framing;
- product-profile fields or validation in the Runtime wire protocol; or
- application or named state held by a hosted Service.

Those boundaries produce this ownership map:

| Concern | A3S Cloud | A3S Runtime | A3S Box | A3S Gateway |
| --- | --- | --- | --- | --- |
| Product service | Own AaaS/WaaS/FaaS and Durable Cell semantics | Bind only an opaque profile digest | No product semantics | Consume the admitted route/policy projection |
| One process or sandbox | Declare and reconcile desired Unit | Own lifecycle and provider evidence | Create, fence, observe, and remove the provider resource | Never create or stop it |
| Workflow | Own WorkflowRun and coordinate A3S Flow | Run only executable child Tasks/Services | Execute those child units | Route only published service endpoints |
| Replica set | Own count, placement, rollout, and fencing | Give every replica a distinct Unit identity | Enforce each exact unit generation | Balance only across the published healthy set |
| Public request | Compile target and policy snapshots; stay off request bytes | No request-path role | Expose node-local endpoint evidence only | Authenticate, authorize, route, stream, and drain |

## 3. Current capability truth

| Area | Current state | Decision |
| --- | --- | --- |
| General Task and Service contract | Implemented foundation; the complete `R00`-`R22` release audit remains authoritative | Preserve one generic lifecycle contract |
| Typed TCP Service endpoints | Implemented by ADR 0004 and bound to provider build, specification digest, and generation | Reuse for hosted MCP; do not add an MCP endpoint registry |
| Product semantics binding | Opaque `semantics_profile_digest` is part of the existing contract | Cloud owns every product profile; Runtime verifies only the binding |
| Workload identity attachment | Component complete (2026-08-30): unit-spec/observation v3 and capabilities v5 expose one opaque attachment digest, one typed attestation projection, and one composed consumer requirement | Cloud owns policy, Fleet/Claim composition, freshness, and issuance; Box must certify exact provider evidence |
| Unified consumer requirements | Implemented component foundation: class, generic feature, semantics evidence, health, and endpoint requirements compose through one API | Cloud consumers must adopt this API instead of duplicating generic admission checks |
| AaaS/FaaS/MCP/Durable Cell fixtures | Component fixtures pass for Agent Service, Function Task, stateless Function/MCP Service, and Durable Cell Service without new Runtime classes | Fixtures prove composition, not product availability |
| A3S Box provider certification | The Box repository implements `BoxRuntimeDriver` and capability-triggered conformance fixtures; exact-revision re-certification and profile gaps such as `Outbound` remain open | No Cloud product may bypass Box certification or infer an unadvertised capability |
| Hosted MCP Service substrate | Consumer-profile foundation implemented: ADR boundary, canonical Runtime Unit fixture, semantics-profile digest equality, typed endpoint/generation acceptance, and stale evidence rejection pass focused tests. Real Box/Linux workload and recovery certification remain open | Keep MCP a black-box Service consumer and proceed to `RMCP-03`; do not claim `MCP0.2` before real provider evidence |
| Native MCP protocol handling | Not implemented by design | Belongs to Gateway and the hosted server |

## 4. Unified Cloud service substrate

Runtime supports Cloud product profiles through composition, not inheritance:

| Cloud surface | Generic projection | Runtime delivery gate |
| --- | --- | --- |
| AaaS | Stateful Agent `Service`; bounded batch Agent may be `Task` | Exact semantics/readiness evidence plus Box recovery and workspace-fence proof |
| WaaS | No Workflow unit; Agent/Function/Execution nodes use their owner projection | Flow replay must not duplicate a child Runtime intent |
| FaaS | Finite `Task`, low-latency stateless `Service`, or no local unit for external FaaS | Task/Service Box profiles plus Cloud Connector evidence for external calls |
| Durable Cell | One application replica as `Service`; individual named Cell is not a unit | Runtime evidence plus provider state-lineage and writer-fence evidence |
| Sessionless MCP | Function-style stateless `Service` | Generic Service gate plus MCP/Gateway protocol conformance |

| Task | Status | Work | Exit evidence |
| --- | --- | --- | --- |
| `RCON-01` | Component complete (2026-08-28) | Export one provider-neutral consumer requirements abstraction | Focused positive and fail-closed tests pass; `src/contract` stays product-neutral |
| `RCON-02` | Component complete (2026-08-28) | Freeze Agent, Function Task/Service, MCP, and Durable Cell consumer fixtures | All fixtures use only `Task` or `Service` and exact semantics evidence |
| `RCON-03` | In progress | Pin Cloud to the exact Runtime revision and replace duplicate generic consumer checks | Cloud architecture and consumer tests prove one shared admission/readiness mechanism |
| `RWI-01` | Component complete (2026-08-30) | Bind one opaque identity attachment to the exact Runtime Unit generation and provider attestation | Golden schemas, fail-closed consumer tests, and ADR 0007 pass without product fields entering Runtime |
| `RWI-02` | In progress | Re-certify Box and pin Cloud to the exact attachment-aware Runtime revision | Box repeats the digest in generation-bound evidence; Cloud composes only the typed Runtime binding with owner ports |
| `RBOX-01` | Driver and conformance harness implemented; exact-revision evidence pending | Certify the A3S Box Runtime driver and close each required capability gap | Base, Recovery, and every advertised profile pass with baseline inventory restored |
| `RBOX-02` | Planned | Run exact-revision Gateway/Cloud/Runtime/Box compatibility gates for each service profile | Requests enter through Gateway, targets bind exact observations, failures recover, cleanup is complete |

See [Unified AI Service Runtime](docs/unified-ai-service-runtime.md) and
[ADR 0006](docs/adr/0006-unify-ai-service-consumers-on-task-service-and-box.md).

## 5. `MCP0` contract

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

## 6. MCP Runtime delivery plan

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

## 7. Exit criteria

Runtime may claim a **Cloud AI service substrate** only when:

- every product executable maps to `Task` or `Service` through one consumer
  admission/readiness abstraction;
- A3S Box passes Base, Recovery, and every advertised capability profile on a
  production-equivalent Linux host;
- Cloud never controls a Box process directly and Gateway never starts a Unit;
- exact profile, generation, spec, provider, health, and endpoint evidence is
  consumed before target publication;
- replay, process loss, provider loss, node loss, and host restart do not create
  duplicate provider resources or product intent;
- public traffic enters through Gateway from one Cloud Edge snapshot; and
- stop, remove, route drain, object/volume cleanup, and provider inventory all
  return to their declared baseline.

The following additional criteria apply to the MCP-ready profile.

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

## 8. Non-goals

- Adding `Agent`, `Workflow`, `Function`, `Mcp`, `Cell`, or model unit classes
  or product fields to the Runtime wire schema.
- Running an MCP router, authorization server, or protocol proxy in Runtime.
- Implementing A3S Flow, a Function scheduler, a Durable Cell state engine, or
  public routing inside Runtime.
- Allowing Cloud to invoke A3S Box outside the Runtime driver boundary.
- Treating stateless requests as idempotent or retryable.
- Sharing one Runtime Unit ID across replicas.
- Letting Runtime discover assets, choose releases, schedule replicas, or
  publish public routes.
- Claiming support for legacy initialization/session-based MCP as part of the
  modern baseline.

## 9. Protocol references

- [MCP 2026-07-28 versioning and compatibility](https://modelcontextprotocol.io/specification/2026-07-28/basic/versioning)
- [MCP 2026-07-28 Streamable HTTP](https://modelcontextprotocol.io/specification/2026-07-28/basic/transports/streamable-http)
- [MCP server discovery](https://modelcontextprotocol.io/specification/2026-07-28/server/discover)

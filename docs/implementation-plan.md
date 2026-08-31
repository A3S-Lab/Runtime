# A3S Runtime Implementation Plan

## Goal

Implement the complete contract accepted by ADR 0001 and ADR 0002, preserve
the bounded unary exec boundary accepted by ADR 0003, preserve the typed
Service endpoint ownership accepted by ADR 0004, certify all advertised
provider capabilities, and satisfy the release gates in the deep test plan. In
parallel with that foundation, freeze the proposed ADR 0005 boundary; certify
the modern MCP Service profile only after its required provider gates pass and
without importing MCP protocol or product fields into Runtime core. Work is
ordered by dependency. ADR 0006 additionally makes Runtime the single generic
Task/Service substrate for A3S Cloud AaaS, WaaS executable nodes, FaaS, hosted
MCP, and Durable Cell application Services, with A3S Box as the production
provider. A later task cannot claim completion from mocks when its required
real-provider evidence is absent.

## Working rules

- Preserve user changes and keep each task reviewable.
- Write or update tests before completing the corresponding implementation.
- Run Cargo, provider, fault, performance, and soak validation only on Linux CI
  or A3S OS. Do not run those workloads on a developer laptop.
- A3S OS obtains every repository through Git at an exact commit.
- Shared production nodes run only the bounded canary defined by the deep test
  plan. Destructive tests require a disposable production-equivalent worker.
- Code and documentation are English, and product configuration remains ACL.

## Task graph

| ID | Task | Depends on | Completion evidence |
| --- | --- | --- | --- |
| R00 | Freeze baseline and task manifest | None | Exact Runtime and consumer SHAs, dirty-file ownership, and capability inventory recorded |
| R01 | Version every top-level wire record | R00 | Golden JSON for valid/old/future/unknown-field cases; Cloud wire consumers compile |
| R02 | Unify and bind provider identity | R01 | Typed capability ID, driver/factory checks, mismatch tests, Docker migration |
| R03 | Complete output and confidential capability matching | R01 | Output/attestation rejection before state work; exact output validation tests |
| R04 | Define and enforce operation postconditions | R01 | Full Task/Service result-state matrix; log and exec state tests |
| R05 | Bound deadlines across queue and dispatch | R04 | Deterministic clock tests for pre-lock, post-lock, provider timeout, and pending replay |
| R06 | Add per-unit cross-process operation leases | R04 | Same-unit race serialization and different-unit parallelism in independent processes |
| R07 | Replace embedded receipts with request journal v2 | R06 | Atomic receipt tests, restart replay, 10,001 requests, permissions, corruption handling |
| R08 | Make bounded unary exec durably idempotent | R07 | Exact replay executes once; conflict, cancellation, timeout, and large-output tests |
| R09 | Enforce generation reconciliation | R06 | N to N+1 success/failure/crash tests with exactly one final provider resource |
| R10 | Build deterministic fault driver and transition generator | R01-R09 | Every state edge and provider boundary has a stable case ID and oracle |
| R11 | Split shared conformance into capability profiles | R10 | Base/Recovery mandatory; optional advertised profiles auto-run and cannot silently skip |
| R12 | Add Runtime repository CI and compatibility gate | R01-R11 | Required Linux MSRV/stable format, lint, docs, unit, integration, and golden checks |
| R13 | Certify Docker identity, lifecycle, and generation handling | R09-R12 | Real Docker Base and Recovery profiles pass; zero duplicate/leaked containers |
| R14 | Certify Docker network, mounts, health, resources, and logs | R11-R13 | Every advertised Docker capability has inspect plus behavioral evidence |
| R15 | Complete A3S Cloud consumer recovery paths | R13-R14 | Projection, journal, restart, redelivery, reconciliation, logs, cancel, and cleanup E2E pass |
| R16 | Integrate and maintain A3S Box RuntimeDriver | R11 | Typed `sandbox` capability mapping with no provider-specific public fields |
| R17 | Certify A3S Box profiles | R16 | Base/Recovery and every advertised Box profile pass on production-equivalent Linux |
| R18 | Add fuzz, property, and security campaigns | R10-R17 | All declared targets run, regressions checked in, zero unresolved crash/hang/leak |
| R19 | Add benchmarks and regression budgets | R10-R17 | Three clean same-runner baselines and enforced median/p95 budgets |
| R20 | Add churn, fault, and 24/72-hour soak automation | R13-R19 | Evidence bundles prove zero resource/FD/state growth and all stop conditions |
| R21 | Run bounded A3S OS production canary | R12-R20 | Git-only exact-SHA canary passes with complete cleanup and no safety stop |
| R22 | Perform release completion audit | R01-R21 | Every deep-test-plan release gate maps to authoritative passing evidence |
| R23 | Freeze the modern MCP Service-profile boundary | R00 | ADR 0005 accepted or explicitly superseded; no MCP fields enter the Runtime wire contract |
| R24 | Define the MCP-ready Service consumer profile and fixture manifest | R23 | Existing generic capabilities map to stable case IDs and one pinned black-box fixture |
| R25 | Certify one real Box-hosted modern MCP Service | R17, R24 | Typed endpoint reaches `server/discover`; exact profile digest and provider inventory evidence pass |
| R26 | Certify MCP Service recovery and replica isolation | R25 | Generation replacement, process/host loss, external deletion, distinct replica IDs, endpoint cleanup, and zero leaks pass |
| R27 | Run the Cloud and Gateway compatibility gate | R26 | Pinned Runtime/Box/Cloud/Gateway revisions consume only exact typed observations and one profile digest |
| R28 | Audit the Runtime `MCP0.2` claim | R22-R27 | Every Runtime-owned `MCP0.2` criterion maps to reviewable exact-SHA evidence |
| R29 | Freeze the unified Cloud consumer boundary | R11, ADR 0006 | Agent, Function, MCP, and Durable Cell fixtures use only Task/Service; Workflow remains orchestration-only |
| R30 | Export one consumer admission/readiness abstraction | R29 | `RuntimeConsumerRequirements` composes class/features/profile/readiness/liveness/endpoints and fails closed in focused tests |
| R31 | Migrate Cloud generic checks to the shared abstraction | R30 | Exact Runtime revision is pinned; duplicate class/capability/profile/readiness predicates are removed without moving product checks |
| R32 | Certify AaaS, FaaS, and Durable Cell Box profiles | R17, R31 | Real Box lifecycle, recovery, endpoint, mount, Secret, output, cleanup, and profile evidence passes per declared profile |
| R33 | Certify WaaS executable-node composition | R31-R32 | Flow replay chains/parallels Agent and Function children without a Workflow Runtime Unit or duplicate product intent |
| R34 | Run the Gateway/Cloud/Runtime/Box service-platform gate | R22, R27, R32-R33 | Exact revisions prove public Gateway ingress, internal endpoint admission, recovery, tenant isolation, and zero residue |
| R35 | Bind workload identity intent without product coupling | R01, R10, R30 | Current unit-spec/observation v4 and capabilities v6 carry one opaque attachment digest; the unified consumer projection rejects missing or drifted provider attestation |
| R36 | Separate Service readiness, liveness, and graceful stop | R01, R11, R30 | v4/v6 contracts bind separate evidence and activate four provider conformance cases; Box and Cloud consume the exact policy without product fields |

## Work packages

### Package A: Protocol completeness (`R00`-`R05`)

Deliverables:

- ADR 0002 implementation;
- schema constants and fields for inspection, log, and exec surfaces;
- `ProviderId` as the sole provider identity type;
- output artifact sizes and `OutputArtifacts` capability;
- confidential-attestation capability matching;
- explicit apply, stop, logs, and exec state guards;
- deadline wrappers with deterministic tests;
- golden wire fixtures and schema compatibility tests;
- declared Rust MSRV.

Exit gate: every public wire value has an explicit version and every operation
has a deterministic state/deadline oracle.

### Package B: Durable coordination (`R06`-`R10`)

Deliverables:

- independent operation and record locks;
- v2 unit directory and request journal;
- durable bounded unary exec receipts;
- generation reconciliation contract and deterministic provider model;
- process-level crash/failpoint harness;
- transition matrix and race matrix;
- hostile filesystem, permission, corruption, disk-full, and cancellation
  cases.

Exit gate: every mutating crash point can replay after a new process starts,
with valid state and no untracked provider resource.

ADR 0003 excludes interactive stdin, PTY, terminal resize, signals,
incremental output, and reconnectable exec sessions from this plan. A future
interactive protocol requires a distinct versioned capability and its own
dependency-ordered implementation and conformance work.

### Package C: Shared certification (`R11`-`R12`)

Deliverables:

- composable Base, Recovery, Networking, Mounts, Health, Resources, Logs, Exec,
  Security, Outputs, and Evidence conformance profiles;
- fixture traits and cleanup inventory contracts;
- CI jobs that fail when a mandatory provider prerequisite is missing;
- MSRV and stable validation, documentation, golden compatibility, and
  cross-architecture digest evidence.

Exit gate: mock and file-state correctness is independently reproducible for an
exact commit on Linux CI.

### Package D: Docker and A3S Cloud (`R13`-`R15`)

Deliverables:

- stale-generation container discovery and cleanup;
- provider identity and label-tamper enforcement;
- Task success/failure/timeout and Service readiness/liveness/graceful-stop
  profiles;
- network, volume, tmpfs, CPU, memory, PIDs, restart, and log-cursor probes;
- node-agent/Docker restart and external-deletion recovery;
- Cloud projection, command journal, redelivery, reconciliation, cancellation,
  log transport, and cleanup E2E tests.

Exit gate: real Docker and Cloud evidence covers every source-advertised
capability, and post-run provider inventory equals its baseline.

### Package E: A3S Box (`R16`-`R17`)

Deliverables:

- a driver hosted at the provider integration boundary;
- digest-pinned OCI apply/inspect/stop/remove/logs/exec mapping;
- stable unit/generation/request labels or metadata;
- `IsolationLevel::Sandbox` mapping;
- output, usage, attestation, secret, network, mount, and resource claims only
  when implemented and tested;
- crash, shim loss, VM loss, host restart, and cleanup profiles.

Exit gate: A3S Box passes Base and Recovery plus every capability it advertises
on production-equivalent A3S OS Linux.

### Package F: Non-functional release proof (`R18`-`R22`)

Deliverables:

- fuzz/property/security campaigns;
- controlled core and provider benchmarks;
- 10,000-cycle churn, resource leak detection, fault automation, and 24/72-hour
  soak;
- immutable evidence bundles;
- bounded production canary;
- requirement-by-requirement release audit.

Exit gate: all nine release gates in the deep test plan have direct,
reviewable, exact-SHA evidence.

### Package G: Modern MCP Service substrate (`R23`-`R28`)

Checkpoint (2026-07-30): the `R23`/`R24` foundation is implemented. ADR 0005,
the canonical consumer fixture, exact semantics-profile binding, typed
endpoint/generation acceptance, and stale evidence rejection pass focused
tests without adding MCP protocol fields to Runtime core. `R25` is next and
requires production-equivalent Linux plus the real A3S Box adapter; that
external evidence cannot be replaced by the current Windows fixture.

Deliverables:

- an accepted profile boundary that maps every hosted MCP replica to an
  ordinary Runtime Service;
- an explicit prohibition on MCP product fields, JSON-RPC parsing, routing,
  authorization, and protocol sessions in Runtime core;
- a canonical fixture manifest binding the MCP server image, Service
  specification, semantics profile digest, declared TCP port, readiness probe,
  optional liveness/graceful-stop policy, and expected endpoint evidence;
- real A3S Box apply, inspect, readiness, liveness, logs, graceful/forced stop,
  remove, restart, generation, and cleanup evidence;
- distinct Runtime Unit identities and typed endpoints for every replica;
- a black-box `server/discover` reachability probe whose protocol assertions
  remain in the consumer harness; and
- a pinned cross-repository compatibility and cleanup bundle.

Exit gate: Runtime closes Cloud sub-gate `MCP0.2` without changing the generic
Runtime wire schema or claiming ownership of MCP request semantics.

### Package H: Unified Cloud service consumers (`R29`-`R34`)

Checkpoint (2026-08-28): `R29` and `R30` are component-complete. The generic
consumer abstraction and Agent Service, Function Task/Service, stateless MCP
Service, and Durable Cell Service fixtures pass focused tests. Cloud adoption,
real Box evidence, Workflow composition, and the joint Gateway gate remain
open.

Checkpoint (2026-08-31): `R36` is Runtime-component complete. The contract and
consumer fixture are not provider evidence; Box implementation and exact
Cloud projection remain required before a hosted Agent capability claim.

Deliverables:

- one non-wire consumer requirements API over the existing capability, spec,
  and observation contracts;
- a source gate that prevents product terms from entering `src/contract`;
- Cloud adoption through exact Git revision pinning;
- no Workflow Runtime Unit: Flow coordinates only owner-created executable
  children;
- a required production A3S Box provider path with capability-triggered
  conformance;
- internal, generation-bound Service endpoints admitted by Cloud and published
  only through A3S Gateway; and
- per-profile recovery, cleanup, tenant isolation, and exact-revision evidence.

Exit gate: AaaS, WaaS executable nodes, FaaS, MCP, and Durable Cell deployments
share the same Runtime/Box lifecycle mechanics without sharing product state or
creating a second scheduler, endpoint registry, provider control path, or
public ingress.

## Immediate execution order

1. Preserve the existing optional ephemeral-storage and `unknown` recovery
   changes as inputs to ADR 0002.
2. Implement `R01`-`R04` with contract tests and update A3S Cloud constructors
   in the same compatibility change.
3. Push an exact feature commit so the A3S OS runner can build and test through
   Git; do not upload a working tree archive.
4. Implement `R05`-`R10`, rerunning the server-side core matrix after each
   durable-state slice.
5. Enable and expand real Docker conformance before starting the A3S Box
   adapter, so the shared provider oracles are proven by one real driver first.
6. Review and land component-complete `R29`/`R30`, then pin Cloud to that exact
   Runtime commit and complete `R31`.
7. Treat A3S Box as the production provider: close `R32` before any Cloud AI
   service availability claim.
8. Prove Workflow Agent/Function child composition in `R33`, then run the joint
   Gateway/Cloud/Runtime/Box gate in `R34`.
9. Keep `R23`/`R24` as the MCP specialization, but do not claim the MCP
   substrate until the required Box profiles and `R25`-`R28` evidence pass.

## Completion rule

The general Runtime release is complete only when `R22` can map every explicit
invariant, profile, performance gate, soak gate, production-safety condition,
and cleanup requirement to authoritative evidence. A release that claims an
MCP-ready Service substrate must additionally pass `R28`. A unified Cloud AI
service substrate must additionally pass `R34`. An unexecuted
environment-gated test, mock-only result, intended provider feature, or passing
narrow smoke test is not completion evidence.

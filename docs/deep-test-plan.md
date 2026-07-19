# A3S Runtime Deep Test Plan

## 1. Purpose

This document defines the verification architecture for `A3S-Lab/Runtime`.
The target is not merely a green crate test suite. The target is evidence that
the provider-neutral contract, durable lifecycle coordinator, state store, real
providers, and consuming control paths preserve the same semantics through
retries, crashes, contention, scale, and provider loss.

All build and runtime execution described here must run on Linux CI runners or
an A3S OS test runner. Developer laptops are not part of the execution matrix.
Servers must obtain source through Git at an exact commit; source archives must
not be copied from a workstation.

## 2. Scope

The plan covers five boundaries:

1. The public contract in `src/contract/`.
2. `ManagedRuntimeClient`, `FileRuntimeStateStore`, provider registration, and
   the exported conformance suite.
3. Every concrete `RuntimeDriver` against its real provider.
4. The A3S Cloud projection, node command journal, reconciliation, and
   observation path that consume this crate.
5. Performance, fault recovery, security, resource cleanup, and production
   canary evidence.

The A3S OS product runtime APIs that happen to use the word "runtime" are not
automatically in scope. They enter this plan only when they construct or carry
the Rust `a3s-runtime` protocol.

## 3. Evidence-Based Baseline

Source inspection on 2026-07-17 established this baseline. It must be refreshed
at the start of an implementation or release campaign.

| Area | Current evidence | Gap |
| --- | --- | --- |
| Runtime repository CI | No repository workflow is present | A green commit has no independent build or test evidence |
| Core integration tests | `tests/general_runtime.rs` contains 16 lifecycle, conflict, recovery, state, registry, and conformance scenarios | Most boundary combinations, crash points, cross-process races, and capacity limits are untested |
| Shared conformance | One happy-path Task and Service flow with replay checks | Optional features and negative/fault profiles are not covered |
| Real provider | A3S Cloud contains `DockerRuntimeDriver` | No other concrete Runtime driver is implemented |
| Real Docker tests | Three tests cover common conformance, create-before-state-update recovery, and external provider loss | They return success without running unless `A3S_CLOUD_TEST_DOCKER=1` |
| A3S Box | Documented as a future provider | No `RuntimeDriver` exists, so Box conformance cannot currently be claimed |
| Non-functional testing | No benchmark, fuzz, mutation, multi-process crash, or soak harness is present | No regression or durability evidence exists |

Existing tests are useful regression assets, but their presence is not proof
that they passed for a given commit. Every report must bind results to an exact
Runtime commit, provider commit, provider build, fixture digest, and host.

## 4. Required Invariants

These invariants are the basis of every test oracle.

### 4.1 Protocol and identity

- Every wire record accepts only its declared schema and fields.
- `(unit_id, generation, canonical_spec_digest)` is immutable.
- A request ID identifies exactly one request kind and digest.
- Exact request replay returns the same logical result.
- Stale generations and conflicting content fail before provider mutation.
- A mutable artifact tag never replaces the declared digest.
- Capability rejection occurs before state reservation and provider dispatch.
- Provider observations, exec results, removals, evidence, and attestations
  cannot substitute caller-owned identity.

### 4.2 Lifecycle and recovery

- Task convergence means `succeeded`; Service convergence means `running` and,
  when configured, `healthy`.
- Terminal observations are immutable.
- Explicit removal creates a durable generation-aware tombstone.
- A missing previously observed provider resource becomes `unknown`, not
  `not_found` or success.
- An ambiguous acknowledgement retries the same provider identity and must not
  create a duplicate resource.
- After confirmed provider loss has been persisted as `unknown`, a same-
  generation apply may adopt one replacement provider identity. Subsequent
  replay must return that replacement without another create.
- A deadline at or before the current clock prevents reservation and dispatch.
- Provider operation timeouts remain bounded independently of the pre-dispatch
  request deadline check.

### 4.3 Durability and concurrency

- A completed state write survives process restart and host reboot.
- A failed or interrupted write never destroys the last valid record.
- A pending receipt survives an ambiguous provider result.
- Concurrent operations preserve every accepted receipt and never create an
  untracked provider resource.
- Same-unit serialization and different-unit parallelism have explicit,
  measured behavior.
- State paths, files, locks, and temporary writes never follow a symbolic-link
  boundary and retain owner-only permissions.
- Corrupt, truncated, mismatched, or future-schema state fails closed before
  provider work.

### 4.4 Provider truthfulness

- Every advertised capability has a real passing test.
- Every unadvertised optional feature is rejected before provider dispatch.
- Resource limits are verified by provider inspection and observable behavior,
  not only by request construction.
- A provider restart, agent restart, or external resource deletion converges to
  one durable outcome with zero duplicate resources.
- A successful suite leaves no provider resources, state roots, ports, mounts,
  processes, or test volumes outside its declared retention policy.

## 5. Contract Decisions and P0 Oracles

Tests must not encode accidental behavior. ADR 0002 resolves the lifecycle and
operation decisions below, and ADR 0003 resolves the interactive streaming
exec boundary. The table retains the motivating risks and required release
oracles.

| Decision | Motivating risk | Required test oracle |
| --- | --- | --- |
| Wire schema boundary | Top-level log, exec, and inspection types do not all carry schema identifiers although the README says all wire records do | Define which types cross a versioned boundary; require a schema on each top-level wire record or narrow the compatibility claim |
| Provider identity binding | Capabilities carry a string validated differently from `ProviderId`, and the managed client does not bind it to a selected factory | Use one grammar and prove reported, registered, and observed provider identities cannot disagree |
| Generation handoff | Reserving a newer generation replaces the stored observation before the old provider resource is necessarily stopped or removed | Define caller, managed client, or driver ownership of the prior resource; prove no orphan on success, error, or crash |
| Cross-operation concurrency | State locking does not span an asynchronous provider call | Define ordering for apply/apply, apply/stop, apply/remove, stop/remove, and generation races |
| Recovery identity | Recovery from `unknown` can differ from ambiguous-ack reattachment | Permit identity replacement only after durable `unknown`; all other identity changes fail |
| Operation postconditions | Apply rejects only `accepted`, while stop can accept any otherwise valid transition | Define whether calls return only converged results or may return transitional observations, then test each allowed result |
| Task output fulfillment | A Task may request outputs, but capabilities do not express output collection and the current Docker driver returns none | Make output support mandatory or capability-gated; prove every requested output is collected, bounded, and digest-bound before convergence |
| Logs by state | Current-generation logs may be useful after Task completion or Service stop | Define allowed states, removal behavior, cursor retention, and provider-loss behavior |
| Exec by state | Generation matching alone does not prove a runnable unit | Require `running`, or explicitly delegate a narrower rule to providers |
| Bidirectional streaming exec | A disconnected stream can leave unbounded buffers, duplicate stdin, unacknowledged output, leaked sessions, or conflicting terminal owners | Runtime v0.2 exposes only bounded unary exec; a distinct versioned optional capability must prove backpressure, ordering, half-close, cancellation, deadline, reconnect, terminal ownership, and restart cleanup before publication |
| Stop after loss | A provider may be absent while durable state is `unknown` | Define whether stop returns `unknown`, is idempotent success, or requires recovery |
| Receipt retention | Records reject more than 10,000 receipts but have no retention/compaction policy | Define bounded retention without breaking exact replay guarantees |
| Deadline semantics | Deadlines are checked only before dispatch | Define whether drivers receive remaining budget and how late results are persisted |

## 6. Test Architecture

The suite is divided into layers so fast deterministic checks run frequently
while destructive checks run only in isolated environments.

| Layer | Purpose | Environment | Trigger | Maximum target duration |
| --- | --- | --- | --- | --- |
| L0 | Format, lint, docs, unit, golden protocol, property checks | Ephemeral Linux CI | Every pull request | 10 minutes |
| L1 | Managed lifecycle, file state, multi-process contention, deterministic fault driver | Ephemeral Linux CI | Every pull request | 20 minutes |
| L2 | Real provider capability and conformance profiles | Dedicated Linux provider runner | Merge and nightly | 30 minutes per provider |
| L3 | Crash, reboot, disk fault, scale, resource enforcement, and leak tests | Disposable A3S OS worker or VM | Nightly or weekly | 2 hours |
| L4 | Long soak and safe production canary | Dedicated soak worker; shared production only for non-destructive canary | Release candidate | 24 to 72 hours |

No layer may silently convert an unavailable prerequisite into a passing test.
A skipped provider job must be reported as `SKIPPED`, and a release gate that
requires that provider must fail.

The initial automation uses the crate's documented commands from the Runtime
repository root:

```text
cargo fmt --all --check
cargo test --all-targets
cargo clippy --all-targets -- -D warnings
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps
```

The real Docker gate runs from the A3S Cloud repository root:

```text
A3S_CLOUD_TEST_DOCKER=1 cargo test -p a3s-cloud-node-agent \
  --test docker_conformance real_docker_passes_all_advertised_runtime_profiles \
  -- --ignored --exact --nocapture --test-threads=1
```

The Docker test is explicitly ignored in ordinary workspace runs. Its
dedicated job must select the ignored test by exact name and fail unless the
Docker enable flag, isolated provider restart target, and provider socket are
present. This prevents an unavailable provider from being counted as a passing
certification.

The Runtime crate must declare a minimum supported Rust version before release;
L0 tests that version and current stable on the production Linux architectures.

## 7. Functional Test Matrix

Each implemented case receives a stable ID in the test name and evidence
manifest. Boundary tests use `minimum-1`, `minimum`, `maximum`, and `maximum+1`
where the type permits those values.

### 7.1 Serialization, validation, and digests

| IDs | Required coverage |
| --- | --- |
| `CT-SCHEMA-*` | Correct, missing, old, future, malformed, and unknown-field behavior for every public record |
| `CT-ID-*` | Empty, 1 byte, maximum length, overlength, control characters, Unicode byte length, path-like characters, and provider ID grammar |
| `CT-DIGEST-*` | Digest algorithm, hexadecimal length, invalid bytes, deterministic output across processes and architectures, and one-field mutation sensitivity |
| `CT-JSON-*` | Golden JSON decode/encode, enum tags, field names, missing required fields, duplicate JSON keys, and map-order independence |
| `CT-URI-*` | Artifact URI schemes, authority/path boundaries, digest binding, credentials, query/fragment, and mutable tags in each provider adapter |
| `CT-PATH-*` | Absolute paths, repeated separators, dot segments, `..`, NUL, CR/LF, maximum length, and mount/secret/output collision policy |

Golden fixtures are versioned test data. Updating one requires an explicit
schema review; snapshot acceptance is never automatic.

### 7.2 Unit specifications

| IDs | Required coverage |
| --- | --- |
| `SPEC-PROCESS-*` | Entrypoint fallback, command/argument counts and sizes, working directory, environment names/values, and deterministic map order |
| `SPEC-MOUNT-*` | Artifact, volume, and tmpfs sources; read-only behavior; zero/overflow size; duplicate names and targets; cross-kind collisions |
| `SPEC-SECRET-*` | Opaque references, environment/file targets, file modes, duplicate names/targets, collision with process environment and mounts, and non-disclosure |
| `SPEC-NET-*` | None/outbound/service modes, TCP/UDP, zero port, duplicate name/socket, 64-port boundary, and service-only publication |
| `SPEC-HEALTH-*` | HTTP/TCP/command probes, named-port binding, status ranges, path validation, timing relationships, start period, and thresholds |
| `SPEC-RESOURCE-*` | CPU, memory, PIDs, optional ephemeral storage, Task timeout, zero values, numeric maxima, and provider conversion overflow |
| `SPEC-ISOLATION-*` | Process, container, sandbox, and confidential requirements; capability rejection; provider mapping and evidence |
| `SPEC-RESTART-*` | Never, always, and on-failure policies; retry boundaries; Task/Service restrictions; provider enforcement |
| `SPEC-CLASS-*` | Every Task/Service restriction for health, timeout, restart policy, and outputs |
| `SPEC-OUTPUT-*` | Names, absolute paths, media types, size limits, duplicates, and succeeded-Task-only observations |
| `SPEC-SEMANTICS-*` | Optional profile digest validation and evidence binding |

Ephemeral storage is conditional: a provider needs
`ResourceControl::EphemeralStorage` only when the specification requests a
quota. CPU, memory, and PIDs remain required for every unit.

### 7.3 Capabilities and registry

Test every required family, duplicate entry, malformed provider identity,
empty optional family, and complete missing-capability ordering. Generate a
spec that independently requires each enum value and optional feature. Verify
that the registry rejects duplicate providers, has no default, never falls
back, and surfaces factory construction failures unchanged.

Capability claims are tested twice:

1. Pure matching proves that the contract calculates requirements correctly.
2. Real-provider probes prove that each advertised claim works.

### 7.4 Observations and transitions

Generate and verify the full state-transition matrix for Task and Service.
Cover provider identity, build identity, monotonic observation time, start and
finish ordering, terminal timestamps, health, outputs, usage, evidence,
attestation, failure details, and convergence.

The matrix must include:

- every allowed transition;
- every forbidden transition;
- same-state refresh;
- all transitions to and from `unknown` allowed by the final contract;
- terminal equality replay and every attempted terminal mutation;
- provider identity substitution before and after confirmed loss;
- a Service attempting `succeeded` and a Task carrying health;
- duplicate output artifacts and evidence bound to another specification.

### 7.5 Managed operations

| IDs | Required scenarios |
| --- | --- |
| `LC-APPLY-*` | First apply; pending replay; completed replay; same generation/new request; generation conflict; stale and next generation; rejected capability; expired deadline |
| `LC-INSPECT-*` | Never seen; active refresh; terminal cache; removed tombstone; provider absent to durable `unknown`; malformed provider response |
| `LC-STOP-*` | Running, already stopped, terminal Task, absent, removed, unknown, exact replay, request conflict, unsupported feature, and deadline |
| `LC-REMOVE-*` | Running, stopped, Task terminal, already absent with new request, exact replay, identity substitution, and provider error |
| `LC-LOG-*` | Current/stale/future/removed generation, stdout/stderr filter, limits, strict sequence, cursor resume, invalid cursor, rotation gap, large chunk, and capability rejection |
| `LC-EXEC-*` | Allowed/forbidden states, timeout, exit range, stdout/stderr bounds, truncation, identity substitution, generation checks, replay policy, and capability rejection |

Every mutation test records state immediately before reservation, after
reservation, after provider completion, and after durable completion.

## 8. Durable State and Race Matrix

### 8.1 Filesystem safety

Run on a real Linux filesystem with hostile fixtures for the root, `locks`,
`units`, lock file, record file, and temporary-file boundary. Cover symbolic
links, hard links where relevant, non-regular files, permission changes,
different umasks, wrong ownership, read-only filesystem, `ENOSPC`, inode
exhaustion, truncated JSON, invalid UTF-8, unknown schema, key mismatch, and a
record exceeding the receipt limit.

After every successful write, assert directory mode `0700`, file and lock mode
`0600`, valid JSON, matching storage key, and a directory sync. After every
injected failure, assert that either the prior record or the complete new record
is readable; a partially published record is never acceptable.

### 8.2 Crash points

Use subprocess tests and named failpoints around:

1. lock acquisition;
2. initial reservation construction;
3. temporary-file creation;
4. partial and complete writes;
5. file sync;
6. permission tightening;
7. atomic publish;
8. directory sync;
9. provider create/start/stop/remove;
10. observation or removal completion.

For each point, kill the process, create a new client from the same state root,
replay the same request, and compare provider inventory with durable state.
Cancel the caller future at each asynchronous boundary as a separate case;
dropping a future must not make a later state mutation or provider result
unrecoverable.

### 8.3 Concurrency cases

Run each race in thread, task, and independent-process variants where the
boundary permits it:

- 2, 32, and 128 identical apply requests;
- same specification with distinct request IDs;
- conflicting content with one request ID;
- generations N and N+1 concurrently;
- apply versus inspect, stop, and remove;
- stop versus remove;
- observation refresh versus remove;
- 1,000 different units concurrently;
- 10,001 sequential receipts for one unit.

Pass criteria are deterministic accepted/error classes, no lost completed
receipt, no invalid record, no deadlock, and exactly the provider resources
allowed by the generation-handoff decision.

## 9. Provider Conformance Profiles

Replace the single all-or-nothing conformance path with composable profiles.
The shared Runtime repository owns the oracles; provider repositories own
fixtures and destructive cleanup.

| Profile | Mandatory evidence |
| --- | --- |
| Base | Valid capabilities, Task success/failure/timeout, Service start/inspect/stop/remove, exact replay, generation conflict, tombstone |
| Recovery | Create-before-ack crash, client restart, provider restart, external deletion to `unknown`, one same-generation replacement, duplicate-resource detection |
| Networking | Every advertised network mode and protocol, loopback publication, outbound denial/allowance, port collision behavior |
| Mounts | Every advertised mount kind, read-only enforcement, persistence, isolation, cleanup |
| Health | Every advertised probe kind, threshold transitions, timeout, start period, unhealthy exit |
| Resources | Every advertised control verified by provider configuration and workload behavior |
| Logs | Stream filtering, total order, cursor resume, same-timestamp records, limit, rotation gap, retention, large records |
| Exec | Bounded unary state policy, timeout, exit code, output bounds, truncation, identity and generation binding |
| Security | Digest pinning, label/metadata tamper, namespace separation, secret handling, least privilege, hostile input |
| Evidence | Usage, evidence claims, profile binding, attestation validity for each advertised optional feature |

The harness must always run the Base and Recovery profiles. It discovers
optional profiles from capabilities and fails if an advertised capability has
no fixture or passing probe.

## 10. Current Provider Matrix

This table describes source claims, not release certification.

| Capability | Docker driver in A3S Cloud | A3S Box driver |
| --- | --- | --- |
| Task and Service | Advertised | Not implemented |
| OCI/Docker manifest | Advertised with digest-pinned URI | Not implemented at this boundary |
| Isolation | `container` | Intended `sandbox`; no adapter evidence |
| Network | none, outbound, service | Not implemented at this boundary |
| Mounts | named volume, tmpfs | Not implemented at this boundary |
| Health | HTTP, TCP, command | Not implemented at this boundary |
| Resources | CPU, memory, PIDs, Task timeout | Not implemented at this boundary |
| Features | durable identity, stop, remove, logs | Not implemented at this boundary |
| Not advertised | exec, usage, attestation, secrets, artifact mounts, ephemeral quota | No claims are testable yet |

An A3S Box row may become a release gate only after a real `RuntimeDriver`
exists and maps `IsolationLevel::Sandbox` without provider-specific fields in
the public contract.

### 10.1 Docker-specific mandatory cases

- Pull and reuse a digest-pinned image; reject tag-only, digest mismatch,
  credentials in the URI, malformed registry responses, and interrupted pull.
- Validate all managed labels and reject label tampering, wrong node,
  namespace collision, and multiple matching containers.
- Prove Task exit 0, nonzero exit, signal exit, timeout, and daemon error.
- Prove HTTP, TCP, and command health success and failure thresholds.
- Verify CPU, memory/swap, PIDs, network mode, loopback port binding, restart
  policy, volume mode, and hardened tmpfs through Docker inspect and behavior.
- Exercise stdout/stderr ordering, nanosecond timestamp ties, cursor replay,
  log rotation gap, filters, and one-MiB record rejection.
- Restart the node agent and Docker daemon at each recovery window.
- Count labeled containers and test volumes before and after every case; both
  deltas must be zero after cleanup.

The Docker job must set `A3S_CLOUD_TEST_DOCKER=1` explicitly and verify the
daemon before the test binary starts. A missing socket is a failed provider job,
not a passed test with early returns.

## 11. Consumer End-to-End Matrix

The first real consumer is A3S Cloud. Its release suite must prove:

- an immutable `WorkloadRevision` projects to the expected Service spec and
  digest;
- scheduler capability matching rejects an ineligible node before dispatch;
- the node command journal deduplicates exact commands and rejects conflicts;
- command lease expiry, redelivery, observation loss, and reordering converge;
- control-plane and node-agent restart preserve one provider resource;
- generation updates and rollback follow the generation-handoff decision;
- runtime observations remain bound to the selected node and deployment;
- ordered log cursors survive transport batching and reconnect;
- stop, cancel, deferred cleanup, and reconciliation leave no untracked unit.

Bench or another caller must add a separate profile-projection suite before it
can claim Runtime compatibility. Product-specific scoring, privacy, and
scheduling remain outside the Runtime crate, but their projection digest and
generic resource/isolation requirements are in scope.

## 12. Fault Injection Matrix

| Fault | Expected result |
| --- | --- |
| Transport error before provider mutation | Pending receipt; exact retry dispatches safely |
| Provider create succeeds, acknowledgement is lost | Retry reattaches the same resource; count remains one |
| Provider resource is externally deleted | Inspect persists `unknown`; one reapply creates one replacement |
| Agent is killed at every state/provider boundary | Restart and replay converge without invalid state or orphan |
| Provider daemon restarts | Operations remain bounded and later reconcile without duplicates |
| Host reboots after file or provider sync | Durable state and provider inventory converge |
| Disk becomes full or read-only | Prior record remains valid; no provider work after failed reservation |
| State file is truncated or tampered | Fail closed; no provider mutation |
| Clock jumps backward/forward | Request deadline and observation monotonicity remain deterministic |
| Duplicate provider resources are injected | Inspect/apply fails closed and reports the invariant violation |
| Network is partitioned or latency exceeds timeout | Bounded error, durable pending state where acknowledgement is ambiguous |
| Caller future is cancelled or provider task panics | Durable state remains replayable and later work cannot publish an untracked result |

Daemon restart, host reboot, disk faults, and broad network faults must never be
run on a shared production node. They require a disposable worker or nested VM
in the A3S OS environment.

## 13. Security and Adversarial Testing

- Fuzz every public JSON decoder, validation function, digest function, state
  record decoder, log cursor decoder, and provider artifact URI adapter.
- Add property tests for validation boundaries and transition invariants.
- Run contract-only tests under Miri where supported and use a concurrency
  model checker for any in-memory coordinator introduced later.
- Verify state path confinement against symlink replacement and time-of-check/
  time-of-use attempts from another process.
- Verify provider namespaces cannot discover, stop, remove, log, or exec units
  owned by another namespace or node.
- Verify error, audit, evidence, and log artifacts never contain secret values
  or registry credentials.
- Verify OCI digest pinning after pull and before create.
- Run dependency license, advisory, and supply-chain policy checks as a release
  input; they supplement rather than replace behavioral tests.

Fuzz regressions are checked in as minimal fixtures. Release candidates require
a bounded continuous fuzz run for every target and zero unresolved crashes,
panics, hangs, or uncontrolled allocations.

## 14. Performance and Capacity Plan

### 14.1 Core benchmarks

Measure minimum, typical, and maximum-size records for:

- specification validation and digest calculation;
- capability matching;
- observation and transition validation;
- state reserve, load, update, removal, and exact replay;
- records containing 1, 100, 1,000, and 10,000 receipts;
- same-unit contention and different-unit throughput at 1, 8, 32, and 128
  clients.

### 14.2 Provider benchmarks

Measure cold-image and warm-image Task apply, Service apply-to-healthy,
inspect, stop, remove, log pagination, provider restart recovery, and external
loss recovery. Record p50, p95, p99, throughput, CPU, RSS, file descriptors,
disk bytes, network bytes, and resource cleanup latency.

### 14.3 Initial gates

- Correctness counters allow zero lost receipts, duplicate resources, corrupt
  records, unexplained log gaps, or leaked resources.
- A pull request fails when a controlled benchmark shows a statistically
  significant regression greater than 10% in median or 15% in p95 against the
  same runner class.
- Absolute latency SLOs are frozen only after three clean baselines on the
  production-equivalent runner; they must not be guessed from laptop results.
- A 10,000-cycle churn test must finish with zero provider resources, mounts,
  ports, state roots outside retention, and a non-growing file descriptor
  count.
- A 24-hour merge soak and 72-hour release soak must show no monotonic RSS,
  state-temporary-file, or provider-resource growth after workload count
  returns to baseline.

## 15. A3S OS Execution Safety

### 15.1 Git-only deployment

The runner checks out exact commits without uploading source:

```text
git fetch --prune origin
git worktree add --detach /var/tmp/a3s-runtime-tests/<run-id> <runtime-sha>
```

Provider and consumer repositories use their own exact SHAs. The evidence
manifest records all of them. A dirty worktree is rejected.

### 15.2 Isolation

Every run has:

- a globally unique run ID and provider namespace;
- a dedicated state root and evidence directory;
- pinned fixture image digests;
- loopback-only dynamic service ports;
- CPU, memory, PIDs, disk, duration, and concurrency limits;
- a process-level cleanup trap and an independent TTL sweeper;
- pre-run and post-run inventories keyed by managed labels.

Shared production nodes run only L4 canaries: one bounded Task, one loopback
Service, inspect, logs when supported, stop, and remove. They never restart a
daemon, reboot the host, corrupt state, fill disk, alter firewall policy, use
production secrets, or mount production data.

L3 and destructive L4 work runs on a disposable A3S OS worker that matches the
production kernel, architecture, provider version, filesystem, and security
policy.

### 15.3 Preflight and stop conditions

Preflight records host identity, kernel, architecture, cgroup mode, filesystem,
provider build, available CPU/memory/disk/inodes, existing managed resources,
and active deployments. The run does not start without the configured safety
headroom.

Abort immediately on namespace collision, unexpected non-test resource
selection, failed cleanup, host free-space/inode threshold, sustained host load
threshold, production service error-budget burn, or a provider invariant
violation. Cleanup and evidence capture still run after an abort.

## 16. Evidence Bundle

Each run produces one immutable bundle:

```text
evidence/<run-id>/
├── manifest.json
├── environment.json
├── preflight.json
├── junit.xml
├── coverage/
├── benchmarks/
├── faults/
├── provider-inventory-before.json
├── provider-inventory-after.json
├── logs/
└── cleanup.json
```

`manifest.json` contains run ID, timestamps, trigger, Runtime/provider/consumer
SHAs, fixture digests, selected layers and case IDs, configuration digest, and
final status. `cleanup.json` lists every created resource and proves removal or
an explicitly retained diagnostic artifact. Evidence is redacted before
upload, checksummed, retained by release policy, and linked from the commit or
release check.

## 17. Delivery Phases

### P0: Deterministic core gate

- Resolve the contract decisions in Section 5.
- Add Linux CI for format, lint, docs, unit, integration, and compatibility
  fixtures.
- Declare and test the minimum supported Rust version and current stable.
- Add shared fixture builders, stable case IDs, golden JSON, property tests,
  and transition-matrix generation.
- Add a deterministic driver with call tracing and failures before and after
  every provider boundary.
- Make skipped mandatory prerequisites fail their owning job.

Exit: L0 and L1 are required checks, all protocol/lifecycle rows have an
identified test, and no unresolved P0 semantic decision remains.

### P1: Durable recovery gate

- Add multi-process state contention, failpoint, process-kill, permission,
  corrupt-state, `ENOSPC`, and restart tests.
- Implement and verify the chosen generation and concurrent-operation model.
- Define and test receipt retention/compaction.

Exit: every crash point preserves a valid durable state and produces no
untracked provider mutation in the deterministic provider model.

### P2: Real Docker certification

- Expand conformance into the profiles in Section 9.
- Run the mandatory Docker job on a dedicated Linux runner with the gate
  explicitly enabled.
- Add Docker fault, resource, log, namespace, and leak cases.
- Run A3S Cloud consumer restart and reconciliation flows.

Exit: every advertised Docker capability has passing real-provider evidence;
Base and Recovery pass; pre/post resource deltas are zero.

### P3: Performance, adversarial, and soak gate

- Add stable benchmark runners, fuzz targets, security cases, 10,000-cycle
  churn, and 24/72-hour soak automation.
- Establish three clean baselines and freeze provider-specific SLOs.

Exit: all correctness, regression, leak, and soak gates in Section 14 pass.

### P4: Additional provider certification and production canary

- For each new driver, supply fixtures for every advertised conformance
  profile.
- Add A3S Box only after its `RuntimeDriver` exists and verify the `sandbox`
  isolation mapping on production-equivalent Linux.
- Run the bounded L4 canary and verify cleanup.

Exit: the provider matrix is evidence-backed for the release SHAs, and the
production canary bundle proves zero leaked resources and no safety stop.

## 18. Release Gates

A Runtime release is blocked unless:

1. L0 and L1 pass for the exact release commit.
2. Golden compatibility changes have explicit schema approval.
3. The transition, request replay, generation, deadline, and state durability
   matrices have no missing P0 row.
4. Base and Recovery pass for every provider declared production-supported.
5. Every advertised optional capability has passing profile evidence.
6. Required consumer end-to-end flows pass at pinned consumer commits.
7. Performance regression, resource leak, fuzz, and soak gates pass at the
   release level required by the change risk.
8. Provider inventory after the final run matches the pre-run baseline.
9. The evidence bundle is complete, redacted, checksummed, and reviewable.

Passing a mock-only suite, an environment-gated test that did not execute, or a
narrow provider smoke test is not sufficient evidence for a release claim.

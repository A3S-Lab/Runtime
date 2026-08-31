# Unified AI Service Runtime

## 1. Purpose

This document defines how A3S Runtime supports Cloud's Agent as a Service
(AaaS), Workflow as a Service (WaaS), Function as a Service (FaaS), hosted
stateless MCP, and Durable Cell deployments without adding a lifecycle model
for each product.

The production path is:

```text
Client / SDK
  -> A3S Gateway                    public authentication, policy, routing
  -> A3S Cloud                      tenant and product semantics
  -> A3S Runtime                    one Task / Service lifecycle contract
  -> A3S Box                        production process and sandbox provider
```

Gateway never starts a process. Cloud never controls a Box process directly.
Box never decides tenant, Agent, Workflow, Function, or Cell policy.

## 2. First-principles model

Four facts determine the abstraction:

1. Product state and process state have different invariants.
2. Durable orchestration history and executable-unit lifecycle are different
   state machines.
3. Every hosted executable is either finite or continuously available.
4. Public traffic policy and node-local endpoint discovery have different
   authorities.

Therefore Runtime has exactly two unit classes:

| Runtime class | Meaning | Convergence |
| --- | --- | --- |
| `Task` | One bounded execution with a terminal result | `succeeded` or a typed terminal failure |
| `Service` | One long-running generation with optional endpoints and lifecycle probes | `running`, ready and live when required |

Agent, Workflow, Function, MCP, model, and Durable Cell are consumer semantics,
not additional Runtime classes.

## 3. Product projection matrix

| Cloud capability | Runtime projection | Product authority retained above Runtime |
| --- | --- | --- |
| Stateful interactive Agent | Warm, fenced `Service`; a bounded batch Agent may use `Task` | AgentExecution, provider run, semantic sequence, approval, checkpoint, workspace lease |
| Workflow | No unit for the orchestration itself; executable nodes delegate to their owning `Task` or `Service` profile | WorkflowRun, graph order, parallel waves, waits, retry, cancellation, compensation, Flow history |
| Finite hosted Function | `Task` | Function release/profile and the ordinary Execution lifecycle |
| Low-latency stateless Function | `Service` | Function release/profile, Workload deployment, scale policy, route intent |
| External FaaS | No local unit | One Cloud Connector revision and attempt; the external provider owns compute |
| Sessionless hosted MCP | Function-style `Service` | MCP release/protocol profile and Gateway request semantics |
| Durable Cell application | One ordinary `Service` per deployed application replica | Application revision, named-state schema, storage lineage, retention, writer epoch |
| Individual named Cell | No unit | Selected Cell provider behind the application Service |
| Static web build | Sandboxed finite `Task`; the built site is not a running unit | Source/release, immutable object bundle, route, cache and SPA policy |

A Workflow may chain or parallel Agent and Function nodes. A Workflow node does
not become a Runtime process merely because it is durable; A3S Flow owns that
durability and Runtime owns only the executable child.

React, Vue, and similar static sites use Runtime only for their untrusted build.
The immutable output is served through A3S Gateway from the admitted object
bundle. SSR, WebSocket, or server-side application code is an ordinary Service.

## 4. One composable consumer gate

`RuntimeConsumerRequirements` is the single non-wire abstraction by which a
consumer declares the generic substrate it needs. It composes:

- one required unit class;
- required generic capabilities;
- an optional requirement for an opaque semantics-profile digest;
- optional healthy-Service readiness;
- optional distinct liveness plus bounded graceful stop; and
- optional exact Service endpoint evidence.

`admit_spec` validates the immutable specification and advertised provider
capabilities before dispatch. `accept_observation` validates exact identity,
generation, specification digest, semantics evidence, readiness, liveness, and
endpoint evidence before an observation can be used as a ready target.

The abstraction is deliberately outside `src/contract`. Product profile
fields remain in the consumer repository, while Runtime binds only their
immutable digest. The wire contract is guarded against product-specific
classes and fields.

## 5. Ownership

| Concern | Sole owner |
| --- | --- |
| Tenant, release, invocation, Workflow, Agent, Function and Cell semantics | A3S Cloud bounded contexts |
| Durable orchestration, retry order and compensation | A3S Flow as coordinated by the owning Cloud context |
| Unit identity, generation, request replay, lifecycle and provider evidence | A3S Runtime |
| Process, sandbox, mount, network and resource mechanics | A3S Box in the production baseline |
| Desired replicas, placement, rollout and autoscaling | Cloud Workloads and Fleet |
| Public authentication, protocol policy, routing, streaming and drain | A3S Gateway from Cloud Edge snapshots |
| Secrets and immutable object bytes | Cloud Secrets and the single deployment object authority |

Runtime remains provider-neutral so the lifecycle contract is independently
testable, but A3S Box is the required production provider. An alternate driver
does not become supported merely by implementing the trait; it must pass Base,
Recovery, and every capability-triggered conformance profile it advertises.

## 6. Failure and recovery rules

- A product request is durable before a Runtime command is dispatched.
- Delivery may be replayed; product intent may not be repeated.
- One `(unit_id, generation, spec_digest)` identifies exactly one provider
  resource lineage.
- A missing prior provider resource becomes `unknown`, never implicit success.
- A new Service generation is publishable only from exact, ready and live
  evidence when those policies are configured.
- Readiness controls traffic admission; liveness controls recovery; the
  shutdown grace bounds provider termination. None may stand in for another.
- Gateway receives only Cloud-admitted targets and never reads Runtime or Box
  state directly.
- External FaaS transport ambiguity remains an indeterminate Connector outcome;
  Runtime cannot infer or repeat it.
- Durable Cell writer replacement requires storage and writer-fence evidence in
  addition to ordinary Runtime recovery evidence.

## 7. Delivery truth

As of 2026-08-31, the generic consumer gate and component fixtures for a
stateful Agent Service, Function Task, stateless Function/MCP Service, and
Durable Cell Service are implemented in this repository. The Agent fixture also
requires distinct liveness and bounded graceful stop. These tests prove
contract composition only.

Production availability still requires:

- a real A3S Box driver and passing capability-specific conformance evidence;
- exact-revision Cloud and Gateway consumer gates;
- process, provider, node and host recovery evidence;
- target removal, cleanup and zero-leak evidence; and
- each product's separate semantic, security, tenancy, load and operations
  gates.

Mocks and Windows component tests do not close those production gates.

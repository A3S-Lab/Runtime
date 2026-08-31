# ADR 0008: Separate Service Readiness, Liveness, and Graceful Stop

- Status: Accepted
- Date: 2026-08-31

## Context

The v3 Unit contract had one optional `health` policy and one corresponding
observation. That is enough to decide whether a Service may receive traffic,
but it cannot also answer whether the provider should recover the process.
Neither answer defines how long termination may wait before it must become
forceful.

Those are three different decisions:

- readiness admits or removes traffic;
- liveness detects a process that must be recovered; and
- shutdown grace bounds cooperative termination before force is required.

Collapsing them lets a provider claim support while ignoring one of the
behaviors, and lets a consumer treat traffic readiness as recovery evidence.
The Code Agent release contract also carries both probe paths and an exact
bounded shutdown grace, so losing either value during Runtime projection would
make the immutable release ineffective.

## Decision

1. Keep `RuntimeUnitSpec::health` as the readiness policy and document that
   meaning explicitly.
2. Add optional `RuntimeServiceLifecycle`, containing one liveness
   `RuntimeHealthCheck` and `shutdown_grace_seconds` in the inclusive range
   `1..=3600`.
3. Allow the lifecycle policy only on `Service`, and require a separate
   readiness policy whenever it is present.
4. Add `RuntimeObservation::liveness`. A running Service must report exactly
   the readiness and liveness observations configured by its specification;
   other states and Tasks report neither. Configured convergence requires both
   observations to be healthy.
5. Add the atomic `RuntimeFeature::ServiceLifecycle` capability. It requires
   Service support, at least one health-check kind, and `Stop`. Admission checks
   both the readiness and liveness probe kinds.
6. Extend `RuntimeConsumerRequirements` with
   `require_service_lifecycle()`, which fails closed on a missing capability,
   policy, observation, or healthy liveness result.
7. Advertising the feature activates four real-provider Health cases:
   `HEALTH-READINESS-LIVENESS-SEPARATION`,
   `HEALTH-LIVENESS-TRANSITION`, `HEALTH-GRACEFUL-STOP`, and
   `HEALTH-GRACE-DEADLINE-FORCE`.
8. Version the wire change as capabilities v6, unit-spec v4, and observation
   v4; release it as `a3s-runtime` 0.5.0.

Runtime remains provider-neutral. It does not define application routes,
restart-count policy, operating-system signals, or a product-specific Agent
field. Providers own those mechanics and must prove the observable contract.

## Consequences

- Existing providers can consume v4 without advertising `ServiceLifecycle`,
  but cannot accept a specification that requires it.
- Consumers can independently require traffic readiness or the stronger
  lifecycle guarantee.
- A source-level capability claim is not certification. Box must persist the
  grace policy and pass all four cases on production-equivalent Linux before
  Cloud may admit a Code Agent release that requires this feature.
- The schema and crate minor-version bumps are intentionally breaking for the
  pre-1.0 protocol; golden fixtures make the boundary explicit.

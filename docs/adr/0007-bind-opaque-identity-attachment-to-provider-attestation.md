# ADR 0007: Bind Opaque Identity Attachment to Provider Attestation

- Status: Accepted
- Date: 2026-08-30
- Decision owners: A3S Runtime maintainers

## Context

An execution-semantics profile and a workload-identity attachment are distinct
facts. The first describes caller-owned product behavior. The second selects an
immutable identity and attestation intent that a platform may later admit for
credential issuance. Reusing `semantics_profile_digest` for both would make two
independent revisions indistinguishable and would couple security changes to
product semantics.

Runtime already binds `unit_id`, generation, the complete specification digest,
provider resource identity, provider build, evidence, and an optional provider
attestation. It lacked the single opaque input that lets a caller prove which
identity attachment entered that exact specification.

## Decision

### 1. Carry one opaque attachment digest

`RuntimeUnitSpec` v3 adds optional `identity_attachment_digest`. Runtime
validates only that it is a canonical SHA-256 digest. Organization, Workload,
policy, credential, issuer, and product-role fields remain outside the Runtime
protocol.

`RuntimeEvidence` in observation v3 repeats the exact digest. A provider-backed
observation for an attached specification fails validation if evidence omits or
changes it. `RuntimeFeature::IdentityAttachment` in capabilities v5 makes
support explicit before dispatch.

### 2. Use one attestation projection

`RuntimeAttestationBinding` is the sole provider-neutral projection from an
exact specification and observation. It requires matching unit and generation,
specification digest, attachment digest, provider resource and build identity,
provider evidence, positive observation time, and a digest-bound provider
attestation artifact. Its stable digest is suitable for a caller-owned durable
admission record.

The projection does not decide freshness, trust profile, workload lifecycle,
tenant authorization, or credential eligibility. Those decisions remain with
the consuming bounded context.

### 3. Compose admission through the existing requirements API

`RuntimeConsumerRequirements::require_identity_attestation` requires both the
identity-attachment and attestation capabilities, rejects a specification
without the digest, and accepts observations only through
`RuntimeAttestationBinding`. No second validator, lifecycle, registry, or state
store is introduced.

### 4. Version the changed wire surfaces

Unit specifications and observations advance to v3; capabilities advance to
v5. Existing durable v2 state requires an explicit caller migration or archival
decoder before the new client starts. Runtime does not silently reinterpret an
older security contract.

## Consequences

- A provider attestation over the complete Runtime spec digest transitively
  binds the opaque identity attachment without learning product policy.
- Callers can combine the typed Runtime binding with their own node, placement,
  resource-claim, policy, freshness, and revocation authorities.
- Echoing a digest without an attestation artifact is not sufficient for the
  typed binding.
- The field does not claim that an artifact is trustworthy; provider-specific
  verification and release certification remain mandatory.

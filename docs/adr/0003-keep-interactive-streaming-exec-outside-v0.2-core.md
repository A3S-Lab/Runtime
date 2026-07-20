# ADR 0003: Keep Interactive Streaming Exec Outside the v0.2 Core

- Status: Accepted
- Date: 2026-07-19
- Decision owners: A3S Runtime maintainers

## Context

Runtime v0.2 exposes a provider-neutral `exec` operation for a command against
the exact running unit generation. The operation is deliberately unary:

- one validated request contains the command and an execution deadline;
- one durable request receipt binds retry and provider reattachment;
- one result contains the exit code, separate buffered stdout and stderr, and
  a truncation indicator;
- stdout and stderr are each limited to 16 MiB; and
- the completed result is replayable without executing the command again.

The contract has no stdin stream, PTY mode, terminal resize, signal stream,
cross-stream ordering, or reconnectable session. `RuntimeFeature::Exec`
advertises only the bounded unary operation.

A bidirectional interactive protocol has materially different lifecycle and
durability requirements. A connection can disappear while input is buffered,
output is unacknowledged, or the provider is exiting. Without an explicit
owner and bounded flow control, a caller retry can duplicate input, lose
output, leak a provider session, or publish two different terminal outcomes.
Those failures cannot be made safe by treating logs as stdout or by issuing
repeated unary exec requests.

## Decision

Bidirectional or interactive streaming exec is not part of the Runtime v0.2
core contract. The existing unary `RuntimeClient::exec`,
`RuntimeDriver::exec`, `RuntimeExecRequest`, `RuntimeExecResult`, and
`RuntimeFeature::Exec` remain unchanged.

In particular:

- `Exec` does not imply stdin, PTY, resize, signals, incremental output, or
  stream resume;
- log cursors are not exec-session cursors and may not be used to emulate an
  interactive command;
- splitting one interactive command into repeated unary exec requests does not
  create a conformant stream; and
- Cloud must not bypass Runtime through direct node or provider access and
  present that path as Runtime-compatible terminal or exec behavior.

A future interactive protocol may be proposed only as a distinct, optional,
separately versioned capability. It must not extend the meaning of the existing
`Exec` feature or reuse the v1 unary wire schemas.

Before that capability can be published, its ADR, wire contract, and
conformance profile must define and test all of the following:

1. A generation-bound session identity, durable request identity, and exactly
   one authority for the terminal exit result.
2. Bounded queues in both directions, explicit flow-control credits or
   acknowledgements, and deterministic behavior when either peer applies
   backpressure.
3. stdin half-close and EOF semantics, stdout/stderr ordering, frame size
   limits, and any PTY, resize, or signal behavior.
4. Cancellation, caller disconnect, provider disconnect, and absolute deadline
   behavior, including which event wins a race with process exit.
5. Reconnect and resume rules, acknowledged offsets, replay bounds, and
   duplicate-input prevention, or an explicit non-resumable contract.
6. Ownership and cleanup after caller, Runtime, or provider restart, including
   retention limits for detached sessions and unacknowledged output.
7. Capability discovery, unsupported-capability errors, authorization
   boundaries, and fault tests that prove no unbounded memory, duplicate
   process, conflicting terminal result, or leaked session.

Until those requirements are accepted and implemented, a caller that needs an
interactive terminal does not have that capability through Runtime.

## Consequences

- Runtime v0.2 keeps one bounded, durable, provider-neutral exec semantic that
  can be tested without a long-lived transport.
- Existing providers and consumers do not acquire hidden streaming obligations
  when they advertise `RuntimeFeature::Exec`.
- Unary exec cannot provide a live shell, interactive stdin, PTY behavior, or
  real-time output. The separate log operation remains suitable only for
  observing unit logs.
- A future streaming design requires a new compatibility review and
  conformance gate instead of an additive method that silently weakens retry,
  cancellation, or resource-ownership guarantees.

## Rejected alternatives

### Add bidirectional streaming to `Exec` now

Rejected because the current request receipt stores one terminal result and
does not define frame acknowledgement, half-close, disconnect, or resume
semantics. Adding a transport stream before those rules would publish
provider-dependent behavior as a core guarantee.

### Emulate a stream with unary exec and logs

Rejected because independent requests cannot preserve stdin identity or
exactly-once delivery, while log cursors do not bind command output or a
terminal exit result.

### Let each provider expose an untyped streaming extension

Rejected because Cloud could no longer discover or test one provider-neutral
capability and provider-specific session fields would escape the driver
boundary.

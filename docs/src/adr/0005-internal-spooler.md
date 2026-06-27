# 0005 — Internal spooler

- Status: Accepted

## Context

Batch jobs need ordered dispatch, per-item cut control, retries for flaky
links, and graceful handling of printer disconnects. Relying on the OS spooler
would couple us to platform queues and obscure these behaviors.

## Decision

Ship an internal, in-process spooler (`lbl-spool`). It holds a FIFO of encoded
jobs, dispatches them sequentially over a `Transport`, retries transient
failures with backoff, and on a persistent failure treats it as a disconnect:
the job is kept at the front of the queue (not lost) for a later retry. The
printer's desired configuration is retained by `lbl-config`.

## Consequences

- Predictable, observable batch behavior independent of the OS.
- We own retry/cut/queue semantics and can expose them in the API/UI.
- Not a persistent system service (yet); the queue lives for the run. Persisting
  the queue across process restarts is a possible future extension.

# lbl-spool

The internal print spooler (not the OS spooler). Owns a FIFO queue of encoded
jobs and dispatches them sequentially over a `Transport`.

- **Per-item cut control**: a job may carry an explicit cut command appended
  after its payload.
- **Retry with backoff**: transient send failures are retried per a
  `RetryPolicy`.
- **Disconnect handling**: if the device is unreachable after the retry budget,
  the run aborts and the job is **kept at the front of the queue** (not lost),
  ready to retry after reconnect. Desired printer configuration is retained by
  `lbl-config`.

## CLI

```bash
lbl-spool --network 192.168.1.50:9100 label1.zpl label2.zpl
lbl-spool --usb 0922:1001 label.bin
lbl-spool --serial /dev/ttyACM0 label.niimbot
```

`run_with` lets a caller confirm each job over a bidirectional `Transport`
(e.g. the orchestrator polls NIIMBOT print status before dispatching the next
label); a failing probe is retried like a send failure, so jobs are never lost.

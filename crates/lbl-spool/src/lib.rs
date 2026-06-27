//! The internal print spooler for `lbl` (not the OS spooler).
//!
//! The spooler owns a FIFO queue of encoded jobs and dispatches them
//! sequentially over a [`Transport`]. It provides:
//! - per-item cut control (a job may carry an explicit cut command appended
//!   after its payload),
//! - bounded retries with backoff for transient failures, and
//! - graceful disconnect handling: when the device is unreachable, the job
//!   stays queued (it is not lost) so it can be retried after reconnect; the
//!   desired printer configuration is retained by `lbl-config`.
//!
//! ```
//! use lbl_spool::{Spooler, JobState};
//! use lbl_device::{Transport, DeviceError};
//!
//! struct OkTransport;
//! impl Transport for OkTransport { fn send(&mut self, _: &[u8]) -> Result<(), DeviceError> { Ok(()) } }
//!
//! let mut spool = Spooler::new();
//! let id = spool.enqueue("label", vec![1, 2, 3], None);
//! let report = spool.run(&mut OkTransport);
//! assert_eq!(report.completed, 1);
//! assert_eq!(spool.state(id), Some(JobState::Completed));
//! ```

use std::collections::{HashMap, VecDeque};
use std::time::Duration;

use lbl_device::{DeviceError, Transport};
use serde::{Deserialize, Serialize};

/// Retry behavior for transient send failures.
#[derive(Debug, Clone, Copy)]
pub struct RetryPolicy {
    /// Maximum send attempts per job before it is considered a connection
    /// failure.
    pub max_attempts: u32,
    /// Delay between attempts.
    pub backoff: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            backoff: Duration::from_millis(500),
        }
    }
}

/// A queued print job: an encoded payload plus an optional cut command appended
/// after it.
#[derive(Debug, Clone)]
pub struct SpoolJob {
    /// Stable job id.
    pub id: u64,
    /// Human-friendly name.
    pub name: String,
    /// Encoded protocol bytes.
    pub payload: Vec<u8>,
    /// Optional cut command bytes sent after the payload (per-item cut control).
    pub cut_command: Option<Vec<u8>>,
}

/// The lifecycle state of a job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "state", content = "detail")]
pub enum JobState {
    /// Waiting in the queue.
    Queued,
    /// Currently being sent.
    Printing,
    /// Sent successfully.
    Completed,
    /// Permanently failed with a message.
    Failed(String),
}

/// The outcome of a [`Spooler::run`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SpoolReport {
    /// Number of jobs completed.
    pub completed: usize,
    /// Number of jobs that permanently failed.
    pub failed: usize,
    /// Number of jobs still queued (e.g. after a disconnect aborted the run).
    pub remaining: usize,
    /// Set when the run aborted because the device became unreachable.
    pub disconnected: bool,
}

/// The print spooler.
#[derive(Default)]
pub struct Spooler {
    queue: VecDeque<SpoolJob>,
    states: HashMap<u64, JobState>,
    next_id: u64,
    policy: RetryPolicy,
}

impl Spooler {
    /// Create a spooler with the default retry policy.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a spooler with an explicit retry policy.
    pub fn with_policy(policy: RetryPolicy) -> Self {
        Self {
            policy,
            ..Self::default()
        }
    }

    /// Enqueue a job, returning its id.
    pub fn enqueue(&mut self, name: impl Into<String>, payload: Vec<u8>, cut_command: Option<Vec<u8>>) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.queue.push_back(SpoolJob {
            id,
            name: name.into(),
            payload,
            cut_command,
        });
        self.states.insert(id, JobState::Queued);
        id
    }

    /// Number of jobs still queued.
    pub fn pending(&self) -> usize {
        self.queue.len()
    }

    /// The state of a job, if known.
    pub fn state(&self, id: u64) -> Option<JobState> {
        self.states.get(&id).cloned()
    }

    /// Cancel a queued job (no effect once it has started/completed).
    pub fn cancel(&mut self, id: u64) -> bool {
        if let Some(pos) = self.queue.iter().position(|j| j.id == id) {
            self.queue.remove(pos);
            self.states.insert(id, JobState::Failed("cancelled".into()));
            true
        } else {
            false
        }
    }

    /// Dispatch queued jobs sequentially over `transport`.
    ///
    /// Transient failures are retried per the [`RetryPolicy`]. If a job still
    /// fails after `max_attempts`, the run aborts and the job is **kept at the
    /// front of the queue** (treated as a disconnect) so it can be retried
    /// later. Non-abort failures are not used here because all transport errors
    /// are treated as potential disconnects.
    pub fn run<T: Transport>(&mut self, transport: &mut T) -> SpoolReport {
        let mut report = SpoolReport::default();

        while let Some(job) = self.queue.pop_front() {
            self.states.insert(job.id, JobState::Printing);

            let mut last_err: Option<DeviceError> = None;
            let mut sent = false;
            for attempt in 0..self.policy.max_attempts {
                if attempt > 0 && !self.policy.backoff.is_zero() {
                    std::thread::sleep(self.policy.backoff);
                }
                match self.send_job(transport, &job) {
                    Ok(()) => {
                        sent = true;
                        break;
                    }
                    Err(e) => last_err = Some(e),
                }
            }

            if sent {
                self.states.insert(job.id, JobState::Completed);
                report.completed += 1;
            } else {
                // Treat as a disconnect: retain the job and stop.
                tracing::warn!(job = job.id, "device unreachable, keeping job queued");
                self.states.insert(job.id, JobState::Queued);
                self.queue.push_front(job);
                report.disconnected = true;
                report.remaining = self.queue.len();
                if let Some(e) = last_err {
                    tracing::warn!("last error: {e}");
                }
                return report;
            }
        }

        report.remaining = self.queue.len();
        report
    }

    fn send_job<T: Transport>(&self, transport: &mut T, job: &SpoolJob) -> Result<(), DeviceError> {
        transport.send(&job.payload)?;
        if let Some(cut) = &job.cut_command {
            transport.send(cut)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct AlwaysOk {
        sent: usize,
    }
    impl Transport for AlwaysOk {
        fn send(&mut self, _data: &[u8]) -> Result<(), DeviceError> {
            self.sent += 1;
            Ok(())
        }
    }

    struct FailsThenOk {
        fail_times: u32,
        calls: u32,
    }
    impl Transport for FailsThenOk {
        fn send(&mut self, _data: &[u8]) -> Result<(), DeviceError> {
            self.calls += 1;
            if self.calls <= self.fail_times {
                Err(DeviceError::Transport("flaky".into()))
            } else {
                Ok(())
            }
        }
    }

    struct AlwaysFails;
    impl Transport for AlwaysFails {
        fn send(&mut self, _data: &[u8]) -> Result<(), DeviceError> {
            Err(DeviceError::NotFound("gone".into()))
        }
    }

    fn fast_policy() -> RetryPolicy {
        RetryPolicy {
            max_attempts: 3,
            backoff: Duration::ZERO,
        }
    }

    #[test]
    fn completes_all_jobs() {
        let mut spool = Spooler::with_policy(fast_policy());
        let a = spool.enqueue("a", vec![1], None);
        let b = spool.enqueue("b", vec![2], None);
        let mut t = AlwaysOk { sent: 0 };
        let report = spool.run(&mut t);
        assert_eq!(report.completed, 2);
        assert_eq!(spool.state(a), Some(JobState::Completed));
        assert_eq!(spool.state(b), Some(JobState::Completed));
        assert_eq!(t.sent, 2);
    }

    #[test]
    fn retries_then_succeeds() {
        let mut spool = Spooler::with_policy(fast_policy());
        spool.enqueue("a", vec![1], None);
        let mut t = FailsThenOk { fail_times: 2, calls: 0 };
        let report = spool.run(&mut t);
        assert_eq!(report.completed, 1);
        assert_eq!(t.calls, 3);
    }

    #[test]
    fn disconnect_keeps_job_queued() {
        let mut spool = Spooler::with_policy(fast_policy());
        let a = spool.enqueue("a", vec![1], None);
        spool.enqueue("b", vec![2], None);
        let mut t = AlwaysFails;
        let report = spool.run(&mut t);
        assert!(report.disconnected);
        assert_eq!(report.completed, 0);
        assert_eq!(report.remaining, 2);
        // The failed job remains queued (not lost) for a later retry.
        assert_eq!(spool.state(a), Some(JobState::Queued));
        assert_eq!(spool.pending(), 2);
    }

    #[test]
    fn cut_command_is_sent_after_payload() {
        let mut spool = Spooler::with_policy(fast_policy());
        spool.enqueue("a", vec![1], Some(vec![0x1d, 0x56, 0x00]));
        let mut t = AlwaysOk { sent: 0 };
        spool.run(&mut t);
        assert_eq!(t.sent, 2); // payload + cut command
    }
}

//! Shared headless-Chromium renderer with bounded concurrency.
//!
//! Launching a browser is expensive, so a single [`ChromiumBackend`] is started
//! once and reused for every request. Chromium is itself multi-process (each
//! page is an isolated renderer), so one browser renders many labels in
//! parallel; the parallelism ceiling is CPU, not the number of browsers. A
//! [`Semaphore`] caps how many renders run at once so a burst of requests can
//! never oversubscribe the available cores.

use std::sync::{Arc, Mutex};

use lbl_render::ChromiumBackend;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// A reusable, self-healing headless-Chromium renderer.
pub struct RenderPool {
    permits: Arc<Semaphore>,
    /// The current browser, launched lazily on first use and replaced when it
    /// stops answering. `None` means "launch on next use".
    backend: Mutex<Option<Arc<ChromiumBackend>>>,
}

impl RenderPool {
    /// Create a pool allowing `concurrency` simultaneous renders (min 1).
    pub fn new(concurrency: usize) -> Self {
        Self {
            permits: Arc::new(Semaphore::new(concurrency.max(1))),
            backend: Mutex::new(None),
        }
    }

    /// Reserve one render slot, waiting if all are in use.
    ///
    /// This is `async` on purpose: callers acquire the slot before moving the
    /// blocking render onto a worker thread, so a client that disconnects while
    /// still queued simply drops its future and frees the slot instead of
    /// running a render nobody is waiting for.
    pub async fn acquire(&self) -> OwnedSemaphorePermit {
        self.permits
            .clone()
            .acquire_owned()
            .await
            .expect("render semaphore is never closed")
    }

    /// Run `render` against the shared browser on the calling (blocking) thread.
    ///
    /// If the render fails and the browser is no longer alive, the browser is
    /// relaunched and the render is retried once, so a crashed Chromium heals
    /// within the same request. `render` is therefore called at most twice and
    /// must be idempotent.
    pub fn render_blocking<F, T>(&self, render: F) -> anyhow::Result<T>
    where
        F: Fn(&ChromiumBackend) -> anyhow::Result<T>,
    {
        let backend = self.backend()?;
        match render(&backend) {
            Ok(value) => Ok(value),
            Err(err) if backend.healthy() => Err(err),
            Err(_) => {
                self.invalidate(&backend);
                let fresh = self.backend()?;
                render(&fresh)
            }
        }
    }

    /// Eagerly launch the browser so the first real render is not slowed by a
    /// cold start.
    pub fn warm(&self) -> anyhow::Result<()> {
        self.backend().map(|_| ())
    }

    /// Number of render slots currently free (for tests and diagnostics).
    pub fn available_permits(&self) -> usize {
        self.permits.available_permits()
    }

    /// Return the live browser, launching it if there is none.
    ///
    /// Launches are serialized by the mutex, so a burst of first requests
    /// produces exactly one browser rather than several racing launches.
    fn backend(&self) -> anyhow::Result<Arc<ChromiumBackend>> {
        let mut guard = self.backend.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(backend) = guard.as_ref() {
            return Ok(backend.clone());
        }
        let backend = Arc::new(ChromiumBackend::launch()?);
        *guard = Some(backend.clone());
        Ok(backend)
    }

    /// Drop `dead` so the next [`Self::backend`] relaunches, but only if it is
    /// still the current browser (a concurrent caller may have replaced it).
    fn invalidate(&self, dead: &Arc<ChromiumBackend>) {
        let mut guard = self.backend.lock().unwrap_or_else(|e| e.into_inner());
        if guard
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, dead))
        {
            *guard = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn acquire_bounds_concurrency() {
        let pool = RenderPool::new(2);
        assert_eq!(pool.available_permits(), 2);

        let first = pool.acquire().await;
        let second = pool.acquire().await;
        assert_eq!(pool.available_permits(), 0);

        drop(first);
        assert_eq!(pool.available_permits(), 1);
        drop(second);
        assert_eq!(pool.available_permits(), 2);
    }

    #[test]
    fn zero_concurrency_is_clamped_to_one() {
        assert_eq!(RenderPool::new(0).available_permits(), 1);
    }
}

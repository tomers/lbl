//! Shared headless-Chromium renderer with bounded concurrency.
//!
//! When the `chromium` feature is disabled, [`RenderPool`] is a no-op stub so
//! transpile-only HTTP deployments can build without linking chromiumoxide.

use std::sync::Arc;

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

#[cfg(feature = "chromium")]
use std::sync::Mutex;

#[cfg(feature = "chromium")]
use lbl_render::ChromiumBackend;

/// A reusable, self-healing headless-Chromium renderer (or a stub without the
/// `chromium` feature).
pub struct RenderPool {
    permits: Arc<Semaphore>,
    #[cfg(feature = "chromium")]
    backend: Mutex<Option<Arc<ChromiumBackend>>>,
}

impl RenderPool {
    /// Create a pool allowing `concurrency` simultaneous renders (min 1).
    pub fn new(concurrency: usize) -> Self {
        Self {
            permits: Arc::new(Semaphore::new(concurrency.max(1))),
            #[cfg(feature = "chromium")]
            backend: Mutex::new(None),
        }
    }

    /// Reserve one render slot, waiting if all are in use.
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
    /// relaunched and the render is retried once.
    #[cfg(feature = "chromium")]
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
    /// cold start. No-op when built without the `chromium` feature.
    pub fn warm(&self) -> anyhow::Result<()> {
        #[cfg(feature = "chromium")]
        {
            self.backend().map(|_| ())
        }
        #[cfg(not(feature = "chromium"))]
        {
            Ok(())
        }
    }

    /// Number of render slots currently free (for tests and diagnostics).
    pub fn available_permits(&self) -> usize {
        self.permits.available_permits()
    }

    #[cfg(feature = "chromium")]
    fn backend(&self) -> anyhow::Result<Arc<ChromiumBackend>> {
        let mut guard = self.backend.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(backend) = guard.as_ref() {
            return Ok(backend.clone());
        }
        let backend = Arc::new(ChromiumBackend::launch()?);
        *guard = Some(backend.clone());
        Ok(backend)
    }

    #[cfg(feature = "chromium")]
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

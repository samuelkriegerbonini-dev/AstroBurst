use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard, PoisonError};

use tokio::task::AbortHandle;

use crate::core::mast::errors::{bad_input, cancelled, internal};
use crate::core::mast::ids::validate_request_id;
use crate::core::mast::types::MastCmdError;

struct Entry {
    seq: u64,
    handle: AbortHandle,
}

#[derive(Default)]
pub struct Registry {
    tasks: Mutex<HashMap<String, Entry>>,
    seq: AtomicU64,
}

impl std::fmt::Debug for Registry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Registry").field("running", &self.len()).finish()
    }
}

struct Registration<'a> {
    registry: &'a Registry,
    id: String,
    seq: u64,
    handle: AbortHandle,
}

impl Drop for Registration<'_> {
    fn drop(&mut self) {
        self.handle.abort();
        let mut tasks = self.registry.tasks();
        if tasks.get(&self.id).is_some_and(|entry| entry.seq == self.seq) {
            tasks.remove(&self.id);
        }
    }
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    fn tasks(&self) -> MutexGuard<'_, HashMap<String, Entry>> {
        self.tasks.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub async fn run<T, F>(&self, request_id: &str, future: F) -> Result<T, MastCmdError>
    where
        T: Send + 'static,
        F: Future<Output = Result<T, MastCmdError>> + Send + 'static,
    {
        validate_request_id(request_id)?;
        let task = tokio::spawn(future);
        let seq = self.seq.fetch_add(1, Ordering::Relaxed);
        let handle = task.abort_handle();
        let previous = self.tasks().insert(request_id.to_owned(), Entry { seq, handle: handle.clone() });
        if let Some(previous) = previous {
            previous.handle.abort();
        }
        let _registration = Registration { registry: self, id: request_id.to_owned(), seq, handle };
        match task.await {
            Ok(result) => result,
            Err(error) if error.is_cancelled() => Err(cancelled()),
            Err(_) => Err(internal()),
        }
    }

    pub fn cancel(&self, request_ids: Option<&[String]>, prefix: Option<&str>) -> Result<usize, MastCmdError> {
        let ids = request_ids.unwrap_or_default();
        for id in ids {
            validate_request_id(id)?;
        }
        if let Some(prefix) = prefix {
            if !prefix.ends_with(':') || validate_request_id(prefix).is_err() {
                return Err(bad_input("prefix", "must be a non-empty request id prefix ending with :"));
            }
        }
        let mut tasks = self.tasks();
        let targets: Vec<String> = tasks
            .keys()
            .filter(|key| ids.contains(key) || prefix.is_some_and(|p| key.starts_with(p)))
            .cloned()
            .collect();
        for key in &targets {
            if let Some(entry) = tasks.remove(key) {
                entry.handle.abort();
            }
        }
        Ok(targets.len())
    }

    pub fn len(&self) -> usize {
        self.tasks().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn contains(&self, request_id: &str) -> bool {
        self.tasks().contains_key(request_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::mast::types::MastErrorCode;
    use std::sync::Arc;
    use std::time::Duration;

    async fn wait_until(registry: &Registry, id: &str) {
        for _ in 0..200 {
            if registry.contains(id) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        panic!("{id} was never registered");
    }

    #[tokio::test]
    async fn cancel_validates_its_arguments() {
        let registry = Registry::new();
        assert_eq!(registry.cancel(None, None).unwrap(), 0);
        assert_eq!(registry.cancel(Some(&["g1:x".to_owned()]), Some("g1:")).unwrap(), 0);
        for prefix in ["", "g1", "g1:x", "g 1:"] {
            let error = registry.cancel(None, Some(prefix)).unwrap_err();
            assert_eq!(error.code, MastErrorCode::BadInput, "{prefix}");
            assert!(error.message.starts_with("prefix: "), "{}", error.message);
        }
        let error = registry.cancel(Some(&["bad id".to_owned()]), None).unwrap_err();
        assert!(error.message.starts_with("requestId: "));
        let error = registry.run("bad id", async { Ok(1) }).await.unwrap_err();
        assert_eq!(error.code, MastErrorCode::BadInput);
    }

    #[tokio::test]
    async fn finished_tasks_leave_the_registry_and_panics_are_internal() {
        let registry = Arc::new(Registry::new());
        assert_eq!(registry.run("g1:search", async { Ok(7) }).await.unwrap(), 7);
        assert!(registry.is_empty());
        let failed: Result<u8, MastCmdError> = registry.run("g1:x", async { Err(bad_input("x", "y")) }).await;
        assert_eq!(failed.unwrap_err().message, "x: y");
        let panicked: Result<u8, MastCmdError> = registry.run("g1:p", async { panic!("boom") }).await;
        let error = panicked.unwrap_err();
        assert_eq!(error.code, MastErrorCode::Decode);
        assert!(!error.message.contains("boom"));
        assert!(registry.is_empty());
        let slow = {
            let registry = Arc::clone(&registry);
            tokio::spawn(async move {
                registry
                    .run("g2:slow", async {
                        tokio::time::sleep(Duration::from_secs(30)).await;
                        Ok(())
                    })
                    .await
            })
        };
        wait_until(&registry, "g2:slow").await;
        slow.abort();
        let _ = slow.await;
        assert!(registry.is_empty());
    }
}

use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::error::MastError;
use crate::core::mast::errors::{retry_after_secs, Operation};

pub const BREAKER_THRESHOLD: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window {
    pub remaining_secs: u64,
    pub consecutive: u32,
}

#[derive(Debug, Default)]
struct State {
    consecutive: u32,
    open_until: Option<Instant>,
}

#[derive(Debug, Default)]
pub struct RateLimiter {
    state: Mutex<State>,
}

pub fn is_rate_limit(error: &MastError) -> bool {
    matches!(
        error,
        MastError::Status { status: 429, .. } | MastError::Status { status: 503, retry_after_secs: Some(_), .. }
    )
}

fn later(current: Option<Instant>, candidate: Instant) -> Instant {
    current.map_or(candidate, |existing| existing.max(candidate))
}

impl RateLimiter {
    pub fn new() -> Self {
        Self::default()
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub fn record(&self, error: &MastError, op: &Operation, now: Instant) -> u32 {
        let mut state = self.state();
        if is_rate_limit(error) {
            let header = match error {
                MastError::Status { retry_after_secs, .. } => *retry_after_secs,
                _ => None,
            };
            state.consecutive += 1;
            let wait = Duration::from_secs(retry_after_secs(header, state.consecutive));
            state.open_until = Some(later(state.open_until, now + wait));
        } else if op.is_image() && matches!(error, MastError::Status { status: 503, .. }) {
            state.consecutive += 1;
            if state.consecutive >= BREAKER_THRESHOLD {
                let wait = Duration::from_secs(retry_after_secs(None, state.consecutive));
                state.open_until = Some(later(state.open_until, now + wait));
            }
        }
        state.consecutive
    }

    pub fn on_success(&self) {
        let mut state = self.state();
        state.consecutive = 0;
        state.open_until = None;
    }

    pub fn consecutive(&self) -> u32 {
        self.state().consecutive
    }

    pub fn open_window(&self, now: Instant) -> Option<Window> {
        let state = self.state();
        let until = state.open_until?;
        if now >= until {
            return None;
        }
        let remaining_secs = (until - now).as_millis().div_ceil(1000) as u64;
        Some(Window { remaining_secs, consecutive: state.consecutive })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::mast::errors::OpKind;
    use crate::core::mast::types::MastMissionId;

    fn status(code: u16, retry_after: Option<u64>) -> MastError {
        MastError::Status {
            status: code,
            url: "u".into(),
            body: String::new(),
            detail: None,
            content_type: None,
            retry_after_secs: retry_after,
        }
    }

    #[test]
    fn limiter_counts_consecutive_limits_and_opens_a_window() {
        let limiter = RateLimiter::new();
        let search = Operation::mission(MastMissionId::Jwst, OpKind::Search, "search");
        let thumb = Operation::mission(MastMissionId::Jwst, OpKind::Thumbnail, "get_image_thumbnail");
        let start = Instant::now();
        assert_eq!(limiter.open_window(start), None);
        assert_eq!(limiter.record(&status(429, Some(5)), &search, start), 1);
        assert_eq!(limiter.open_window(start), Some(Window { remaining_secs: 5, consecutive: 1 }));
        assert_eq!(limiter.open_window(start + Duration::from_millis(4500)), Some(Window { remaining_secs: 1, consecutive: 1 }));
        assert_eq!(limiter.open_window(start + Duration::from_secs(5)), None);
        assert_eq!(limiter.record(&status(503, None), &search, start), 1);
        assert_eq!(limiter.record(&status(503, None), &thumb, start), 2);
        assert_eq!(limiter.record(&status(429, None), &thumb, start), 3);
        assert_eq!(limiter.open_window(start), Some(Window { remaining_secs: 60, consecutive: 3 }));
        assert_eq!(limiter.record(&status(500, None), &thumb, start), 3);
        assert_eq!(limiter.record(&MastError::NotImage { url: "u".into() }, &thumb, start), 3);
        limiter.on_success();
        assert_eq!(limiter.consecutive(), 0);
        assert_eq!(limiter.open_window(start), None);
        assert!(is_rate_limit(&status(429, None)));
        assert!(is_rate_limit(&status(503, Some(1))));
        assert!(!is_rate_limit(&status(503, None)));
        assert!(!is_rate_limit(&status(500, Some(1))));
    }
}

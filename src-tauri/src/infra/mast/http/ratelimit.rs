use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::error::MastError;
use crate::core::mast::errors::{retry_after_secs, Operation};

pub const BREAKER_THRESHOLD: u32 = 3;
pub const SERVER_TROUBLE_WINDOW: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window {
    pub remaining_secs: u64,
    pub consecutive: u32,
}

#[derive(Debug, Default)]
struct State {
    consecutive: u32,
    counted_at: Option<Instant>,
    open_until: Option<Instant>,
    troubled_at: Option<Instant>,
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

    pub fn record(&self, error: &MastError, op: &Operation, sent: Instant, now: Instant) -> u32 {
        let mut state = self.state();
        let limited = is_rate_limit(error);
        let busy = !limited && op.is_image() && matches!(error, MastError::Status { status: 503, .. });
        if !limited && !busy {
            return state.consecutive;
        }
        if state.counted_at.is_none_or(|counted| sent > counted) {
            state.consecutive += 1;
            state.counted_at = Some(now);
        }
        let header = match error {
            MastError::Status { retry_after_secs, .. } if limited => *retry_after_secs,
            _ => None,
        };
        if limited || state.consecutive >= BREAKER_THRESHOLD {
            let wait = Duration::from_secs(retry_after_secs(header, state.consecutive));
            state.open_until = Some(later(state.open_until, now + wait));
        }
        state.consecutive
    }

    pub fn on_success(&self) {
        let mut state = self.state();
        state.consecutive = 0;
        state.counted_at = None;
        state.open_until = None;
        state.troubled_at = None;
    }

    pub fn note_server_trouble(&self, now: Instant) {
        let mut state = self.state();
        state.troubled_at = Some(later(state.troubled_at, now));
    }

    pub fn server_trouble(&self, now: Instant) -> bool {
        self.state().troubled_at.is_some_and(|at| now < at + SERVER_TROUBLE_WINDOW)
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
        let at = |secs: u64| start + Duration::from_secs(secs);
        assert_eq!(limiter.open_window(start), None);
        assert_eq!(limiter.record(&status(429, Some(5)), &search, start, start), 1);
        assert_eq!(limiter.open_window(start), Some(Window { remaining_secs: 5, consecutive: 1 }));
        assert_eq!(limiter.open_window(start + Duration::from_millis(4500)), Some(Window { remaining_secs: 1, consecutive: 1 }));
        assert_eq!(limiter.open_window(at(5)), None);
        assert_eq!(limiter.record(&status(503, None), &search, at(6), at(6)), 1);
        assert_eq!(limiter.record(&status(503, None), &thumb, at(7), at(7)), 2);
        assert_eq!(limiter.open_window(at(7)), None);
        assert_eq!(limiter.record(&status(429, None), &thumb, at(8), at(8)), 3);
        assert_eq!(limiter.open_window(at(8)), Some(Window { remaining_secs: 60, consecutive: 3 }));
        assert_eq!(limiter.record(&status(500, None), &thumb, at(9), at(9)), 3);
        assert_eq!(limiter.record(&MastError::NotImage { url: "u".into() }, &thumb, at(9), at(9)), 3);
        limiter.on_success();
        assert_eq!(limiter.consecutive(), 0);
        assert_eq!(limiter.open_window(at(9)), None);
        assert!(is_rate_limit(&status(429, None)));
        assert!(is_rate_limit(&status(503, Some(1))));
        assert!(!is_rate_limit(&status(503, None)));
        assert!(!is_rate_limit(&status(500, Some(1))));
    }

    #[test]
    fn limits_answered_to_requests_already_in_flight_are_one_episode() {
        let limiter = RateLimiter::new();
        let thumb = Operation::mission(MastMissionId::Jwst, OpKind::Thumbnail, "get_image_thumbnail");
        let sent = Instant::now();
        let answered = sent + Duration::from_millis(300);
        for _ in 0..4 {
            assert_eq!(limiter.record(&status(429, Some(5)), &thumb, sent, answered), 1);
        }
        assert_eq!(limiter.open_window(answered), Some(Window { remaining_secs: 5, consecutive: 1 }));
        let resumed = answered + Duration::from_secs(5);
        assert_eq!(limiter.open_window(resumed), None);
        assert_eq!(limiter.record(&status(429, Some(5)), &thumb, resumed, resumed + Duration::from_millis(300)), 2);
    }

    #[test]
    fn parallel_image_503s_without_retry_after_are_one_episode() {
        let limiter = RateLimiter::new();
        let thumb = Operation::mission(MastMissionId::Jwst, OpKind::Thumbnail, "get_image_thumbnail");
        let sent = Instant::now();
        let answered = sent + Duration::from_millis(300);
        for _ in 0..4 {
            assert_eq!(limiter.record(&status(503, None), &thumb, sent, answered), 1);
        }
        assert_eq!(limiter.open_window(answered), None);
        let second = answered + Duration::from_secs(1);
        assert_eq!(limiter.record(&status(503, None), &thumb, second, second), 2);
        let third = second + Duration::from_secs(1);
        assert_eq!(limiter.record(&status(503, None), &thumb, third, third), 3);
        assert_eq!(limiter.open_window(third), Some(Window { remaining_secs: 60, consecutive: 3 }));
    }

    #[test]
    fn server_trouble_lasts_ten_seconds_or_until_an_answer() {
        let limiter = RateLimiter::new();
        let start = Instant::now();
        assert!(!limiter.server_trouble(start));
        limiter.note_server_trouble(start);
        assert!(limiter.server_trouble(start + Duration::from_millis(9_999)));
        assert!(!limiter.server_trouble(start + SERVER_TROUBLE_WINDOW));
        limiter.note_server_trouble(start + Duration::from_secs(5));
        limiter.note_server_trouble(start + Duration::from_secs(1));
        assert!(limiter.server_trouble(start + Duration::from_secs(12)));
        assert_eq!(limiter.open_window(start + Duration::from_secs(6)), None, "server trouble never pauses requests");
        assert_eq!(limiter.consecutive(), 0);
        limiter.on_success();
        assert!(!limiter.server_trouble(start + Duration::from_secs(6)));
    }

    #[test]
    fn a_success_starts_a_new_count() {
        let limiter = RateLimiter::new();
        let thumb = Operation::mission(MastMissionId::Jwst, OpKind::Thumbnail, "get_image_thumbnail");
        let sent = Instant::now();
        let answered = sent + Duration::from_millis(300);
        assert_eq!(limiter.record(&status(429, Some(5)), &thumb, sent, answered), 1);
        limiter.on_success();
        assert_eq!(limiter.record(&status(429, Some(5)), &thumb, sent, answered + Duration::from_millis(10)), 1);
    }
}

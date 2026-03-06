use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

pub struct RateLimiter {
    window: Duration,
    max_requests: usize,
    buckets: HashMap<String, VecDeque<Instant>>,
}

impl RateLimiter {
    pub fn new(window_secs: u64, max_requests: usize) -> Self {
        Self {
            window: Duration::from_secs(window_secs),
            max_requests,
            buckets: HashMap::new(),
        }
    }

    pub fn check(&mut self, scope_key: &str) -> bool {
        if self.max_requests == 0 {
            return true;
        }
        let now = Instant::now();
        let cutoff = now - self.window;
        let bucket = self.buckets.entry(scope_key.to_string()).or_default();

        while bucket.front().is_some_and(|&t| t < cutoff) {
            bucket.pop_front();
        }

        if bucket.len() >= self.max_requests {
            return false;
        }
        bucket.push_back(now);
        true
    }

    pub fn evict_stale(&mut self) {
        let now = Instant::now();
        let cutoff = now - self.window;
        self.buckets.retain(|_, bucket| {
            while bucket.front().is_some_and(|&t| t < cutoff) {
                bucket.pop_front();
            }
            !bucket.is_empty()
        });
    }
}

#[cfg(test)]
mod tests {
    use super::RateLimiter;

    #[test]
    fn allows_within_limit() {
        let mut limiter = RateLimiter::new(60, 3);
        assert!(limiter.check("scope-a"));
        assert!(limiter.check("scope-a"));
        assert!(limiter.check("scope-a"));
        assert!(!limiter.check("scope-a"));
    }

    #[test]
    fn separate_scopes_are_independent() {
        let mut limiter = RateLimiter::new(60, 1);
        assert!(limiter.check("scope-a"));
        assert!(!limiter.check("scope-a"));
        assert!(limiter.check("scope-b"));
    }

    #[test]
    fn zero_max_allows_all() {
        let mut limiter = RateLimiter::new(60, 0);
        for _ in 0..100 {
            assert!(limiter.check("scope-a"));
        }
    }

    #[test]
    fn evict_stale_removes_empty_buckets() {
        let mut limiter = RateLimiter::new(0, 5);
        limiter.check("scope-a");
        limiter.evict_stale();
        assert!(limiter.buckets.is_empty());
    }
}

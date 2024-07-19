use std::time::Duration;

/// Exponential delay with a maximum delay
///
/// The delay will be doubled every time it reaches the maximum delay.
/// When reset is called, the delay will be reset to the initial delay.
///
/// By default the initial delay is 50ms and the maximum delay is 60s.
pub struct ExponentialDelay {
    current: Duration,
    initial: Duration,
    max: Duration,
}

impl ExponentialDelay {
    pub fn new(initial: Duration, max: Duration) -> Self {
        ExponentialDelay {
            current: initial,
            initial,
            max,
        }
    }

    pub fn next(&mut self) -> Duration {
        let current = self.current;
        self.current = (self.current * 2).min(self.max);
        current
    }

    pub fn reset(&mut self) {
        self.current = self.initial;
    }
}

impl Default for ExponentialDelay {
    fn default() -> Self {
        Self::new(Duration::from_millis(50), Duration::from_secs(60))
    }
}

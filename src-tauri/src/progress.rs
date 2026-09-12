//! Throttled progress reporting from callbacks that fire per chunk.

use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter};

pub struct Throttle {
    every: Duration,
    last: Option<Instant>,
}

impl Throttle {
    pub fn new(millis: u64) -> Self {
        Self {
            every: Duration::from_millis(millis),
            last: None,
        }
    }

    /// True at most once per interval; the first call is always ready.
    pub fn ready(&mut self) -> bool {
        if self.last.is_some_and(|last| last.elapsed() < self.every) {
            return false;
        }
        self.last = Some(Instant::now());
        true
    }
}

#[derive(Debug, Clone, Serialize, ts_rs::TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct TransferProgress {
    pub received_bytes: u64,
    pub total_bytes: u64,
    pub bytes_per_second: f64,
}

impl From<&gameyfin_core::Progress> for TransferProgress {
    fn from(p: &gameyfin_core::Progress) -> Self {
        Self {
            received_bytes: p.received_bytes,
            total_bytes: p.total_bytes.unwrap_or(0),
            bytes_per_second: p.bytes_per_second,
        }
    }
}

/// A download callback emitting `event` at most five times a second.
pub fn emitter(app: &AppHandle, event: &'static str) -> impl FnMut(gameyfin_core::Progress) + Send {
    let app = app.clone();
    let mut throttle = Throttle::new(200);
    move |p| {
        if throttle.ready() {
            let _ = app.emit(event, TransferProgress::from(&p));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_update_passes_and_bursts_are_dropped() {
        let mut throttle = Throttle::new(60_000);
        assert!(throttle.ready());
        assert!(!throttle.ready());
    }
}

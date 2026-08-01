//! Download pacing, resume and retry decisions — pure, no clock, no network.
//!
//! `Instant` is a parameter everywhere so the pacer can be driven through a
//! simulated download in a unit test without sleeping.

use std::time::{Duration, Instant};

/// Rate and ETA computed for one emitted progress frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sample {
    pub bytes_per_sec: Option<u64>,
    pub eta_secs: Option<u64>,
}

/// Caps how often a transfer is allowed to report progress.
///
/// The callback in `download_model_with_progress` fires once per network chunk,
/// and every call becomes a `PostMessageW` to the Windows UI thread plus a React
/// re-render. At 8-64 KB per chunk a 491 MB model produces tens of thousands of
/// them, the producer outruns the consumer, and past the per-thread posted
/// message limit the payloads are dropped outright — the percentage freezes
/// while bytes keep arriving. Ten frames a second is more than the eye reads.
#[derive(Debug)]
pub struct ProgressPacer {
    interval: Duration,
    last_emit: Option<Instant>,
    last_bytes: u64,
    rate_ewma: Option<f64>,
}

impl ProgressPacer {
    pub const INTERVAL: Duration = Duration::from_millis(100);
    const ALPHA: f64 = 0.3;

    pub fn new() -> Self {
        Self {
            interval: Self::INTERVAL,
            last_emit: None,
            last_bytes: 0,
            rate_ewma: None,
        }
    }

    /// `Some` when this frame should be emitted. `force` is for anything the user
    /// would notice missing: a phase change, the terminal `done`, a retry boundary.
    pub fn tick(
        &mut self,
        now: Instant,
        downloaded: u64,
        total: Option<u64>,
        force: bool,
    ) -> Option<Sample> {
        let since_last = self.last_emit.map(|t| now.saturating_duration_since(t));
        if let Some(elapsed) = since_last {
            if !force && elapsed < self.interval {
                return None;
            }
        }

        // A retry that restarts or resumes moves `downloaded` backwards; there is no
        // rate to read from that, so the previous estimate stands.
        if let Some(elapsed) = since_last {
            if elapsed > Duration::ZERO && downloaded >= self.last_bytes {
                let instant = (downloaded - self.last_bytes) as f64 / elapsed.as_secs_f64();
                self.rate_ewma = Some(match self.rate_ewma {
                    Some(previous) => Self::ALPHA * instant + (1.0 - Self::ALPHA) * previous,
                    None => instant,
                });
            }
        }
        self.last_emit = Some(now);
        self.last_bytes = downloaded;

        let rate = self.rate_ewma.filter(|r| *r >= 1.0);
        Some(Sample {
            bytes_per_sec: rate.map(|r| r as u64),
            eta_secs: match (total, rate) {
                (Some(total), Some(rate)) if total >= downloaded => {
                    Some(((total - downloaded) as f64 / rate).round() as u64)
                }
                _ => None,
            },
        })
    }
}

impl Default for ProgressPacer {
    fn default() -> Self {
        Self::new()
    }
}

/// What to do with the `.part` file already on disk, given the server's answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResumeAction {
    Append { from: u64, total: Option<u64> },
    Restart { total: Option<u64> },
    DiscardAndRestart,
    Fail(u16),
}

/// Decide how to continue a transfer from the response head alone.
///
/// The total **must** come from the `/N` tail of `Content-Range`: on a 206 the
/// `Content-Length` is the remaining bytes, so using it as the total makes a
/// resumed download start its bar at 0% and reach 100% early.
pub fn plan_resume(
    part_len: u64,
    status: u16,
    content_length: Option<u64>,
    content_range: Option<&str>,
) -> ResumeAction {
    match status {
        206 => ResumeAction::Append {
            from: part_len,
            total: content_range
                .and_then(total_from_content_range)
                .or_else(|| content_length.map(|remaining| part_len + remaining)),
        },
        // The server ignored `Range`, or `If-Range` did not match and it is sending
        // the whole object again. Either way the bytes on disk are not a prefix.
        200 => ResumeAction::Restart {
            total: content_length,
        },
        // The `.part` is at or past the end of the remote object.
        416 => ResumeAction::DiscardAndRestart,
        other => ResumeAction::Fail(other),
    }
}

/// `bytes 276485400-491400031/491400032` → `491400032`. `*/N` and a missing tail
/// both fall through to `None`.
fn total_from_content_range(header: &str) -> Option<u64> {
    header.rsplit('/').next()?.trim().parse().ok()
}

/// How many consecutive attempts may fail without moving a byte before giving up.
pub const MAX_RETRIES: u32 = 5;

pub fn is_retryable_status(status: u16) -> bool {
    status == 408 || status == 429 || (500..600).contains(&status)
}

/// Delay before the next attempt, given how many attempts in a row have failed
/// without moving a byte. `None` means the budget is spent.
///
/// The counter is *consecutive*: an attempt that advanced `downloaded` resets it
/// to zero, so a flaky link that keeps making headway still finishes 1.1 GB.
pub fn backoff(consecutive_failures: u32) -> Option<Duration> {
    match consecutive_failures {
        0 => Some(Duration::ZERO),
        n if n <= MAX_RETRIES => Some(Duration::from_secs(1 << (n - 1))),
        _ => None,
    }
}

/// Render an error together with everything that caused it.
///
/// `reqwest` maps every body failure — reset, unexpected EOF, TLS close, read
/// timeout — to one kind whose `Display` is the six words "error decoding
/// response body" and never walks `source()`. Calling `to_string()` on it throws
/// the actual reason away at the exact point of failure.
pub fn describe(error: &dyn std::error::Error) -> String {
    let mut rendered = error.to_string();
    let mut cause = error.source();
    while let Some(current) = cause {
        rendered.push_str(": ");
        rendered.push_str(&current.to_string());
        cause = current.source();
    }
    rendered
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The regression test for the freeze: it turns "how many Tauri events does a
    /// 491 MB download produce" into a number a reviewer can read.
    #[test]
    fn a_full_download_emits_at_most_ten_events_per_second() {
        let start = Instant::now();
        let mut pacer = ProgressPacer::new();
        let size = 491_400_032u64;
        let chunks = 60_000u64;
        let seconds = 25u64;

        let mut emitted = 0usize;
        let mut last = None;
        for chunk in 1..=chunks {
            let now = start + Duration::from_micros(chunk * seconds * 1_000_000 / chunks);
            if let Some(sample) = pacer.tick(now, chunk * size / chunks, Some(size), false) {
                emitted += 1;
                last = Some(sample);
            }
        }

        assert!(
            emitted <= 260,
            "{chunks} chunks produced {emitted} events; the pacer allows ~{}",
            seconds * 10 + 1
        );
        assert!(
            emitted >= 200,
            "only {emitted} events in {seconds}s — the pacer went silent"
        );

        let last = last.expect("a download must report at least once");
        let expected_rate = size / seconds;
        let rate = last.bytes_per_sec.expect("a moving download has a rate");
        assert!(
            rate.abs_diff(expected_rate) < expected_rate / 5,
            "rate {rate} B/s is nowhere near the actual {expected_rate} B/s"
        );
        assert_eq!(last.eta_secs, Some(0), "the last frame has nothing left");
    }

    #[test]
    fn phase_changes_and_completion_bypass_the_pacer() {
        let start = Instant::now();
        let mut pacer = ProgressPacer::new();

        assert!(pacer.tick(start, 0, Some(100), false).is_some());
        assert!(
            pacer
                .tick(start + Duration::from_millis(1), 10, Some(100), false)
                .is_none(),
            "a routine chunk inside the interval is throttled"
        );
        assert!(
            pacer
                .tick(start + Duration::from_millis(2), 20, Some(100), true)
                .is_some(),
            "a phase change must reach the UI immediately"
        );
        assert!(
            pacer
                .tick(start + Duration::from_millis(3), 100, Some(100), true)
                .is_some(),
            "swallowing the terminal frame would stick the bar below 100% forever"
        );
    }

    #[test]
    fn resuming_reports_total_from_content_range_not_remaining_length() {
        // The exact byte counts off the reporting user's disk: a `.part` at 56% of
        // the catalog size, with 43% of it never rendered.
        let part_len = 276_485_400u64;
        let remaining = 214_914_632u64;
        let total = 491_400_032u64;

        assert_eq!(
            plan_resume(
                part_len,
                206,
                Some(remaining),
                Some("bytes 276485400-491400031/491400032")
            ),
            ResumeAction::Append {
                from: part_len,
                total: Some(total)
            }
        );

        // No header: the sum, still never the bare remainder.
        assert_eq!(
            plan_resume(part_len, 206, Some(remaining), None),
            ResumeAction::Append {
                from: part_len,
                total: Some(total)
            }
        );

        // An unsatisfied `*/N` tail must not be read as a total either.
        assert_eq!(
            plan_resume(part_len, 206, None, Some("bytes 276485400-491400031/*")),
            ResumeAction::Append {
                from: part_len,
                total: None
            }
        );
    }

    #[test]
    fn a_server_that_ignores_range_restarts_from_zero() {
        assert_eq!(
            plan_resume(276_485_400, 200, Some(491_400_032), None),
            ResumeAction::Restart {
                total: Some(491_400_032)
            }
        );
    }

    #[test]
    fn a_part_file_longer_than_the_remote_object_is_discarded() {
        assert_eq!(
            plan_resume(600_000_000, 416, None, None),
            ResumeAction::DiscardAndRestart
        );
    }

    #[test]
    fn an_attempt_that_moved_bytes_resets_the_retry_budget() {
        // A flaky link that keeps making headway is never given up on.
        let mut consecutive = 0u32;
        for round in 0..9 {
            assert!(
                backoff(consecutive).is_some(),
                "round {round} moved bytes and must be retried"
            );
            consecutive = 0;
        }

        // A link that moves nothing walks the rungs and then stops.
        let mut consecutive = 0u32;
        let mut delays = Vec::new();
        loop {
            consecutive += 1;
            match backoff(consecutive) {
                Some(delay) => delays.push(delay),
                None => break,
            }
        }
        assert_eq!(delays, [1, 2, 4, 8, 16].map(Duration::from_secs).to_vec());
        assert_eq!(consecutive, MAX_RETRIES + 1);
    }

    #[test]
    fn a_404_is_not_retried_but_a_503_is() {
        assert!(!is_retryable_status(404));
        assert!(!is_retryable_status(403));
        assert!(!is_retryable_status(416));
        assert!(is_retryable_status(408));
        assert!(is_retryable_status(429));
        assert!(is_retryable_status(500));
        assert!(is_retryable_status(503));
        assert_eq!(plan_resume(0, 404, None, None), ResumeAction::Fail(404));
        assert_eq!(plan_resume(0, 503, None, None), ResumeAction::Fail(503));
    }

    /// Synthetic on purpose: `reqwest::Error` has no public constructor, and this
    /// asserts the shape its chain has — decode wrapping body wrapping the io error.
    #[derive(Debug)]
    struct Layer {
        message: &'static str,
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    }

    impl std::fmt::Display for Layer {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(self.message)
        }
    }

    impl std::error::Error for Layer {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            self.source
                .as_ref()
                .map(|e| e.as_ref() as &(dyn std::error::Error + 'static))
        }
    }

    #[test]
    fn the_error_string_carries_the_transport_reason() {
        let body = Layer {
            message: "request or response body error",
            source: Some(Box::new(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "operation timed out",
            ))),
        };
        let decode = Layer {
            message: "error decoding response body",
            source: Some(Box::new(body)),
        };

        assert_eq!(
            describe(&decode),
            "error decoding response body: request or response body error: operation timed out"
        );
    }
}

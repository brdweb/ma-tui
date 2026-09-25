//! Worker-side stream health policy and allocation-free callback telemetry.
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

const CALLBACK_TIMEOUT: Duration = Duration::from_secs(1);
const RECOVERY_WINDOW: Duration = Duration::from_secs(10);
// Pinned CPAL/ALSA timing query failure observed to clear without recreating
// the stream. No other I/O error is covered by this narrow grace period.
const ALSA_TIMING_ERROR: &str =
    "ALSA function 'snd_pcm_avail_delay' failed with error 'I/O error (5)'";

/// One producer (the CPAL callback), with snapshots read by the audio worker.
/// A new instance belongs to each stream; it is never reset while callbacks run.
pub(crate) struct CallbackStats {
    origin: Instant,
    count: AtomicU64,
    last_elapsed_ns: AtomicU64,
    max_gap_ns: AtomicU64,
    min_frames: AtomicU64,
    max_frames: AtomicU64,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct CallbackSnapshot {
    pub(crate) count: u64,
    pub(crate) last_callback: Option<Instant>,
    pub(crate) max_gap: Duration,
    pub(crate) min_frames: usize,
    pub(crate) max_frames: usize,
}

impl CallbackStats {
    pub(crate) fn new() -> Self {
        Self {
            origin: Instant::now(),
            count: AtomicU64::new(0),
            last_elapsed_ns: AtomicU64::new(0),
            max_gap_ns: AtomicU64::new(0),
            min_frames: AtomicU64::new(u64::MAX),
            max_frames: AtomicU64::new(0),
        }
    }

    /// Called from the audio callback: no allocation, locks, or I/O.
    pub(crate) fn observe(&self, frames: usize) {
        self.observe_at(Instant::now(), frames);
    }

    fn observe_at(&self, now: Instant, frames: usize) {
        let elapsed = now
            .saturating_duration_since(self.origin)
            .as_nanos()
            .min(u128::from(u64::MAX)) as u64;
        let previous = self.last_elapsed_ns.swap(elapsed, Ordering::Relaxed);
        if self.count.load(Ordering::Relaxed) != 0 {
            self.max_gap_ns
                .fetch_max(elapsed.saturating_sub(previous), Ordering::Relaxed);
        }
        self.min_frames.fetch_min(frames as u64, Ordering::Relaxed);
        self.max_frames.fetch_max(frames as u64, Ordering::Relaxed);
        // Publish the completed update. Snapshots can include newer individual
        // fields if another callback runs concurrently; counts never overstate
        // progress, and the latest timestamp is safe for the stall watchdog.
        self.count.fetch_add(1, Ordering::Release);
    }

    pub(crate) fn snapshot(&self) -> CallbackSnapshot {
        let count = self.count.load(Ordering::Acquire);
        CallbackSnapshot {
            count,
            last_callback: (count != 0).then(|| {
                self.origin + Duration::from_nanos(self.last_elapsed_ns.load(Ordering::Relaxed))
            }),
            max_gap: Duration::from_nanos(self.max_gap_ns.load(Ordering::Relaxed)),
            min_frames: if count == 0 {
                0
            } else {
                self.min_frames.load(Ordering::Relaxed) as usize
            },
            max_frames: if count == 0 {
                0
            } else {
                self.max_frames.load(Ordering::Relaxed) as usize
            },
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum HealthDecision {
    Healthy,
    Recovering,
    Recovered,
    Failed(String),
}

struct PendingRecovery {
    started: Instant,
    callbacks: u64,
    xruns: u64,
    saw_timing_error: bool,
}

/// Construct after each stream starts; drop when that stream is cleared.
/// Only opt into recovery for the verified Linux ALSA backend. CPAL prepares
/// after XRUN and can continue after a transient availability/delay query error.
pub(crate) struct HealthMonitor {
    started: Instant,
    allow_alsa_recovery: bool,
    xrun_message: String,
    callback_timeout: Duration,
    recent_xruns: [Option<Instant>; 3],
    observed_xruns: u64,
    recovered_xruns: u64,
    recent_timing_errors: [Option<Instant>; 3],
    observed_timing_errors: u64,
    recovered_timing_errors: u64,
    pending: Option<PendingRecovery>,
    failure: Option<String>,
}

impl HealthMonitor {
    pub(crate) fn new(started: Instant, allow_alsa_recovery: bool) -> Self {
        Self::with_callback_timeout(started, allow_alsa_recovery, CALLBACK_TIMEOUT)
    }

    /// The default permits 1s between callbacks and 2s for the first callback.
    /// Callers with a deliberately longer output period can supply more margin.
    pub(crate) fn with_callback_timeout(
        started: Instant,
        allow_alsa_recovery: bool,
        callback_timeout: Duration,
    ) -> Self {
        Self {
            started,
            allow_alsa_recovery,
            xrun_message: cpal::ErrorKind::Xrun.to_string(),
            callback_timeout: callback_timeout.max(CALLBACK_TIMEOUT),
            recent_xruns: [None; 3],
            observed_xruns: 0,
            recovered_xruns: 0,
            recent_timing_errors: [None; 3],
            observed_timing_errors: 0,
            recovered_timing_errors: 0,
            pending: None,
            failure: None,
        }
    }

    pub(crate) fn observed_xruns(&self) -> u64 {
        self.observed_xruns
    }

    pub(crate) fn recovered_xruns(&self) -> u64 {
        self.recovered_xruns
    }

    /// Episodes, not individual reports: CPAL may repeat a timing error rapidly
    /// until its next successful callback.
    pub(crate) fn observed_timing_errors(&self) -> u64 {
        self.observed_timing_errors
    }

    pub(crate) fn recovered_timing_errors(&self) -> u64 {
        self.recovered_timing_errors
    }

    pub(crate) fn is_recovering(&self) -> bool {
        self.pending.is_some() && self.failure.is_none()
    }

    /// Allow room for the actual callback period without shrinking the default
    /// scheduling margin. The worker may update this after observing frame sizes.
    pub(crate) fn set_callback_timeout(&mut self, timeout: Duration) {
        self.callback_timeout = timeout.max(CALLBACK_TIMEOUT);
    }

    fn fail(&mut self, detail: &str) -> HealthDecision {
        self.failure = Some(detail.to_owned());
        HealthDecision::Failed(detail.to_owned())
    }

    pub(crate) fn poll(
        &mut self,
        now: Instant,
        error: Option<&str>,
        callbacks: CallbackSnapshot,
    ) -> HealthDecision {
        if let Some(detail) = &self.failure {
            return HealthDecision::Failed(detail.clone());
        }
        if let Some(error) = error {
            // Do not infer recovery from arbitrary driver text. Other error
            // messages, including a failed ALSA prepare(), stay fatal.
            let is_xrun = error == self.xrun_message;
            let is_timing_error = error == ALSA_TIMING_ERROR;
            if !self.allow_alsa_recovery || (!is_xrun && !is_timing_error) {
                return self.fail(error);
            }
            if is_xrun {
                self.observed_xruns = self.observed_xruns.saturating_add(1);
                self.recent_xruns.rotate_left(1);
                self.recent_xruns[2] = Some(now);
                if self.recent_xruns[0]
                    .is_some_and(|oldest| now.saturating_duration_since(oldest) <= RECOVERY_WINDOW)
                {
                    return self.fail("Repeated audio output underruns");
                }
            } else if !self.pending.as_ref().is_some_and(|p| p.saw_timing_error) {
                self.observed_timing_errors = self.observed_timing_errors.saturating_add(1);
                self.recent_timing_errors.rotate_left(1);
                self.recent_timing_errors[2] = Some(now);
                if self.recent_timing_errors[0]
                    .is_some_and(|oldest| now.saturating_duration_since(oldest) <= RECOVERY_WINDOW)
                {
                    return self.fail(&format!(
                        "Repeated audio output timing errors: {ALSA_TIMING_ERROR}"
                    ));
                }
            }
            // Repeated or mixed errors must not extend the original deadline.
            let pending = self.pending.get_or_insert(PendingRecovery {
                started: now,
                callbacks: callbacks.count,
                xruns: 0,
                saw_timing_error: false,
            });
            // Progress preceding a newly reported error cannot prove that the
            // new interruption recovered; require a callback after this poll.
            pending.callbacks = callbacks.count;
            if is_xrun {
                pending.xruns = pending.xruns.saturating_add(1);
            } else {
                pending.saw_timing_error = true;
            }
        }

        let timing_error_pending = self.pending.as_ref().is_some_and(|p| p.saw_timing_error);
        match callbacks.last_callback {
            Some(last) if now.saturating_duration_since(last) >= self.callback_timeout => {
                if timing_error_pending {
                    return self.fail(ALSA_TIMING_ERROR);
                }
                return self.fail("Audio output callbacks stalled");
            }
            None if now.saturating_duration_since(self.started)
                >= self.callback_timeout.saturating_mul(2) =>
            {
                if timing_error_pending {
                    return self.fail(ALSA_TIMING_ERROR);
                }
                return self.fail("Audio output callbacks did not start");
            }
            _ => {}
        }

        if let Some(pending) = &self.pending {
            // If the worker itself woke late but callbacks have resumed, keep
            // the now-working stream instead of rebuilding it unnecessarily.
            if error.is_none() && callbacks.count > pending.callbacks {
                let recovered = pending.xruns;
                let recovered_timing_error = pending.saw_timing_error;
                self.pending = None;
                self.recovered_xruns = self.recovered_xruns.saturating_add(recovered);
                if recovered_timing_error {
                    self.recovered_timing_errors = self.recovered_timing_errors.saturating_add(1);
                }
                return HealthDecision::Recovered;
            }
            if now.saturating_duration_since(pending.started) >= self.callback_timeout {
                if pending.saw_timing_error {
                    return self.fail(ALSA_TIMING_ERROR);
                }
                return self.fail("Audio output did not recover after a buffer underrun");
            }
            return HealthDecision::Recovering;
        }
        HealthDecision::Healthy
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(count: u64, last_callback: Option<Instant>) -> CallbackSnapshot {
        CallbackSnapshot {
            count,
            last_callback,
            max_gap: Duration::ZERO,
            min_frames: 512,
            max_frames: 512,
        }
    }

    #[test]
    fn callback_stats_record_real_frame_bounds_and_gaps_without_first_gap() {
        let stats = CallbackStats::new();
        let origin = stats.origin;
        let empty = stats.snapshot();
        assert_eq!(empty.count, 0);
        assert_eq!(empty.last_callback, None);
        assert_eq!((empty.min_frames, empty.max_frames), (0, 0));
        stats.observe_at(origin, 1024);
        stats.observe_at(origin + Duration::from_millis(20), 512);
        stats.observe_at(origin + Duration::from_millis(55), 2048);
        let result = stats.snapshot();
        assert_eq!(result.count, 3);
        assert_eq!(
            result.last_callback,
            Some(origin + Duration::from_millis(55))
        );
        assert_eq!(result.max_gap, Duration::from_millis(35));
        assert_eq!((result.min_frames, result.max_frames), (512, 2048));
    }

    #[test]
    fn isolated_alsa_xrun_waits_for_new_callback_then_recovers_in_place() {
        let now = Instant::now();
        let mut monitor = HealthMonitor::new(now, true);
        let xrun = cpal::ErrorKind::Xrun.to_string();
        assert_eq!(
            monitor.poll(now, Some(&xrun), snapshot(5, Some(now))),
            HealthDecision::Recovering
        );
        assert!(monitor.is_recovering());
        let later = now + Duration::from_millis(20);
        assert_eq!(
            monitor.poll(later, None, snapshot(5, Some(now))),
            HealthDecision::Recovering
        );
        assert_eq!(
            monitor.poll(later, None, snapshot(6, Some(later))),
            HealthDecision::Recovered
        );
        assert!(!monitor.is_recovering());
        assert_eq!(monitor.observed_xruns(), 1);
        assert_eq!(monitor.recovered_xruns(), 1);
        assert_eq!(
            monitor.poll(later, None, snapshot(6, Some(later))),
            HealthDecision::Healthy
        );
    }

    #[test]
    fn unknown_and_non_alsa_errors_fail_without_suppression() {
        let now = Instant::now();
        let xrun = cpal::ErrorKind::Xrun.to_string();
        for (allow_alsa, error) in [
            (false, xrun.as_str()),
            (false, ALSA_TIMING_ERROR),
            (true, "ALSA I/O error (5)"),
            (
                true,
                "ALSA function 'snd_pcm_writei' failed with error 'I/O error (5)'",
            ),
            (
                true,
                "ALSA function 'snd_pcm_avail_delay' failed with error 'I/O error (5)' extra context",
            ),
            (true, "Device does not support suspend/resume"),
            (true, "A buffer underrun or overrun occurred. extra context"),
        ] {
            let mut monitor = HealthMonitor::new(now, allow_alsa);
            assert_eq!(
                monitor.poll(now, Some(error), snapshot(1, Some(now))),
                HealthDecision::Failed(error.to_owned())
            );
            assert_eq!(monitor.observed_xruns(), 0);
            assert_eq!(monitor.observed_timing_errors(), 0);
        }
    }

    #[test]
    fn timing_error_burst_requires_progress_after_the_last_report() {
        let now = Instant::now();
        let mut monitor = HealthMonitor::new(now, true);
        for index in 0..10 {
            let time = now + Duration::from_millis(index * 10);
            assert_eq!(
                monitor.poll(
                    time,
                    Some(ALSA_TIMING_ERROR),
                    snapshot(index + 1, Some(time))
                ),
                HealthDecision::Recovering
            );
        }
        let later = now + Duration::from_millis(100);
        assert_eq!(monitor.observed_timing_errors(), 1);
        assert_eq!(monitor.recovered_timing_errors(), 0);
        assert_eq!(monitor.observed_xruns(), 0);
        assert_eq!(
            monitor.poll(later, None, snapshot(10, Some(later))),
            HealthDecision::Recovering
        );
        assert_eq!(
            monitor.poll(later, None, snapshot(11, Some(later))),
            HealthDecision::Recovered
        );
        assert_eq!(monitor.recovered_timing_errors(), 1);
        assert!(!monitor.is_recovering());
    }

    #[test]
    fn persistent_timing_error_reports_do_not_extend_deadline_or_exhaust_episode_limit() {
        let now = Instant::now();
        let mut monitor = HealthMonitor::new(now, true);
        for index in 0..100 {
            let time = now + Duration::from_millis(index * 10);
            assert_eq!(
                monitor.poll(time, Some(ALSA_TIMING_ERROR), snapshot(0, None)),
                HealthDecision::Recovering
            );
        }
        assert_eq!(monitor.observed_timing_errors(), 1);
        assert_eq!(monitor.observed_xruns(), 0);
        assert_eq!(
            monitor.poll(
                now + CALLBACK_TIMEOUT,
                Some(ALSA_TIMING_ERROR),
                snapshot(0, None)
            ),
            HealthDecision::Failed(ALSA_TIMING_ERROR.into())
        );
        assert_eq!(
            monitor.poll(
                now + CALLBACK_TIMEOUT,
                None,
                snapshot(1, Some(now + CALLBACK_TIMEOUT))
            ),
            HealthDecision::Failed(ALSA_TIMING_ERROR.into())
        );
        assert_eq!(monitor.recovered_timing_errors(), 0);
    }

    #[test]
    fn distinct_timing_error_episodes_use_a_trailing_window() {
        let now = Instant::now();
        for (spacing, episodes, should_fail) in [(5, 3, true), (6, 4, false)] {
            let mut monitor = HealthMonitor::new(now, true);
            for index in 0..episodes {
                let time = now + Duration::from_secs(index * spacing);
                let decision = monitor.poll(
                    time,
                    Some(ALSA_TIMING_ERROR),
                    snapshot(index * 2 + 1, Some(time)),
                );
                if should_fail && index == episodes - 1 {
                    assert_eq!(
                        decision,
                        HealthDecision::Failed(format!(
                            "Repeated audio output timing errors: {ALSA_TIMING_ERROR}"
                        ))
                    );
                } else {
                    assert_eq!(decision, HealthDecision::Recovering);
                    assert_eq!(
                        monitor.poll(time, None, snapshot(index * 2 + 2, Some(time))),
                        HealthDecision::Recovered
                    );
                }
            }
            assert_eq!(monitor.observed_timing_errors(), episodes);
            assert_eq!(
                monitor.recovered_timing_errors(),
                episodes - u64::from(should_fail)
            );
        }
    }

    #[test]
    fn mixed_errors_keep_the_first_deadline_and_recover_both_counters_together() {
        let now = Instant::now();
        let xrun = cpal::ErrorKind::Xrun.to_string();
        for (first, second) in [
            (xrun.as_str(), ALSA_TIMING_ERROR),
            (ALSA_TIMING_ERROR, xrun.as_str()),
        ] {
            for resume in [false, true] {
                let mut monitor = HealthMonitor::new(now, true);
                assert_eq!(
                    monitor.poll(now, Some(first), snapshot(1, Some(now))),
                    HealthDecision::Recovering
                );
                let later = now + Duration::from_millis(900);
                assert_eq!(
                    monitor.poll(later, Some(second), snapshot(2, Some(later))),
                    HealthDecision::Recovering
                );
                assert_eq!(monitor.observed_xruns(), 1);
                assert_eq!(monitor.observed_timing_errors(), 1);
                assert_eq!(
                    monitor.poll(later, None, snapshot(2, Some(later))),
                    HealthDecision::Recovering
                );
                if resume {
                    assert_eq!(
                        monitor.poll(later, None, snapshot(3, Some(later))),
                        HealthDecision::Recovered
                    );
                    assert_eq!(monitor.recovered_xruns(), 1);
                    assert_eq!(monitor.recovered_timing_errors(), 1);
                } else {
                    assert_eq!(
                        monitor.poll(now + CALLBACK_TIMEOUT, None, snapshot(2, Some(later))),
                        HealthDecision::Failed(ALSA_TIMING_ERROR.into())
                    );
                    assert_eq!(monitor.recovered_xruns(), 0);
                    assert_eq!(monitor.recovered_timing_errors(), 0);
                }
            }
        }
    }

    #[test]
    fn third_xrun_inside_window_fails_even_if_first_two_recovered() {
        let now = Instant::now();
        let mut monitor = HealthMonitor::new(now, true);
        let xrun = cpal::ErrorKind::Xrun.to_string();
        for index in 0..2 {
            let time = now + Duration::from_secs(index * 4);
            assert_eq!(
                monitor.poll(time, Some(&xrun), snapshot(index * 2 + 1, Some(time))),
                HealthDecision::Recovering
            );
            assert_eq!(
                monitor.poll(time, None, snapshot(index * 2 + 2, Some(time))),
                HealthDecision::Recovered
            );
        }
        let time = now + RECOVERY_WINDOW;
        assert_eq!(
            monitor.poll(time, Some(&xrun), snapshot(5, Some(time))),
            HealthDecision::Failed("Repeated audio output underruns".into())
        );
        assert_eq!(monitor.observed_xruns(), 3);
        assert_eq!(monitor.recovered_xruns(), 2);
        assert!(!monitor.is_recovering());
        assert_eq!(
            monitor.poll(time, None, snapshot(100, Some(time))),
            HealthDecision::Failed("Repeated audio output underruns".into())
        );
    }

    #[test]
    fn xruns_outside_trailing_window_do_not_exhaust_recovery() {
        let now = Instant::now();
        let mut monitor = HealthMonitor::new(now, true);
        let xrun = cpal::ErrorKind::Xrun.to_string();
        for index in 0..4 {
            let time = now + Duration::from_secs(index * 6);
            assert_eq!(
                monitor.poll(time, Some(&xrun), snapshot(index * 2 + 1, Some(time))),
                HealthDecision::Recovering
            );
            assert_eq!(
                monitor.poll(time, None, snapshot(index * 2 + 2, Some(time))),
                HealthDecision::Recovered
            );
        }
        assert_eq!(monitor.observed_xruns(), 4);
        assert_eq!(monitor.recovered_xruns(), 4);
    }

    #[test]
    fn subsequent_xrun_does_not_extend_no_progress_deadline() {
        let now = Instant::now();
        let mut monitor = HealthMonitor::new(now, true);
        let xrun = cpal::ErrorKind::Xrun.to_string();
        assert_eq!(
            monitor.poll(now, Some(&xrun), snapshot(0, None)),
            HealthDecision::Recovering
        );
        assert_eq!(
            monitor.poll(
                now + Duration::from_millis(900),
                Some(&xrun),
                snapshot(0, None)
            ),
            HealthDecision::Recovering
        );
        assert_eq!(
            monitor.poll(now + CALLBACK_TIMEOUT, None, snapshot(0, None)),
            HealthDecision::Failed("Audio output did not recover after a buffer underrun".into())
        );
    }

    #[test]
    fn new_xrun_requires_new_progress_and_success_counts_both_observed_xruns() {
        let now = Instant::now();
        let mut monitor = HealthMonitor::new(now, true);
        let xrun = cpal::ErrorKind::Xrun.to_string();
        assert_eq!(
            monitor.poll(now, Some(&xrun), snapshot(1, Some(now))),
            HealthDecision::Recovering
        );
        let later = now + Duration::from_millis(20);
        assert_eq!(
            monitor.poll(later, Some(&xrun), snapshot(2, Some(later))),
            HealthDecision::Recovering
        );
        assert_eq!(
            monitor.poll(later, None, snapshot(3, Some(later))),
            HealthDecision::Recovered
        );
        assert_eq!(monitor.observed_xruns(), 2);
        assert_eq!(monitor.recovered_xruns(), 2);
    }

    #[test]
    fn callbacks_are_watched_even_without_reported_driver_errors() {
        let now = Instant::now();
        let mut monitor = HealthMonitor::new(now, true);
        assert_eq!(
            monitor.poll(now + Duration::from_millis(1999), None, snapshot(0, None)),
            HealthDecision::Healthy
        );
        assert_eq!(
            monitor.poll(now + Duration::from_secs(2), None, snapshot(0, None)),
            HealthDecision::Failed("Audio output callbacks did not start".into())
        );
        let mut monitor = HealthMonitor::new(now, true);
        assert_eq!(
            monitor.poll(now + CALLBACK_TIMEOUT, None, snapshot(1, Some(now))),
            HealthDecision::Failed("Audio output callbacks stalled".into())
        );
    }

    #[test]
    fn explicit_long_callback_timeout_keeps_slow_stream_alive() {
        let now = Instant::now();
        let mut monitor = HealthMonitor::with_callback_timeout(now, true, Duration::from_secs(3));
        assert_eq!(
            monitor.poll(now + Duration::from_secs(5), None, snapshot(0, None)),
            HealthDecision::Healthy
        );
        let last = now + Duration::from_secs(5);
        assert_eq!(
            monitor.poll(last + Duration::from_secs(2), None, snapshot(1, Some(last))),
            HealthDecision::Healthy
        );
    }

    #[test]
    fn observed_period_can_extend_watchdog_with_a_one_second_floor() {
        let now = Instant::now();
        let mut monitor = HealthMonitor::new(now, true);
        monitor.set_callback_timeout(Duration::from_secs(3));
        assert_eq!(
            monitor.poll(now + Duration::from_secs(5), None, snapshot(0, None)),
            HealthDecision::Healthy
        );
        let last = now + Duration::from_secs(5);
        assert_eq!(
            monitor.poll(
                last + Duration::from_millis(2999),
                None,
                snapshot(1, Some(last))
            ),
            HealthDecision::Healthy
        );
        assert_eq!(
            monitor.poll(last + Duration::from_secs(3), None, snapshot(1, Some(last))),
            HealthDecision::Failed("Audio output callbacks stalled".into())
        );

        let mut monitor = HealthMonitor::new(now, true);
        monitor.set_callback_timeout(Duration::from_millis(1));
        assert_eq!(
            monitor.poll(
                now + Duration::from_millis(999),
                None,
                snapshot(1, Some(now))
            ),
            HealthDecision::Healthy
        );
        assert_eq!(
            monitor.poll(now + CALLBACK_TIMEOUT, None, snapshot(1, Some(now))),
            HealthDecision::Failed("Audio output callbacks stalled".into())
        );
    }

    #[tokio::test]
    async fn worker_keeps_recovered_stream_alive_but_exits_on_repeated_xruns_or_stall() {
        use super::super::{
            formats_for_ranges, spawn_output_worker, AudioStatus, Gain, Output, SharedClock,
        };
        use sendspin::audio::{AudioBuffer, AudioFormat};
        use sendspin::protocol::messages::AudioFormatSpec;
        use std::sync::{
            atomic::{AtomicBool, AtomicUsize},
            Arc,
        };

        struct State {
            callbacks: CallbackSnapshot,
            error: Option<String>,
            observed: u64,
            recovered: u64,
            recovering: bool,
        }
        struct MonitoredOutput {
            state: Arc<parking_lot::Mutex<State>>,
            health: HealthMonitor,
            drops: Arc<AtomicUsize>,
        }
        impl Drop for MonitoredOutput {
            fn drop(&mut self) {
                self.drops.fetch_add(1, Ordering::Relaxed);
            }
        }
        impl Output for MonitoredOutput {
            fn formats(&self) -> anyhow::Result<Vec<AudioFormatSpec>> {
                Ok(formats_for_ranges(&[(2, 48000, 48000)]))
            }
            fn begin(&mut self, _: AudioFormat, _: SharedClock, _: Gain) -> anyhow::Result<()> {
                Ok(())
            }
            fn write(&mut self, _: AudioBuffer) -> bool {
                true
            }
            fn clear(&mut self) {}
            fn gain(&mut self, _: Gain) {}
            fn failed(&self) -> bool {
                false
            }
            fn recovering(&self) -> bool {
                self.health.is_recovering()
            }
            fn poll_failure(&mut self) -> Option<(bool, String)> {
                let mut state = self.state.lock();
                let error = state.error.take();
                let decision = self
                    .health
                    .poll(Instant::now(), error.as_deref(), state.callbacks);
                state.observed = self.health.observed_xruns();
                state.recovered = self.health.recovered_xruns();
                state.recovering = self.health.is_recovering();
                match decision {
                    HealthDecision::Failed(detail) => Some((true, detail)),
                    _ => None,
                }
            }
        }
        async fn wait_for(
            state: &Arc<parking_lot::Mutex<State>>,
            predicate: impl Fn(&State) -> bool,
        ) {
            tokio::time::timeout(Duration::from_secs(1), async {
                loop {
                    if predicate(&state.lock()) {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
            })
            .await
            .expect("audio worker did not observe the test input");
        }

        // Exercise both terminal paths after proving that a first XRUN was
        // recovered by the same live worker and same output object.
        for terminal_stall in [false, true] {
            let now = Instant::now();
            let state = Arc::new(parking_lot::Mutex::new(State {
                callbacks: snapshot(1, Some(now)),
                error: None,
                observed: 0,
                recovered: 0,
                recovering: false,
            }));
            let opens = Arc::new(AtomicUsize::new(0));
            let drops = Arc::new(AtomicUsize::new(0));
            let output_state = state.clone();
            let output_opens = opens.clone();
            let output_drops = drops.clone();
            let (status, _status_receiver) = tokio::sync::watch::channel(AudioStatus::default());
            let mut worker = spawn_output_worker(
                Arc::new(parking_lot::Mutex::new(move || {
                    output_opens.fetch_add(1, Ordering::Relaxed);
                    Ok(MonitoredOutput {
                        state: output_state.clone(),
                        health: HealthMonitor::new(Instant::now(), true),
                        drops: output_drops.clone(),
                    })
                })),
                Gain {
                    volume: 50,
                    muted: false,
                    delay: 0,
                },
                Arc::new(AtomicBool::new(false)),
                Arc::new(AtomicU64::new(0)),
                status,
            )
            .unwrap();
            tokio::time::timeout(Duration::from_secs(1), &mut worker.ready)
                .await
                .unwrap()
                .unwrap()
                .unwrap();

            state.lock().error = Some(cpal::ErrorKind::Xrun.to_string());
            wait_for(&state, |state| state.observed == 1 && state.recovering).await;
            assert!(matches!(
                worker.done.try_recv(),
                Err(tokio::sync::oneshot::error::TryRecvError::Empty)
            ));
            assert_eq!(drops.load(Ordering::Relaxed), 0);
            state.lock().callbacks = snapshot(2, Some(Instant::now()));
            wait_for(&state, |state| state.recovered == 1 && !state.recovering).await;
            assert!(matches!(
                worker.done.try_recv(),
                Err(tokio::sync::oneshot::error::TryRecvError::Empty)
            ));
            assert_eq!(opens.load(Ordering::Relaxed), 1);
            assert_eq!(drops.load(Ordering::Relaxed), 0);

            if terminal_stall {
                state.lock().callbacks = snapshot(2, Some(Instant::now() - Duration::from_secs(2)));
            } else {
                state.lock().error = Some(cpal::ErrorKind::Xrun.to_string());
                wait_for(&state, |state| state.observed == 2 && state.recovering).await;
                state.lock().callbacks = snapshot(3, Some(Instant::now()));
                wait_for(&state, |state| state.recovered == 2).await;
                state.lock().error = Some(cpal::ErrorKind::Xrun.to_string());
            }
            let (recoverable, detail) =
                tokio::time::timeout(Duration::from_secs(1), &mut worker.done)
                    .await
                    .unwrap()
                    .unwrap();
            worker.thread.join().unwrap();
            assert!(recoverable, "supervisor must be allowed to recreate output");
            assert_eq!(
                detail.as_deref(),
                Some(if terminal_stall {
                    "Audio output callbacks stalled"
                } else {
                    "Repeated audio output underruns"
                })
            );
            assert_eq!(opens.load(Ordering::Relaxed), 1);
            assert_eq!(drops.load(Ordering::Relaxed), 1);
        }
    }
}

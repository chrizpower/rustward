use std::{
    fmt::Write as _,
    sync::atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering::Relaxed},
};

#[derive(Clone, Copy)]
pub(crate) enum State {
    Idle,
    Paused,
    Review,
    ExecutionCapacity,
    LogCapacity,
    Draining,
    Failed,
}

macro_rules! counters {
    ($($variant:ident => $name:literal),+ $(,)?) => {
        #[derive(Clone, Copy)]
        pub(crate) enum Counter { $($variant),+ }
        const NAMES: &[&str] = &[$($name),+];
    };
}

counters! {
    Manual => "admissions_total{trigger=\"manual\"}",
    Scheduled => "admissions_total{trigger=\"schedule\"}",
    RejectKey => "admission_rejections_total{reason=\"invalid_key\"}",
    RejectInputs => "admission_rejections_total{reason=\"invalid_inputs\"}",
    RejectSize => "admission_rejections_total{reason=\"too_large\"}",
    RejectConflict => "admission_rejections_total{reason=\"payload_conflict\"}",
    RejectQueue => "admission_rejections_total{reason=\"queue_full\"}",
    RejectClosed => "admission_rejections_total{reason=\"closed\"}",
    RejectBusy => "admission_rejections_total{reason=\"busy\"}",
    RejectStorage => "admission_rejections_total{reason=\"storage\"}",
    RejectOther => "admission_rejections_total{reason=\"other\"}",
    Success => "completions_total{result=\"success\"}",
    Failure => "completions_total{result=\"failure\"}",
    Cancellation => "completions_total{result=\"cancellation\"}",
    Interruption => "completions_total{result=\"interruption\"}",
    Timeout => "completions_total{result=\"timeout\"}",
    SkipPaused => "schedule_skips_total{reason=\"paused\"}",
    SkipClosed => "schedule_skips_total{reason=\"admission_closed\"}",
    SkipQueued => "schedule_skips_total{reason=\"schedule_queued\"}",
    SkipFull => "schedule_skips_total{reason=\"queue_full\"}",
    SkipActivation => "schedule_skips_total{reason=\"activation\"}",
    SkipEvaluation => "schedule_skips_total{reason=\"evaluation_gap\"}",
    SkipSize => "schedule_skips_total{reason=\"too_large\"}",
    Truncated => "log_truncations_total",
    StateFailure => "storage_failures_total{component=\"state\"}",
    LogFailure => "storage_failures_total{component=\"task_log\"}",
    Expired => "expired_runs_total",
}

const BOUNDS: [(u64, &str); 8] = [
    (100, "0.1"),
    (500, "0.5"),
    (1000, "1"),
    (5000, "5"),
    (30000, "30"),
    (60000, "60"),
    (300000, "300"),
    (3600000, "3600"),
];
const STATES: [&str; 7] = [
    "idle",
    "paused",
    "review",
    "execution_capacity",
    "log_capacity",
    "draining",
    "failed",
];
const UNKNOWN: u64 = u64::MAX;

pub(super) struct Metrics {
    counters: [AtomicU64; NAMES.len()],
    buckets: [AtomicU64; BOUNDS.len() + 1],
    milliseconds: AtomicU64,
    queue: AtomicU64,
    active: AtomicUsize,
    state: AtomicU32,
}

impl Default for Metrics {
    fn default() -> Self {
        Self {
            counters: Default::default(),
            buckets: Default::default(),
            milliseconds: AtomicU64::new(0),
            queue: AtomicU64::new(UNKNOWN),
            active: AtomicUsize::new(0),
            state: AtomicU32::new(State::Paused as u32),
        }
    }
}

impl Metrics {
    pub(super) fn add(&self, counter: Counter, amount: u64) {
        self.counters[counter as usize].fetch_add(amount, Relaxed);
    }

    pub(super) fn duration(&self, milliseconds: u64) {
        let index = BOUNDS.partition_point(|(limit, _)| *limit < milliseconds);
        self.buckets[index].fetch_add(1, Relaxed);
        self.milliseconds.fetch_add(milliseconds, Relaxed);
    }

    pub(super) fn seed_queue(&self, queue: Option<u32>) {
        self.queue.store(queue.map_or(UNKNOWN, u64::from), Relaxed);
    }

    pub(super) fn queue_change(&self, admitted: bool) {
        let _ = self.queue.try_update(Relaxed, Relaxed, |queue| {
            if queue == UNKNOWN {
                None
            } else if admitted {
                queue.checked_add(1)
            } else {
                Some(queue.saturating_sub(1))
            }
        });
    }

    pub(super) fn state(&self, active: usize, state: State) {
        self.active.store(active, Relaxed);
        self.state.store(state as u32, Relaxed);
    }

    pub(super) fn render(&self, dropped: u64) -> String {
        let mut out = String::with_capacity(4096);
        let current = self.state.load(Relaxed);
        let queue = if current == State::Failed as u32 {
            UNKNOWN
        } else {
            self.queue.load(Relaxed)
        };
        writeln!(out, "# TYPE rustward_queue_depth gauge").unwrap();
        if queue != UNKNOWN {
            writeln!(out, "rustward_queue_depth {queue}").unwrap();
        }
        writeln!(
            out,
            "# TYPE rustward_queue_depth_available gauge\nrustward_queue_depth_available {}",
            u8::from(queue != UNKNOWN)
        )
        .unwrap();
        writeln!(
            out,
            "# TYPE rustward_active_runs gauge\nrustward_active_runs {}",
            self.active.load(Relaxed)
        )
        .unwrap();
        writeln!(out, "# TYPE rustward_dispatch_state gauge").unwrap();
        for (index, state) in STATES.iter().enumerate() {
            writeln!(
                out,
                "rustward_dispatch_state{{state=\"{state}\"}} {}",
                u8::from(current as usize == index)
            )
            .unwrap();
        }
        let mut last = "";
        for (counter, name) in self.counters.iter().zip(NAMES) {
            let family = name.split('{').next().unwrap();
            if family != last {
                writeln!(out, "# TYPE rustward_{family} counter").unwrap();
                last = family;
            }
            writeln!(out, "rustward_{name} {}", counter.load(Relaxed)).unwrap();
        }
        writeln!(out, "# TYPE rustward_service_logs_dropped_total counter\nrustward_service_logs_dropped_total {dropped}").unwrap();
        writeln!(out, "# TYPE rustward_run_duration_seconds histogram").unwrap();
        let mut count = 0;
        for (index, (_, bound)) in BOUNDS.iter().enumerate() {
            count += self.buckets[index].load(Relaxed);
            writeln!(
                out,
                "rustward_run_duration_seconds_bucket{{le=\"{bound}\"}} {count}"
            )
            .unwrap();
        }
        count += self.buckets[BOUNDS.len()].load(Relaxed);
        let ms = self.milliseconds.load(Relaxed);
        writeln!(out, "rustward_run_duration_seconds_bucket{{le=\"+Inf\"}} {count}\nrustward_run_duration_seconds_count {count}\nrustward_run_duration_seconds_sum {}.{:03}", ms / 1000, ms % 1000).unwrap();
        out
    }
}

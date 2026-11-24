mod metrics;
pub(crate) use metrics::{Counter, Probe, State};

use std::{
    io::{self, Write},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering::Relaxed},
        mpsc::{self, SyncSender},
    },
    time::Duration,
};

use serde::Serialize;

use crate::admission::{self, Error};

const LOG_QUEUE: usize = 128;
const LOG_BYTES: usize = 4096;

#[derive(Clone)]
pub(crate) struct Telemetry(Arc<Inner>);

struct Inner {
    metrics: metrics::Metrics,
    dropped: Arc<AtomicU64>,
    output: SyncSender<Message>,
}

enum Message {
    Record(Event),
    Flush(SyncSender<()>),
}

#[derive(Serialize)]
pub(crate) struct Event {
    timestamp_ms: Option<i64>,
    severity: &'static str,
    event: &'static str,
    component: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    run_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    task_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    schedule_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    count: Option<u64>,
}

impl Event {
    pub(crate) fn new(event: &'static str, component: &'static str) -> Self {
        Self {
            timestamp_ms: admission::utc_ms().ok(),
            severity: "info",
            event,
            component,
            run_id: None,
            task_id: None,
            schedule_id: None,
            reason: None,
            count: None,
        }
    }

    pub(crate) fn error(mut self) -> Self {
        self.severity = "error";
        self
    }

    pub(crate) fn run(mut self, id: &str) -> Self {
        self.run_id = Some(identifier(id));
        self
    }

    pub(crate) fn task(mut self, id: &str) -> Self {
        self.task_id = Some(identifier(id));
        self
    }

    pub(crate) fn schedule(mut self, id: &str) -> Self {
        self.schedule_id = Some(identifier(id));
        self
    }

    pub(crate) fn reason(mut self, value: &'static str) -> Self {
        self.reason = Some(value);
        self
    }

    pub(crate) fn count(mut self, value: u64) -> Self {
        self.count = Some(value);
        self
    }
}

fn identifier(source: &str) -> String {
    source[..source.floor_char_boundary(128)].to_owned()
}

impl Telemetry {
    pub(crate) fn stderr() -> io::Result<Self> {
        let mut stderr = std::fs::File::from(rustix::io::dup(io::stderr())?);
        Self::start(move |line| stderr.write_all(line))
    }

    pub(crate) fn start(
        mut write: impl FnMut(&[u8]) -> io::Result<()> + Send + 'static,
    ) -> io::Result<Self> {
        let (output, receiver) = mpsc::sync_channel(LOG_QUEUE);
        let dropped = Arc::new(AtomicU64::new(0));
        let losses = Arc::clone(&dropped);
        std::thread::Builder::new()
            .name("rustward-service-logs".into())
            .spawn(move || {
                while let Ok(message) = receiver.recv() {
                    match message {
                        Message::Record(event) => {
                            let written = serde_json::to_vec(&event).is_ok_and(|mut line| {
                                line.push(b'\n');
                                line.len() <= LOG_BYTES && write(&line).is_ok()
                            });
                            if !written {
                                losses.fetch_add(1, Relaxed);
                            }
                        }
                        Message::Flush(done) => {
                            let _ = done.try_send(());
                        }
                    }
                }
            })?;
        Ok(Self(Arc::new(Inner {
            metrics: metrics::Metrics::default(),
            dropped,
            output,
        })))
    }

    pub(crate) fn log(&self, event: Event) {
        if self.0.output.try_send(Message::Record(event)).is_err() {
            self.0.dropped.fetch_add(1, Relaxed);
        }
    }

    pub(crate) fn flush(&self) {
        let (send, receive) = mpsc::sync_channel(1);
        if self.0.output.try_send(Message::Flush(send)).is_ok() {
            let _ = receive.recv_timeout(Duration::from_millis(100));
        }
    }

    pub(crate) fn add(&self, counter: Counter, amount: u64) {
        self.0.metrics.add(counter, amount);
    }

    pub(crate) fn seed_queue(&self, queue: Option<u32>) {
        self.0.metrics.seed_queue(queue);
    }

    pub(crate) fn queue_change(&self, admitted: bool) {
        self.0.metrics.queue_change(admitted);
    }

    pub(crate) fn state(&self, active: usize, state: State) {
        self.0.metrics.state(active, state);
    }

    pub(crate) fn completed(
        &self,
        run: &str,
        task: &str,
        result: &str,
        duration_ms: Option<u64>,
        truncated: bool,
    ) {
        let (counter, reason) = match result {
            "success" => (Counter::Success, "success"),
            "failure" => (Counter::Failure, "failure"),
            "timeout" => (Counter::Timeout, "timeout"),
            "interruption" => (Counter::Interruption, "interruption"),
            "cancellation" => (Counter::Cancellation, "cancellation"),
            _ => return,
        };
        self.add(counter, 1);
        if let Some(duration) = duration_ms {
            self.0.metrics.duration(duration);
        }
        if truncated {
            self.add(Counter::Truncated, 1);
        }
        let event = Event::new("run_completed", "execution")
            .run(run)
            .reason(reason);
        self.log(if task.is_empty() {
            event
        } else {
            event.task(task)
        });
    }

    pub(crate) fn rejected(&self, error: &Error) {
        let (counter, reason) = rejection_details(error);
        self.add(counter, 1);
        self.log(Event::new("admission_rejected", "admission").reason(reason));
    }

    pub(crate) fn occurrence(&self, schedule: &str, task: &str, decision: &str, run: Option<&str>) {
        let (counter, reason) = match decision {
            "admitted" => (Counter::Scheduled, "schedule"),
            "paused" => (Counter::SkipPaused, "paused"),
            "admission_closed" => (Counter::SkipClosed, "admission_closed"),
            "schedule_queued" => (Counter::SkipQueued, "schedule_queued"),
            "queue_full" => (Counter::SkipFull, "queue_full"),
            "too_large" => (Counter::SkipSize, "too_large"),
            _ => return,
        };
        self.add(counter, 1);
        if run.is_some() {
            self.queue_change(true);
        }
        let event = Event::new(
            if run.is_some() {
                "run_admitted"
            } else {
                "occurrence_skipped"
            },
            "scheduler",
        )
        .schedule(schedule)
        .task(task)
        .reason(reason);
        self.log(if let Some(run) = run {
            event.run(run)
        } else {
            event
        });
    }

    pub(crate) fn gap(&self, schedule: &str, reason: &'static str, missed: Option<u32>) {
        let mut event = Event::new("schedule_gap", "scheduler")
            .schedule(schedule)
            .reason(reason);
        if let Some(missed) = missed {
            let counter = if reason == "activation" {
                Counter::SkipActivation
            } else {
                Counter::SkipEvaluation
            };
            self.add(counter, u64::from(missed));
            event = event.count(u64::from(missed));
        }
        if reason != "first_installation" {
            if missed.is_none() {
                self.add(Counter::UnknownGap, 1);
            }
            if let Some(timestamp) = event.timestamp_ms.and_then(|ms| u64::try_from(ms).ok()) {
                self.0.metrics.gap_at(timestamp);
            }
        }
        self.log(event);
    }

    pub(crate) fn probe(&self) -> Probe {
        self.0.metrics.probe()
    }

    pub(crate) fn scrape(&self) -> String {
        self.0.metrics.render(self.0.dropped.load(Relaxed))
    }
}

pub(crate) fn rejection(error: &Error) -> &'static str {
    rejection_details(error).1
}

fn rejection_details(error: &Error) -> (Counter, &'static str) {
    match error {
        Error::InvalidKey => (Counter::RejectKey, "invalid_key"),
        Error::InvalidInputs => (Counter::RejectInputs, "invalid_inputs"),
        Error::TooLarge => (Counter::RejectSize, "too_large"),
        Error::PayloadConflict => (Counter::RejectConflict, "payload_conflict"),
        Error::QueueFull => (Counter::RejectQueue, "queue_full"),
        Error::Closed => (Counter::RejectClosed, "closed"),
        Error::Busy => (Counter::RejectBusy, "busy"),
        Error::Storage => (Counter::RejectStorage, "storage"),
        _ => (Counter::RejectOther, "other"),
    }
}

#[cfg(test)]
mod tests;

use super::{Request, Response, Service, Supervisor, controller_error, state_error};
use crate::{
    admission::{self, Admission, Cancellation},
    supervision::RunId,
    telemetry::Event,
};
use serde_json::{Value, json};

impl<S: Supervisor + Clone + Send + 'static> Service<S> {
    pub(super) fn request(&mut self, request: Request) -> Response {
        match self.handle(request) {
            Ok(value) => Response::Ok(value),
            Err(error) => Response::Error(state_error(error)),
        }
    }

    pub(super) fn submit(
        &mut self,
        key: &str,
        task_id: &str,
        inputs: &std::collections::BTreeMap<String, toml::Value>,
    ) -> Result<Admission, admission::Error> {
        self.store.admit_with_gate(
            key,
            &self.config,
            task_id,
            inputs,
            &self.runtime_id,
            self.shutdown,
        )
    }

    pub(super) fn handle(&mut self, request: Request) -> Result<Value, admission::Error> {
        use admission::Error;
        let paused = matches!(&request, Request::SchedulePause { .. });
        match request {
            Request::IssueKey => {
                if self.shutdown || self.safety.failed() {
                    return Err(Error::Closed);
                }
                Ok(json!({"key": self.store.issue_key()?}))
            }
            Request::Submit {
                key,
                task_id,
                inputs,
            } => Ok(submission_response(self.submit(&key, &task_id, &inputs)?)),
            Request::Status { run_id } => {
                RunId::new(&run_id).map_err(|_| Error::InvalidInputs)?;
                Ok(json!(self.store.run_state(&run_id)?))
            }
            Request::Cancel { run_id } => {
                RunId::new(&run_id).map_err(|_| Error::InvalidInputs)?;
                let result = self.store.cancel_queued(&run_id)?;
                let status = match result {
                    Cancellation::Cancelled => "Cancelled".into(),
                    Cancellation::AlreadyCancelled => "AlreadyCancelled".into(),
                    Cancellation::NotFound => "NotFound".into(),
                    Cancellation::NotQueued => format!(
                        "{:?}",
                        self.controller
                            .cancel_active(&run_id)
                            .map_err(controller_error)?
                    ),
                };
                Ok(json!({"status": status}))
            }
            Request::Pause | Request::Resume => {
                let paused = matches!(request, Request::Pause);
                if !paused && (self.shutdown || self.safety.failed()) {
                    return Err(Error::Closed);
                }
                self.controller
                    .set_paused(paused)
                    .map_err(controller_error)?;
                self.paused = paused;
                self.block = None;
                self.event(
                    Event::new("dispatch_control_changed", "control").reason(if paused {
                        "paused"
                    } else {
                        "resumed"
                    }),
                );
                Ok(json!(null))
            }
            Request::ServiceStatus => Ok(json!({
                "version": crate::VERSION,
                "paused": self.paused, "draining": self.shutdown, "failed": self.safety.failed(),
                "active_runs": self.controller.active_runs(),
                "dispatch_block": self.block.as_ref().map(|block| format!("{block:?}")),
            })),
            Request::Reviews { after } => {
                if let Some(after) = &after {
                    RunId::new(after).map_err(|_| Error::InvalidInputs)?;
                }
                Ok(json!(self.store.pending_reviews(after.as_deref(), 10)?))
            }
            Request::Acknowledge {
                acknowledgement_id,
                review,
                note,
            } => Ok(json!(self.store.acknowledge(
                &acknowledgement_id,
                &review,
                &note
            )?)),
            Request::Reload => self.reload(),
            Request::SchedulePause { schedule_id } | Request::ScheduleResume { schedule_id } => {
                if self.shutdown || self.safety.failed() {
                    return Err(Error::Closed);
                }
                self.store.pause_schedule(schedule_id.as_deref(), paused)?;
                let event = Event::new("schedule_control_changed", "control").reason(if paused {
                    "paused"
                } else {
                    "resumed"
                });
                self.event(if let Some(id) = &schedule_id {
                    event.schedule(id)
                } else {
                    event
                });
                Ok(json!(null))
            }
            Request::ScheduleHistory {
                schedule_id,
                after_due_ms,
                after_gap_id,
            } => self
                .store
                .schedule_history(&schedule_id, after_due_ms, after_gap_id),
            Request::Shutdown => {
                self.shutdown = true;
                Ok(json!(null))
            }
        }
    }
}

pub(super) fn submission_response(result: Admission) -> Value {
    let (status, run_id) = match result {
        Admission::Admitted { run_id } => ("admitted", run_id),
        Admission::AlreadyAdmitted { run_id } => ("already_admitted", run_id),
        Admission::HistoryRemoved { run_id } => ("history_removed", run_id),
    };
    json!({"status": status, "run_id": run_id})
}

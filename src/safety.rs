use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

/// Shared, irreversible process-local storage failure gate. All admission and
/// dispatch connections in one service must use the same gate.
#[derive(Clone, Default)]
pub struct Safety {
    failed: Arc<AtomicBool>,
    #[cfg(target_os = "linux")]
    telemetry: Option<crate::telemetry::Telemetry>,
}

impl Safety {
    #[cfg(target_os = "linux")]
    pub(crate) fn observed(telemetry: crate::telemetry::Telemetry) -> Self {
        Self {
            telemetry: Some(telemetry),
            ..Self::default()
        }
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn telemetry(&self) -> Option<&crate::telemetry::Telemetry> {
        self.telemetry.as_ref()
    }

    pub(crate) fn state_failed(&self) {
        #[cfg(target_os = "linux")]
        if let Some(telemetry) = self.telemetry() {
            telemetry.add(crate::telemetry::Counter::StateFailure, 1);
            telemetry.log(crate::telemetry::Event::new("storage_failure", "state").error());
        }
        self.fail();
    }

    pub fn fail(&self) {
        self.failed.store(true, Ordering::SeqCst);
    }

    pub fn failed(&self) -> bool {
        self.failed.load(Ordering::SeqCst)
    }
}

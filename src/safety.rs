use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

/// Shared, irreversible process-local storage failure gate. All admission and
/// dispatch connections in one service must use the same gate.
#[derive(Clone, Default)]
pub struct Safety(Arc<AtomicBool>);

impl Safety {
    pub fn fail(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn failed(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

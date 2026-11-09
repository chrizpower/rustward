use super::Report;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamEvidence {
    pub retained_bytes: u64,
    pub observed_bytes: Option<u64>,
    pub truncated: Option<bool>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Evidence {
    pub stdout: StreamEvidence,
    pub stderr: StreamEvidence,
    pub synchronized: bool,
    pub recovered: bool,
}
impl Evidence {
    pub fn from_report(report: Report, synchronized: bool, recovered: bool) -> Self {
        let stream = |report: super::StreamReport| StreamEvidence {
            retained_bytes: report.retained_bytes,
            observed_bytes: (!recovered).then_some(report.observed_bytes),
            truncated: (!recovered).then_some(report.truncated),
        };
        Self {
            stdout: stream(report.stdout),
            stderr: stream(report.stderr),
            synchronized,
            recovered,
        }
    }
    pub(crate) fn valid(&self) -> bool {
        [&self.stdout,&self.stderr].iter().all(|stream| {
            if self.recovered {stream.observed_bytes.is_none() && stream.truncated.is_none()}
            else {matches!((stream.observed_bytes,stream.truncated),(Some(observed),Some(truncated)) if observed>=stream.retained_bytes && (!truncated || observed>stream.retained_bytes))}
        })
    }
    pub fn retained_bytes(&self) -> Option<u64> {
        self.stdout
            .retained_bytes
            .checked_add(self.stderr.retained_bytes)
    }
}

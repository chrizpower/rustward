use hmac::{KeyInit, Mac};
use rusqlite::{OptionalExtension, TransactionBehavior, params};
use std::collections::BTreeMap;
use toml::Value;

use super::{
    Admission, Config, Error, KeyMac, Store, decode_key, fingerprint, insert_run, storage,
};

impl Store {
    pub(crate) fn admit_with_gate(
        &mut self,
        key: &str,
        config: &Config,
        task_id: &str,
        submitted_values: &BTreeMap<String, Value>,
        runtime_id: &str,
        closed_for_request: bool,
    ) -> Result<Admission, Error> {
        let safety = self.safety.clone();
        let decision = self.perform(|connection| {
            let key_bytes = decode_key(key)?;
            let fingerprint = fingerprint(task_id, submitted_values)?;
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(storage)?;
            let existing: Option<(Vec<u8>, String, bool)> = transaction.query_row(
                "SELECT a.fingerprint, a.run_id, EXISTS(SELECT 1 FROM runs r WHERE r.run_id=a.run_id)
                 FROM admissions a WHERE request_key=?1", [key],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            ).optional().map_err(storage)?;
            if let Some((original, run_id, history_exists)) = existing {
                if original != fingerprint {
                    return Err(Error::PayloadConflict);
                }
                return Ok(if history_exists {
                    Admission::AlreadyAdmitted { run_id }
                } else {
                    Admission::HistoryRemoved { run_id }
                });
            }
            let (generation, secret, limit, closed): (Vec<u8>, Vec<u8>, u32, bool) = transaction.query_row(
                "SELECT generation, secret, queue_limit, closed FROM issuer WHERE id=1", [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            ).map_err(storage)?;
            if generation.len() != 32 || secret.len() != 32 {
                return Err(Error::Storage);
            }
            if key_bytes[1..33] != generation {
                return Err(Error::InvalidKey);
            }
            let mut mac = KeyMac::new_from_slice(&secret).unwrap();
            mac.update(&key_bytes[..65]);
            mac.verify_slice(&key_bytes[65..]).map_err(|_| Error::InvalidKey)?;
            if closed || closed_for_request || safety.failed() {
                return Err(Error::Closed);
            }
            let run_id = insert_run(&transaction, config, task_id, submitted_values, runtime_id, limit)?;
            transaction.execute("INSERT INTO admissions VALUES (?1, ?2, ?3)", params![key, fingerprint, run_id]).map_err(storage)?;
            transaction.commit().map_err(storage)?;
            Ok(Admission::Admitted { run_id })
        });
        #[cfg(target_os = "linux")]
        if let Some(telemetry) = safety.telemetry() {
            use crate::telemetry::{Counter, Event};
            match &decision {
                Ok(Admission::Admitted { run_id }) => {
                    telemetry.add(Counter::Manual, 1);
                    telemetry.queue_change(true);
                    telemetry.log(
                        Event::new("run_admitted", "admission")
                            .run(run_id)
                            .task(task_id)
                            .reason("manual"),
                    );
                }
                Err(error) => telemetry.rejected(error),
                _ => {}
            }
        }
        decision
    }
}

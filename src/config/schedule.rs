use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use toml::Value;

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Schedule {
    #[serde(skip_deserializing)]
    pub(super) id: String,
    pub(super) task: String,
    pub(super) cron: String,
    #[serde(default)]
    pub(super) inputs: BTreeMap<String, Value>,
}

impl Schedule {
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn task(&self) -> &str {
        &self.task
    }
    pub fn cron(&self) -> &str {
        &self.cron
    }
    pub fn inputs(&self) -> &BTreeMap<String, Value> {
        &self.inputs
    }

    pub(super) fn validate_cron(&mut self) -> Result<(), String> {
        let fields: Vec<_> = self.cron.split_whitespace().collect();
        let bounds = [(0, 59), (0, 23), (1, 31), (1, 12), (0, 6)];
        if fields.len() != 5
            || fields
                .iter()
                .zip(bounds)
                .any(|(field, (min, max))| !field.split(',').all(|part| valid_part(part, min, max)))
        {
            return Err(format!("schedule {}: invalid five-field UTC cron", self.id));
        }
        self.cron = fields.join(" ");
        Ok(())
    }
}

fn number(text: &str) -> Option<u32> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

fn valid_part(part: &str, min: u32, max: u32) -> bool {
    let (range, step) = match part.split_once('/') {
        Some((range, step)) => (range, number(step)),
        None => (part, Some(1)),
    };
    if !step.is_some_and(|n| n > 0) {
        return false;
    }
    if range == "*" {
        return true;
    }
    let (start, end) = match range.split_once('-') {
        Some((start, end)) => (number(start), number(end)),
        None => (number(range), number(range)),
    };
    matches!((start, end), (Some(start), Some(end)) if min <= start && start <= end && end <= max)
}

use std::collections::BTreeMap;

use croner::{
    Cron,
    parser::{CronParser, Seconds, Year},
};
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

    #[cfg(any(target_os = "linux", test))]
    pub(crate) fn matcher(&self) -> Cron {
        parse_cron(&self.cron).expect("schedule was validated")
    }

    pub(super) fn validate_cron(&mut self) -> Result<(), String> {
        parse_cron(&self.cron).map_err(|reason| format!("schedule {}: {reason}", self.id))?;
        self.cron = self.cron.split_whitespace().collect::<Vec<_>>().join(" ");
        Ok(())
    }
}

fn parse_cron(source: &str) -> Result<Cron, &'static str> {
    let fields: Vec<_> = source.split_whitespace().collect();
    if fields.len() != 5 {
        return Err("cron must have five UTC fields");
    }
    if fields.iter().any(|field| {
        *field != "*"
            && (!field
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b',' | b'-'))
                || field.split(',').any(str::is_empty))
    }) {
        return Err(
            "cron permits only *, numbers, lists, and ascending ranges; steps are not supported",
        );
    }
    if fields[2] != "*" && fields[4] != "*" {
        return Err("cron day of month or day of week must be exactly *");
    }
    // Croner also accepts 7 for Sunday; Rustward permits only 0 through 6.
    if fields[4] != "*"
        && fields[4]
            .split([',', '-'])
            .any(|value| !value.parse::<u16>().is_ok_and(|number| number <= 6))
    {
        return Err("cron day of week must use 0 through 6; Sunday is 0");
    }
    CronParser::builder()
        .seconds(Seconds::Disallowed)
        .year(Year::Disallowed)
        .build()
        .parse(source)
        .map_err(|_| "invalid five-field UTC cron")
}

#[cfg(test)]
mod tests {
    use super::parse_cron;
    use chrono::{TimeZone, Utc};

    #[test]
    fn matches_utc_lists_ranges_and_day_fields() {
        let time = Utc.with_ymd_and_hms(2026, 11, 9, 2, 15, 0).unwrap();
        for (source, matches) in [
            ("0,15,30,45 2 * * *", true),
            ("0,15,30,45 2 * * 1-5", true),
            ("15 2 9 11 *", true),
            ("15 2 1 11 *", false),
            ("15 2 * * 0", false),
            ("15 2 * 10 *", false),
            ("15 3 * * *", false),
            ("14 2 * * *", false),
            ("0-10,15 2 * * *", true),
        ] {
            assert_eq!(
                parse_cron(source).unwrap().is_time_matching(&time).unwrap(),
                matches,
                "{source}"
            );
        }
        let sunday = Utc.with_ymd_and_hms(2026, 11, 8, 2, 15, 0).unwrap();
        assert!(
            parse_cron("15 2 * * 0")
                .unwrap()
                .is_time_matching(&sunday)
                .unwrap()
        );
        assert!(
            !parse_cron("15 2 * * 1-5")
                .unwrap()
                .is_time_matching(&sunday)
                .unwrap()
        );
    }

    #[test]
    fn evaluates_month_year_and_leap_day_boundaries() {
        for (source, start, expected) in [
            ("0 2 1 * *", (2026, 12, 31, 23, 59), (2027, 1, 1, 2, 0)),
            ("0 2 31 * *", (2026, 4, 1, 0, 0), (2026, 5, 31, 2, 0)),
            ("0 2 29 2 *", (2027, 1, 1, 0, 0), (2028, 2, 29, 2, 0)),
            (
                "0,15,30,45 * * * *",
                (2026, 12, 31, 23, 45),
                (2027, 1, 1, 0, 0),
            ),
        ] {
            let start = Utc
                .with_ymd_and_hms(start.0, start.1, start.2, start.3, start.4, 0)
                .unwrap();
            let expected = Utc
                .with_ymd_and_hms(
                    expected.0, expected.1, expected.2, expected.3, expected.4, 0,
                )
                .unwrap();
            assert_eq!(
                parse_cron(source)
                    .unwrap()
                    .find_next_occurrence(&start, false)
                    .unwrap(),
                expected,
                "{source}"
            );
        }
    }
}

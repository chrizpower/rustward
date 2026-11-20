use std::time::Instant;

pub(super) struct Clock {
    previous: Option<(i64, Instant)>,
    evaluated: i64,
    changed: bool,
}

impl Clock {
    pub(super) fn new(evaluated: i64) -> Self {
        Self {
            previous: None,
            evaluated,
            changed: false,
        }
    }

    pub(super) fn sample(&mut self, wall: i64, monotonic: Instant) -> Option<(i64, bool)> {
        if let Some((previous_wall, previous_monotonic)) = self.previous.replace((wall, monotonic))
        {
            let elapsed = monotonic.duration_since(previous_monotonic).as_millis() as i128;
            let difference = i128::from(wall - previous_wall) - elapsed;
            self.changed |= wall < previous_wall || difference.abs() > 2_000;
        }
        let minute = minute(wall);
        (minute != self.evaluated || self.changed).then_some((minute, self.changed))
    }

    pub(super) fn evaluated(&mut self, minute: i64) {
        self.evaluated = minute;
        self.changed = false;
    }
}

pub(super) fn minute(wall: i64) -> i64 {
    wall - wall.rem_euclid(60_000)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn uses_monotonic_time_to_detect_wall_clock_changes() {
        let now = Instant::now();
        let mut clock = Clock::new(0);
        assert_eq!(clock.sample(59_000, now), None);
        assert_eq!(
            clock.sample(60_000, now + Duration::from_secs(1)),
            Some((60_000, false))
        );
        clock.evaluated(60_000);
        assert_eq!(
            clock.sample(180_000, now + Duration::from_secs(2)),
            Some((180_000, true))
        );
        assert_eq!(
            clock.sample(180_001, now + Duration::from_millis(2001)),
            Some((180_000, true))
        );
        clock.evaluated(180_000);
        assert_eq!(
            clock.sample(120_000, now + Duration::from_secs(3)),
            Some((120_000, true))
        );
    }
}

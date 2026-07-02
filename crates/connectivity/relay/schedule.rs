use crate::common::error::{config_error, NetError};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RelaySchedule {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub windows: Vec<RelayWindow>,
}

impl RelaySchedule {
    pub fn validate(&self) -> Result<(), NetError> {
        for window in &self.windows {
            window.validate()?;
        }
        Ok(())
    }

    pub fn is_open_now_utc(&self) -> bool {
        if !self.enabled {
            return true;
        }
        let Ok(elapsed) = SystemTime::now().duration_since(UNIX_EPOCH) else {
            return false;
        };
        let days_since_epoch = elapsed.as_secs() / 86_400;
        let minute_of_day = ((elapsed.as_secs() % 86_400) / 60) as u16;
        let day = ((days_since_epoch + 4) % 7) as u8; // 0=sun, 1=mon, ..., 6=sat.
        self.is_open_at_utc(day, minute_of_day)
    }

    /// Deterministic UTC schedule check. `day` uses 0=sun, 1=mon, ..., 6=sat.
    pub fn is_open_at_utc(&self, day: u8, minute_of_day: u16) -> bool {
        if !self.enabled {
            return true;
        }
        if self.windows.is_empty() || day > 6 || minute_of_day >= 1_440 {
            return false;
        }

        let previous_day = if day == 0 { 6 } else { day - 1 };
        self.windows
            .iter()
            .any(|window| window.matches(day, previous_day, minute_of_day))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelayWindow {
    /// UTC days: "sun", "mon", "tue", "wed", "thu", "fri", "sat", or "all".
    pub days: Vec<String>,
    /// UTC start time in HH:MM, e.g. "18:00".
    pub start: String,
    /// UTC end time in HH:MM, e.g. "23:00". End is exclusive.
    pub end: String,
}

impl RelayWindow {
    fn validate(&self) -> Result<(), NetError> {
        if self.days.is_empty() {
            return Err(config_error(
                "relay.schedule.windows entries must include at least one day",
            ));
        }
        for day in &self.days {
            if !is_valid_day(day) {
                return Err(config_error(format!(
                    "relay.schedule.windows contains invalid day `{day}`"
                )));
            }
        }
        if parse_hhmm(&self.start).is_none() {
            return Err(config_error(format!(
                "relay.schedule.windows contains invalid start time `{}`",
                self.start
            )));
        }
        if parse_hhmm(&self.end).is_none() {
            return Err(config_error(format!(
                "relay.schedule.windows contains invalid end time `{}`",
                self.end
            )));
        }
        Ok(())
    }

    fn matches(&self, day: u8, previous_day: u8, minute_of_day: u16) -> bool {
        let Some(start) = parse_hhmm(&self.start) else {
            return false;
        };
        let Some(end) = parse_hhmm(&self.end) else {
            return false;
        };

        if start == end {
            return self.matches_day(day);
        }

        if start < end {
            return self.matches_day(day) && minute_of_day >= start && minute_of_day < end;
        }

        (self.matches_day(day) && minute_of_day >= start)
            || (self.matches_day(previous_day) && minute_of_day < end)
    }

    fn matches_day(&self, day: u8) -> bool {
        self.days
            .iter()
            .any(|raw| match raw.trim().to_ascii_lowercase().as_str() {
                "all" | "*" => true,
                "sun" | "sunday" => day == 0,
                "mon" | "monday" => day == 1,
                "tue" | "tues" | "tuesday" => day == 2,
                "wed" | "wednesday" => day == 3,
                "thu" | "thur" | "thurs" | "thursday" => day == 4,
                "fri" | "friday" => day == 5,
                "sat" | "saturday" => day == 6,
                _ => false,
            })
    }
}

fn is_valid_day(raw: &str) -> bool {
    matches!(
        raw.trim().to_ascii_lowercase().as_str(),
        "all"
            | "*"
            | "sun"
            | "sunday"
            | "mon"
            | "monday"
            | "tue"
            | "tues"
            | "tuesday"
            | "wed"
            | "wednesday"
            | "thu"
            | "thur"
            | "thurs"
            | "thursday"
            | "fri"
            | "friday"
            | "sat"
            | "saturday"
    )
}

fn parse_hhmm(value: &str) -> Option<u16> {
    let mut parts = value.split(':');
    let hour = parts.next()?.parse::<u16>().ok()?;
    let minute = parts.next()?.parse::<u16>().ok()?;
    if parts.next().is_some() || hour >= 24 || minute >= 60 {
        return None;
    }
    Some(hour * 60 + minute)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hhmm() {
        assert_eq!(parse_hhmm("00:00"), Some(0));
        assert_eq!(parse_hhmm("23:59"), Some(1439));
        assert_eq!(parse_hhmm("24:00"), None);
        assert_eq!(parse_hhmm("12:60"), None);
    }
}

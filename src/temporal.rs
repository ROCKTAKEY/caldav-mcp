use crate::error::{Error, Result};
use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, TimeZone, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum TimeInput {
    Date {
        value: NaiveDate,
    },
    Floating {
        value: NaiveDateTime,
    },
    Utc {
        value: DateTime<Utc>,
    },
    Zoned {
        value: NaiveDateTime,
        timezone: String,
    },
}
/// Only constructible after timezone and local-time validation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(try_from = "TimeInput", into = "TimeInput")]
#[schemars(with = "TimeInput")]
pub struct TimeValue(TimeInput);
impl From<TimeValue> for TimeInput {
    fn from(v: TimeValue) -> Self {
        v.0
    }
}
impl TryFrom<TimeInput> for TimeValue {
    type Error = Error;
    fn try_from(v: TimeInput) -> Result<Self> {
        if let TimeInput::Zoned { value, timezone } = &v {
            let zone: chrono_tz::Tz = timezone
                .parse()
                .map_err(|_| Error::input("Unknown timezone"))?;
            if zone.from_local_datetime(value).single().is_none() {
                return Err(Error::input(
                    "Local time is ambiguous or nonexistent; specify UTC",
                ));
            }
        }
        Ok(Self(v))
    }
}
impl TimeValue {
    pub fn is_date(&self) -> bool {
        matches!(self.0, TimeInput::Date { .. })
    }
    pub fn is_floating(&self) -> bool {
        matches!(self.0, TimeInput::Floating { .. })
    }
    pub fn compatible(&self, other: &Self) -> bool {
        self.is_date() == other.is_date() && self.is_floating() == other.is_floating()
    }
    pub fn timezone(&self) -> Option<&str> {
        if let TimeInput::Zoned { timezone, .. } = &self.0 {
            Some(timezone)
        } else {
            None
        }
    }
    pub fn local(&self) -> NaiveDateTime {
        match &self.0 {
            TimeInput::Date { value } => value.and_hms_opt(0, 0, 0).unwrap(),
            TimeInput::Floating { value } | TimeInput::Zoned { value, .. } => *value,
            TimeInput::Utc { value } => value.naive_utc(),
        }
    }
    pub fn instant(&self) -> DateTime<Utc> {
        match &self.0 {
            TimeInput::Zoned { value, timezone } => timezone
                .parse::<chrono_tz::Tz>()
                .unwrap()
                .from_local_datetime(value)
                .single()
                .unwrap()
                .with_timezone(&Utc),
            _ => self.local().and_utc(),
        }
    }
    pub fn equivalent(&self, other: &Self) -> bool {
        self.compatible(other) && self.instant() == other.instant()
    }
    pub fn at_instant(&self, dt: DateTime<Utc>) -> Result<Self> {
        let raw = match &self.0 {
            TimeInput::Date { .. } => TimeInput::Date {
                value: dt.date_naive(),
            },
            TimeInput::Floating { .. } => TimeInput::Floating {
                value: dt.naive_utc(),
            },
            TimeInput::Utc { .. } => TimeInput::Utc { value: dt },
            TimeInput::Zoned { timezone, .. } => TimeInput::Zoned {
                value: dt
                    .with_timezone(&timezone.parse::<chrono_tz::Tz>().unwrap())
                    .naive_local(),
                timezone: timezone.clone(),
            },
        };
        raw.try_into()
    }
    pub fn add_wall(&self, delta: Duration) -> Result<Self> {
        let local = self
            .local()
            .checked_add_signed(delta)
            .ok_or_else(|| Error::input("Date overflow"))?;
        let raw = match &self.0 {
            TimeInput::Date { .. } => TimeInput::Date {
                value: local.date(),
            },
            TimeInput::Floating { .. } => TimeInput::Floating { value: local },
            TimeInput::Utc { .. } => TimeInput::Utc {
                value: local.and_utc(),
            },
            TimeInput::Zoned { timezone, .. } => TimeInput::Zoned {
                value: local,
                timezone: timezone.clone(),
            },
        };
        raw.try_into()
    }
    pub fn ical(&self) -> (String, String) {
        match &self.0 {
            TimeInput::Date { value } => (";VALUE=DATE".into(), value.format("%Y%m%d").to_string()),
            TimeInput::Floating { value } => {
                (String::new(), value.format("%Y%m%dT%H%M%S").to_string())
            }
            TimeInput::Utc { value } => (String::new(), dav_time(*value)),
            TimeInput::Zoned { value, timezone } => (
                format!(";TZID={timezone}"),
                value.format("%Y%m%dT%H%M%S").to_string(),
            ),
        }
    }
    pub fn from_ical(value: &str, tzid: Option<&str>) -> Result<Self> {
        let bad = || Error::new("invalid_calendar", "Invalid calendar date/time");
        let raw = if value.len() == 8 {
            TimeInput::Date {
                value: NaiveDate::parse_from_str(value, "%Y%m%d").map_err(|_| bad())?,
            }
        } else if value.ends_with('Z') {
            TimeInput::Utc {
                value: NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%SZ")
                    .map_err(|_| bad())?
                    .and_utc(),
            }
        } else {
            let local = NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%S").map_err(|_| bad())?;
            match tzid {
                Some(tz) => TimeInput::Zoned {
                    value: local,
                    timezone: tz.into(),
                },
                None => TimeInput::Floating { value: local },
            }
        };
        raw.try_into()
    }
}
pub fn dav_time(value: DateTime<Utc>) -> String {
    value.format("%Y%m%dT%H%M%SZ").to_string()
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(try_from = "PeriodInput")]
pub struct Period {
    pub(crate) start: DateTime<Utc>,
    pub(crate) end: DateTime<Utc>,
}
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct PeriodInput {
    start: DateTime<Utc>,
    end: DateTime<Utc>,
}
impl TryFrom<PeriodInput> for Period {
    type Error = Error;
    fn try_from(p: PeriodInput) -> Result<Self> {
        if p.end <= p.start {
            return Err(Error::input("end must be after start"));
        }
        Ok(Self {
            start: p.start,
            end: p.end,
        })
    }
}
/// RFC 5545 durations do not contain months or years.
pub fn duration(value: &str) -> Result<Duration> {
    let invalid = || Error::input("Expected an RFC 5545 duration");
    let (sign, s) = if let Some(s) = value.strip_prefix('-') {
        (-1, s)
    } else {
        (1, value.strip_prefix('+').unwrap_or(value))
    };
    let s = s.strip_prefix('P').ok_or_else(invalid)?;
    let mut time = false;
    let mut digits = String::new();
    let mut seconds = 0i64;
    let mut last = 0;
    let mut count = 0;
    for c in s.chars() {
        if c == 'T' && !time && digits.is_empty() {
            time = true;
            continue;
        }
        if c.is_ascii_digit() {
            digits.push(c);
            continue;
        }
        let (order, mult) = match (time, c) {
            (false, 'W') => (1, 604800),
            (false, 'D') => (2, 86400),
            (true, 'H') => (3, 3600),
            (true, 'M') => (4, 60),
            (true, 'S') => (5, 1),
            _ => return Err(invalid()),
        };
        if order <= last || last == 1 {
            return Err(invalid());
        }
        let n: i64 = digits.parse().map_err(|_| invalid())?;
        seconds = seconds
            .checked_add(n.checked_mul(mult).ok_or_else(invalid)?)
            .ok_or_else(invalid)?;
        digits.clear();
        last = order;
        count += 1;
    }
    if !digits.is_empty() || count == 0 || s.ends_with('T') {
        return Err(invalid());
    }
    Duration::try_seconds(seconds * sign).ok_or_else(invalid)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_ambiguous_and_nonexistent_local_times() {
        for value in ["2026-03-08T02:30:00", "2026-11-01T01:30:00"] {
            assert!(
                serde_json::from_value::<TimeValue>(
                    serde_json::json!({"kind":"zoned","value":value,"timezone":"America/New_York"})
                )
                .is_err()
            );
        }
    }
    #[test]
    fn rejects_backwards_periods() {
        assert!(
            serde_json::from_str::<Period>(
                r#"{"start":"2026-11-02T11:00:00Z","end":"2026-11-02T10:00:00Z"}"#
            )
            .is_err()
        );
    }
}

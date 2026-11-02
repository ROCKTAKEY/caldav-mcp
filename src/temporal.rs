use crate::error::{Error, Result};
use chrono::{DateTime, NaiveDate, NaiveDateTime, TimeZone, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TimeValue {
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
impl TimeValue {
    pub fn checked(self) -> Result<Self> {
        if let Self::Zoned { value, timezone } = &self {
            let zone: chrono_tz::Tz = timezone
                .parse()
                .map_err(|_| Error::input("Unknown timezone"))?;
            if zone.from_local_datetime(value).single().is_none() {
                return Err(Error::input(
                    "Local time is ambiguous or nonexistent; specify UTC",
                ));
            }
        }
        Ok(self)
    }
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(try_from = "PeriodInput")]
pub struct Period {
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
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
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_ambiguous_and_nonexistent_local_times() {
        for value in ["2026-03-08T02:30:00", "2026-11-01T01:30:00"] {
            let time: TimeValue = serde_json::from_value(
                serde_json::json!({"kind":"zoned","value":value,"timezone":"America/New_York"}),
            )
            .unwrap();
            assert!(time.checked().is_err());
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

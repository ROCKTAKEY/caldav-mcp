use crate::{
    calendars::{CalendarRef, Calendars},
    dav::envelope,
    error::{Error, Result},
    items::document::Component,
    temporal::{Period, TimeValue, dav_time, duration},
};
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
pub type Interval = (DateTime<Utc>, DateTime<Utc>);
pub fn free_intervals(period: &Period, mut busy: Vec<Interval>) -> Vec<Value> {
    busy.sort();
    let mut cursor = period.start;
    let mut free = vec![];
    for (a, b) in busy {
        let a = a.max(cursor);
        let b = b.min(period.end);
        if b <= cursor || a >= period.end {
            continue;
        }
        if a > cursor {
            free.push(json!({"start":cursor,"end":a}));
        }
        cursor = cursor.max(b);
    }
    if cursor < period.end {
        free.push(json!({"start":cursor,"end":period.end}));
    }
    free
}
pub struct AvailabilityResult {
    pub value: Value,
    pub intervals: Vec<Interval>,
}
pub async fn query(
    calendars: Calendars<'_>,
    calendar: &CalendarRef,
    period: &Period,
) -> Result<AvailabilityResult> {
    let url = calendars.check_ref(calendar)?;
    // read-free-busy does not grant metadata or item access (RFC 4791 section 7.10).
    let response = calendars
        .client
        .xml(
            "REPORT",
            url.as_str(),
            envelope(
                "C:free-busy-query",
                &format!(
                    "<C:time-range start=\"{}\" end=\"{}\"/>",
                    dav_time(period.start),
                    dav_time(period.end)
                ),
            ),
            "1",
        )
        .await?;
    let doc = Component::parse(&response.body)?;
    if doc.name != "VCALENDAR" || !doc.children.iter().any(|c| c.name == "VFREEBUSY") {
        return Err(Error::new("invalid_response", "Missing VFREEBUSY"));
    }
    let mut intervals = vec![];
    let mut busy = vec![];
    for c in doc.children.iter().filter(|c| c.name == "VFREEBUSY") {
        for p in c.lines.iter().filter(|p| p.name() == "FREEBUSY") {
            if p.parameter("FBTYPE") == Some("FREE") {
                continue;
            }
            for v in p.value().split(',') {
                let (a, b) = v
                    .split_once('/')
                    .ok_or_else(|| Error::new("invalid_response", "Malformed FREEBUSY period"))?;
                let start = TimeValue::from_ical(a, None)?;
                if start.is_date() || start.is_floating() {
                    return Err(Error::new("invalid_response", "FREEBUSY start must be UTC"));
                }
                let end = if b.starts_with('P') {
                    start
                        .instant()
                        .checked_add_signed(duration(b)?)
                        .ok_or_else(|| Error::new("invalid_response", "FREEBUSY overflow"))?
                } else {
                    let t = TimeValue::from_ical(b, None)?;
                    if t.is_date() || t.is_floating() {
                        return Err(Error::new("invalid_response", "FREEBUSY end must be UTC"));
                    }
                    t.instant()
                };
                if end <= start.instant() {
                    return Err(Error::new(
                        "invalid_response",
                        "FREEBUSY period must have positive duration",
                    ));
                }
                let a = start.instant().max(period.start);
                let b = end.min(period.end);
                if b <= a {
                    continue;
                }
                intervals.push((a, b));
                busy.push(
                    json!({"start":a,"end":b,"type":p.parameter("FBTYPE").unwrap_or("BUSY")}),
                );
            }
        }
    }
    let value = json!({"calendar":calendar,"period":period,"busy":busy,"free":free_intervals(period,intervals.clone()),"complete":true});
    Ok(AvailabilityResult { value, intervals })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn merges_overlaps_and_clips_busy_periods_before_finding_gaps() {
        let p: Period = serde_json::from_value(
            json!({"start":"2026-11-02T09:00:00Z","end":"2026-11-02T17:00:00Z"}),
        )
        .unwrap();
        let dt = |h| Utc.with_ymd_and_hms(2026, 11, 2, h, 0, 0).unwrap();
        use chrono::TimeZone;
        let free = free_intervals(
            &p,
            vec![
                (dt(8), dt(10)),
                (dt(9), dt(11)),
                (dt(12), dt(13)),
                (dt(13), dt(14)),
                (dt(16), dt(18)),
            ],
        );
        assert_eq!(
            free,
            vec![
                json!({"start":dt(11),"end":dt(12)}),
                json!({"start":dt(14),"end":dt(16)})
            ]
        );
    }
}

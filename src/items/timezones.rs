use super::document::{Component, Document};
use crate::error::{Error, Result};
use chrono::{DateTime, Datelike, Duration, TimeZone, Utc};

pub fn definition(name: &str, first: i32, last: i32) -> Result<Component> {
    let zone = jiff::tz::TimeZone::get(name).map_err(|_| Error::input("Unknown timezone"))?;
    let mut result = Component::new("VTIMEZONE");
    result.set("TZID", name)?;
    let first = Utc
        .with_ymd_and_hms(first, 1, 1, 0, 0, 0)
        .single()
        .ok_or_else(|| Error::input("Invalid timezone range"))?;
    let last = Utc
        .with_ymd_and_hms(last, 12, 31, 23, 59, 59)
        .single()
        .ok_or_else(|| Error::input("Invalid timezone range"))?;
    let stamp = jiff::Timestamp::from_second(first.timestamp())
        .map_err(|_| Error::input("Timezone range out of bounds"))?;
    let info = zone.to_offset_info(stamp);
    let mut previous = info.offset().seconds();
    result.children.push(observance(
        first,
        previous,
        previous,
        info.dst().is_dst(),
        info.abbreviation(),
    )?);
    // Use the timezone database's exact transitions, including historical second offsets.
    for transition in zone.following(stamp) {
        let t = DateTime::from_timestamp(transition.timestamp().as_second(), 0)
            .ok_or_else(|| Error::input("Invalid timezone transition"))?;
        if t > last {
            break;
        }
        let next = transition.offset().seconds();
        result.children.push(observance(
            t,
            previous,
            next,
            transition.dst().is_dst(),
            transition.abbreviation(),
        )?);
        previous = next;
    }
    Ok(result)
}
fn observance(t: DateTime<Utc>, from: i32, to: i32, dst: bool, name: &str) -> Result<Component> {
    fn offset(v: i32) -> String {
        let sign = if v < 0 { '-' } else { '+' };
        let v = v.unsigned_abs();
        if v.is_multiple_of(60) {
            format!("{sign}{:02}{:02}", v / 3600, v / 60 % 60)
        } else {
            format!("{sign}{:02}{:02}{:02}", v / 3600, v / 60 % 60, v % 60)
        }
    }
    let mut c = Component::new(if dst { "DAYLIGHT" } else { "STANDARD" });
    c.set(
        "DTSTART",
        &(t.naive_utc() + Duration::seconds(from.into()))
            .format("%Y%m%dT%H%M%S")
            .to_string(),
    )?;
    c.set("TZOFFSETFROM", &offset(from))?;
    c.set("TZOFFSETTO", &offset(to))?;
    c.set("TZNAME", &super::document::escape(name))?;
    Ok(c)
}
pub fn timezone_document(name: &str) -> Result<String> {
    let mut root = Component::new("VCALENDAR");
    root.set("VERSION", "2.0")?;
    root.set("PRODID", "-//caldav-mcp//EN")?;
    root.children.push(definition(name, 1900, 2100)?);
    Ok(root.to_ical())
}
impl Document {
    pub(super) fn add_timezones(&mut self) -> Result<()> {
        let mut names = std::collections::BTreeSet::new();
        let mut years = vec![];
        for c in self.members() {
            for p in &c.lines {
                if let Some(tz) = p.parameter("TZID") {
                    names.insert(tz.to_owned());
                }
                if matches!(p.name(), "DTSTART" | "DTEND" | "DUE") {
                    years.push(p.time()?.local().year());
                }
            }
        }
        let first = (years.iter().min().copied().unwrap_or(Utc::now().year()) - 1).max(1);
        let last = (years.iter().max().copied().unwrap_or(first) + 100).min(9998);
        for name in names {
            if !self
                .root
                .children
                .iter()
                .any(|c| c.name == "VTIMEZONE" && c.value("TZID") == Some(&name))
            {
                self.root.children.push(definition(&name, first, last)?);
            }
        }
        Ok(())
    }
}

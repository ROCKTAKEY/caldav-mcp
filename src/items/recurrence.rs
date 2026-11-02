use super::{
    document::Document,
    ical::Component,
    models::{Change, ItemPatch, Scope},
};
use crate::{
    error::{Error, Result},
    temporal::TimeValue,
};
use chrono::Duration;
use rrule::{RRule, RRuleSet, Unvalidated};

pub(super) fn rules(c: &Component) -> Result<RRuleSet> {
    let start = c
        .time("DTSTART")?
        .ok_or_else(|| Error::input("Recurrence requires start"))?;
    let tz: rrule::Tz = start
        .timezone()
        .unwrap_or("UTC")
        .parse::<chrono_tz::Tz>()
        .map_err(|_| Error::input("Unsupported recurrence timezone"))?
        .into();
    let dt = start.instant().with_timezone(&tz);
    let mut set = RRuleSet::new(dt).rdate(dt);
    for p in c.lines.iter().filter(|p| p.name() == "RRULE") {
        let raw = p.value();
        if raw.len() > 2000 || raw.contains("COUNT=") && raw.contains("UNTIL=") {
            return Err(Error::input("Invalid RRULE: COUNT and UNTIL are exclusive"));
        }
        let mut seen = std::collections::HashSet::new();
        for part in raw.split(';') {
            let (k, v) = part
                .split_once('=')
                .ok_or_else(|| Error::input("Invalid RRULE"))?;
            if !seen.insert(k)
                || matches!(k, "COUNT" | "INTERVAL") && v.parse::<u32>().map_or(true, |v| v == 0)
            {
                return Err(Error::input("Invalid or duplicate RRULE part"));
            }
        }
        // DATE and floating UNTIL values use the same civil-time axis as DTSTART.
        let normalized = raw
            .split(';')
            .map(|p| {
                if let Some(v) = p.strip_prefix("UNTIL=") {
                    if start.is_date() && v.len() == 8 {
                        return format!("UNTIL={v}T000000Z");
                    }
                    if start.is_floating() && !v.ends_with('Z') {
                        return format!("UNTIL={v}Z");
                    }
                }
                p.to_owned()
            })
            .collect::<Vec<_>>()
            .join(";");
        let rule: RRule<Unvalidated> = normalized
            .parse()
            .map_err(|_| Error::input("Invalid RRULE"))?;
        set = set.rrule(
            rule.validate(dt)
                .map_err(|_| Error::input("Invalid RRULE for start"))?,
        );
    }
    for p in c
        .lines
        .iter()
        .filter(|p| matches!(p.name(), "RDATE" | "EXDATE"))
    {
        for value in p.value().split(',') {
            let t = TimeValue::from_ical(value, p.parameter("TZID"))?;
            if !start.compatible(&t) {
                return Err(Error::input("Recurrence dates must match start type"));
            }
            let d = t.instant().with_timezone(&tz);
            set = if p.name() == "RDATE" {
                set.rdate(d)
            } else {
                set.exdate(d)
            };
        }
    }
    Ok(set)
}
impl Document {
    pub(super) fn occurrence_index(&mut self, target: &TimeValue) -> Result<usize> {
        for (i, c) in self.root.children.iter().enumerate() {
            if let Some(t) = c.time("RECURRENCE-ID")?
                && t.equivalent(target)
            {
                if c.get("RECURRENCE-ID").unwrap().parameter("RANGE").is_some() {
                    return Err(Error::new(
                        "unsupported",
                        "Cannot edit a THISANDFUTURE anchor as one occurrence",
                    ));
                }
                return Ok(i);
            }
        }
        let master = self
            .members()
            .find(|c| c.get("RECURRENCE-ID").is_none())
            .ok_or_else(|| Error::new("not_found", "No recurring master"))?;
        if master.get("RRULE").is_none() && master.get("RDATE").is_none() {
            return Err(Error::new("not_found", "Item is not recurring"));
        }
        // A range exception changes the derived instance. Refuse to derive from the unmodified master.
        if self.members().any(|c| {
            c.get("RECURRENCE-ID")
                .is_some_and(|p| p.parameter("RANGE").is_some())
        }) {
            return Err(Error::new(
                "unsupported",
                "Local occurrence editing with range exceptions is not supported",
            ));
        }
        let start = master
            .time("DTSTART")?
            .ok_or_else(|| Error::new("invalid_calendar", "Missing recurring start"))?;
        if !start.compatible(target) {
            return Err(Error::input("recurrence_id must match start type"));
        }
        let set = rules(master)?;
        let tz = set.get_dt_start().timezone();
        let result = set
            .after((target.instant() - Duration::seconds(1)).with_timezone(&tz))
            .before((target.instant() + Duration::seconds(1)).with_timezone(&tz))
            .all(3);
        if !result
            .dates
            .iter()
            .any(|d| d.timestamp() == target.instant().timestamp())
        {
            return Err(Error::new(
                if result.limited {
                    "recurrence_limit"
                } else {
                    "not_found"
                },
                "Cannot locate recurrence_id within expansion limits",
            ));
        }
        let mut c = master.clone();
        let next_start = start.at_instant(target.instant())?;
        c.set_time("DTSTART", &next_start)?;
        for n in ["DTEND", "DUE"] {
            if let Some(end) = master.time(n)? {
                // DTEND/DUE imply exact duration, even when the master crosses DST.
                // https://www.rfc-editor.org/rfc/rfc5545.html#section-3.8.5.3
                let finish = next_start
                    .instant()
                    .checked_add_signed(end.instant() - start.instant())
                    .ok_or_else(|| Error::input("Recurrence end overflows supported dates"))?;
                c.set_time(n, &end.at_instant(finish)?)?;
            }
        }
        c.set_time("RECURRENCE-ID", &next_start)?;
        for n in ["RRULE", "RDATE", "EXDATE"] {
            c.remove(n);
        }
        self.root.children.push(c);
        Ok(self.root.children.len() - 1)
    }
    pub fn exclude(&self, target: &TimeValue) -> Result<Self> {
        let mut doc = self.clone();
        let index = doc.occurrence_index(target)?;
        doc.root.children.remove(index);
        let master = doc
            .root
            .children
            .iter_mut()
            .find(|c| {
                matches!(c.name.as_str(), "VEVENT" | "VTODO" | "VJOURNAL")
                    && c.get("RECURRENCE-ID").is_none()
            })
            .ok_or_else(|| Error::new("invalid_calendar", "Missing master"))?;
        master.push_time("EXDATE", target)?;
        doc.edit(&ItemPatch::default(), &Scope::Series)
    }
    pub(super) fn edit_occurrence(&mut self, p: &ItemPatch, target: &TimeValue) -> Result<()> {
        if !matches!(p.recurrence_rule, Change::Keep)
            || !matches!(p.recurrence_dates, Change::Keep)
            || !matches!(p.exception_dates, Change::Keep)
        {
            return Err(Error::input("Edit recurrence rules with series scope"));
        }
        let i = self.occurrence_index(target)?;
        super::document::apply(&mut self.root.children[i], p)
    }
}

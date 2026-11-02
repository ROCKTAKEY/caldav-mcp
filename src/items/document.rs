use super::ical::{Component, escape, parameter};
use crate::{
    error::{Error, Result},
    items::models::*,
    temporal::{TimeValue, dav_time, duration},
};
use chrono::Utc;
#[cfg(test)]
use serde_json::json;
#[derive(Debug, Clone)]
pub struct Document {
    pub(crate) root: Component,
}
impl Document {
    pub fn members(&self) -> impl Iterator<Item = &Component> {
        self.root
            .children
            .iter()
            .filter(|c| matches!(c.name.as_str(), "VEVENT" | "VTODO" | "VJOURNAL"))
    }
    pub fn parse(s: &str) -> Result<Self> {
        let root = Component::parse(s)?;
        if root.name != "VCALENDAR" {
            return Err(Error::new("invalid_calendar", "Expected VCALENDAR"));
        }
        let doc = Self { root };
        let mut members = doc.members();
        let first = members
            .next()
            .ok_or_else(|| Error::new("invalid_calendar", "Missing calendar item"))?;
        let uid = first
            .value("UID")
            .filter(|s| !s.is_empty())
            .ok_or_else(|| Error::new("invalid_calendar", "Missing UID"))?;
        if members.any(|c| c.name != first.name || c.value("UID") != Some(uid)) {
            return Err(Error::new(
                "invalid_calendar",
                "Expected a single UID and component type",
            ));
        }
        drop(members);
        Ok(doc)
    }
    pub fn create(kind: ComponentKind, uid: &str, patch: &ItemPatch) -> Result<Self> {
        if uid.is_empty() || uid.len() > 255 || uid.contains(['\r', '\n', '\0']) {
            return Err(Error::input("Invalid UID"));
        }
        let mut root = Component::new("VCALENDAR");
        root.set("VERSION", "2.0")?;
        root.set("PRODID", "-//caldav-mcp//EN")?;
        let mut c = Component::new(kind.as_str());
        c.set("UID", &escape(uid))?;
        c.set("CREATED", &dav_time(Utc::now()))?;
        apply(&mut c, patch)?;
        root.children.push(c);
        let mut doc = Self { root };
        doc.add_timezones()?;
        Ok(doc)
    }
    pub fn edit(&self, patch: &ItemPatch, scope: &Scope) -> Result<Self> {
        let mut doc = self.clone();
        match scope {
            Scope::Series => {
                let ids: Vec<_> = doc
                    .root
                    .children
                    .iter()
                    .enumerate()
                    .filter(|(_, c)| {
                        matches!(c.name.as_str(), "VEVENT" | "VTODO" | "VJOURNAL")
                            && c.get("RECURRENCE-ID").is_none()
                    })
                    .map(|(i, _)| i)
                    .collect();
                if ids.len() != 1 {
                    return Err(Error::new(
                        "invalid_calendar",
                        "Exactly one master is required",
                    ));
                }
                apply(&mut doc.root.children[ids[0]], patch)?;
            }
            Scope::Occurrence { recurrence_id } => doc.edit_occurrence(patch, recurrence_id)?,
        }
        doc.add_timezones()?;
        Ok(doc)
    }
    pub fn to_ical(&self) -> String {
        self.root.to_ical()
    }
}
fn text_change(c: &mut Component, name: &str, change: &Change<String>, text: bool) -> Result<()> {
    match change {
        Change::Keep => {}
        Change::Remove => c.remove(name),
        Change::Set(v) => c.set(name, &if text { escape(v) } else { v.clone() })?,
    };
    Ok(())
}
fn time_change(c: &mut Component, name: &str, change: &Change<TimeValue>) -> Result<()> {
    match change {
        Change::Keep => {}
        Change::Remove => c.remove(name),
        Change::Set(t) => c.set_time(name, t)?,
    };
    Ok(())
}
pub(super) fn apply(c: &mut Component, p: &ItemPatch) -> Result<()> {
    for (n, v, text) in [
        ("SUMMARY", &p.summary, true),
        ("DESCRIPTION", &p.description, true),
        ("LOCATION", &p.location, true),
        ("STATUS", &p.status, false),
        ("DURATION", &p.duration, false),
        ("CLASS", &p.classification, false),
        ("TRANSP", &p.transparency, false),
        ("RRULE", &p.recurrence_rule, false),
    ] {
        text_change(c, n, v, text)?;
    }
    for (n, v) in [("DTSTART", &p.start), ("DTEND", &p.end), ("DUE", &p.due)] {
        time_change(c, n, v)?;
    }
    for (n, v, max) in [
        ("PERCENT-COMPLETE", &p.percent_complete, 100),
        ("PRIORITY", &p.priority, 9),
    ] {
        match v {
            Change::Keep => {}
            Change::Remove => c.remove(n),
            Change::Set(v) => {
                if *v > max {
                    return Err(Error::input(format!("Invalid {n}")));
                }
                c.set(n, &v.to_string())?;
            }
        }
    }
    match &p.categories {
        Change::Keep => {}
        Change::Remove => c.remove("CATEGORIES"),
        Change::Set(v) => c.set(
            "CATEGORIES",
            &v.iter().map(|s| escape(s)).collect::<Vec<_>>().join(","),
        )?,
    }
    for (n, change) in [
        ("RDATE", &p.recurrence_dates),
        ("EXDATE", &p.exception_dates),
    ] {
        if !matches!(change, Change::Keep) {
            c.remove(n);
            if let Change::Set(values) = change {
                for t in values {
                    c.push_time(n, t)?;
                }
            }
        }
    }
    if !matches!(p.related_to, Change::Keep) {
        c.remove("RELATED-TO");
        if let Change::Set(values) = &p.related_to {
            for r in values {
                if r.uid.is_empty()
                    || !["PARENT", "CHILD", "SIBLING"].contains(&r.relationship.as_str())
                {
                    return Err(Error::input("Invalid relation"));
                }
                c.push(&format!(
                    "RELATED-TO;RELTYPE={}:{}",
                    r.relationship,
                    escape(&r.uid)
                ))?;
            }
        }
    }
    if !matches!(p.attachments, Change::Keep) {
        c.remove("ATTACH");
        if let Change::Set(values) = &p.attachments {
            for a in values {
                url::Url::parse(&a.uri)
                    .map_err(|_| Error::input("Attachment requires absolute URI"))?;
                let mime = a
                    .media_type
                    .as_ref()
                    .map(|s| parameter(s).map(|p| format!(";FMTTYPE={p}")))
                    .transpose()?
                    .unwrap_or_default();
                c.push(&format!("ATTACH;VALUE=URI{mime}:{}", a.uri))?;
            }
        }
    }
    if !matches!(p.alarms, Change::Keep) {
        c.children.retain(|a| a.name != "VALARM");
        if let Change::Set(values) = &p.alarms {
            for a in values {
                let mut alarm = Component::new("VALARM");
                alarm.set("ACTION", "DISPLAY")?;
                alarm.set("DESCRIPTION", &escape(&a.description))?;
                match &a.trigger {
                    Trigger::Relative(s) => {
                        duration(s)?;
                        if !["START", "END"].contains(&a.relative_to.as_str()) {
                            return Err(Error::input("Invalid alarm relation"));
                        }
                        alarm.push(&format!("TRIGGER;RELATED={}:{}", a.relative_to, s))?;
                    }
                    Trigger::Absolute(t) => {
                        if t.is_date() || t.is_floating() || t.timezone().is_some() {
                            return Err(Error::input("Absolute alarm must be UTC"));
                        }
                        alarm.push(&format!("TRIGGER;VALUE=DATE-TIME:{}", t.ical().1))?;
                    }
                }
                c.children.push(alarm);
            }
        }
    }
    if c.name == "VTODO" && !matches!(p.status, Change::Keep) {
        c.remove("COMPLETED");
        match c.value("STATUS") {
            Some("COMPLETED") => {
                c.set("PERCENT-COMPLETE", "100")?;
                c.set("COMPLETED", &dav_time(Utc::now()))?;
            }
            Some("NEEDS-ACTION") => c.set("PERCENT-COMPLETE", "0")?,
            _ => {}
        }
    }
    let seq = c
        .value("SEQUENCE")
        .unwrap_or("0")
        .parse::<u64>()
        .map_err(|_| Error::new("invalid_calendar", "Invalid SEQUENCE"))?
        .checked_add(1)
        .ok_or_else(|| Error::input("Sequence overflow"))?;
    c.set("SEQUENCE", &seq.to_string())?;
    c.set("DTSTAMP", &dav_time(Utc::now()))?;
    c.set("LAST-MODIFIED", &dav_time(Utc::now()))?;
    validate(c)
}
fn validate(c: &Component) -> Result<()> {
    let start = c.time("DTSTART")?;
    let end = c.time("DTEND")?;
    let due = c.time("DUE")?;
    if c.name == "VEVENT" && start.is_none() {
        return Err(Error::input("Event requires start"));
    }
    if c.name != "VEVENT" && end.is_some()
        || c.name != "VTODO" && (due.is_some() || c.get("PERCENT-COMPLETE").is_some())
    {
        return Err(Error::input("Property not valid for this component type"));
    }
    for finish in [&end, &due].into_iter().flatten() {
        if let Some(s) = &start
            && (!s.compatible(finish) || finish.instant() <= s.instant())
        {
            return Err(Error::input("end/due must match start type and be later"));
        }
    }
    if let Some(d) = c.value("DURATION") {
        let d = duration(d)?;
        if c.name == "VJOURNAL"
            || start.is_none()
            || end.is_some()
            || due.is_some()
            || d <= chrono::Duration::zero()
            || start.as_ref().is_some_and(|s| s.is_date()) && d.num_seconds() % 86400 != 0
        {
            return Err(Error::input("Invalid duration or conflicting end/due"));
        }
    }
    if let Some(s) = c.value("STATUS") {
        let allowed: &[&str] = match c.name.as_str() {
            "VEVENT" => &["TENTATIVE", "CONFIRMED", "CANCELLED"],
            "VTODO" => &["NEEDS-ACTION", "IN-PROCESS", "COMPLETED", "CANCELLED"],
            _ => &["DRAFT", "FINAL", "CANCELLED"],
        };
        if !allowed.contains(&s) {
            return Err(Error::input("Invalid status for component"));
        }
    }
    if c.value("CLASS")
        .is_some_and(|s| !["PUBLIC", "PRIVATE", "CONFIDENTIAL"].contains(&s))
        || c.value("TRANSP")
            .is_some_and(|s| !["OPAQUE", "TRANSPARENT"].contains(&s))
    {
        return Err(Error::input("Invalid class/transparency"));
    }
    if c.get("RRULE").is_some() {
        super::recurrence::rules(c)?;
    }
    for n in ["RDATE", "EXDATE"] {
        for p in c.lines.iter().filter(|p| p.name() == n) {
            for v in p.value().split(',') {
                let t = TimeValue::from_ical(v, p.parameter("TZID"))?;
                if start.as_ref().is_none_or(|s| !s.compatible(&t)) {
                    return Err(Error::input("Recurrence date must match start type"));
                }
            }
        }
    }
    for alarm in c.children.iter().filter(|c| c.name == "VALARM") {
        if let Some(trigger) = alarm.get("TRIGGER")
            && trigger.parameter("VALUE") != Some("DATE-TIME")
        {
            duration(trigger.value())?;
            if trigger.parameter("RELATED") == Some("END") {
                if end.is_none() && due.is_none() && c.get("DURATION").is_none() {
                    return Err(Error::input("End-relative alarm requires end/due/duration"));
                }
            } else if start.is_none() {
                return Err(Error::input("Start-relative alarm requires start"));
            }
        }
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn edit_preserves_unknown_parameters_and_alarms() {
        let source = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VTODO\r\nUID:task\r\nDTSTART:20261102T100000Z\r\nSUMMARY:Old\r\nX-VENDOR;X-PARAM=hello:preserve me\r\nATTENDEE;CN=\"A: B\";PARTSTAT=ACCEPTED:mailto:a@example.org\r\nBEGIN:VALARM\r\nACTION:DISPLAY\r\nDESCRIPTION:Reminder\r\nTRIGGER:-PT15M\r\nEND:VALARM\r\nEND:VTODO\r\nEND:VCALENDAR\r\n";
        let doc = Document::parse(source).unwrap();
        let patch = serde_json::from_str(r#"{"summary":"New"}"#).unwrap();
        let result = doc.edit(&patch, &Scope::Series).unwrap().to_ical();
        for line in [
            "SUMMARY:New",
            "X-VENDOR;X-PARAM=hello:preserve me",
            "ATTENDEE;CN=\"A: B\";PARTSTAT=ACCEPTED:mailto:a@example.org",
            "TRIGGER:-PT15M",
        ] {
            assert!(result.contains(line), "{result}");
        }
        assert!(doc.to_ical().contains("SUMMARY:Old"));
    }
    #[test]
    fn creates_task_and_distinguishes_removal_from_omission() {
        let patch = serde_json::from_str(
            r#"{"summary":"買う","description":"お茶","due":{"kind":"date","value":"2026-11-09"}}"#,
        )
        .unwrap();
        let doc = Document::create(ComponentKind::Task, "shopping", &patch).unwrap();
        assert!(doc.to_ical().contains("DUE;VALUE=DATE:20261109"));
        let kept = doc.edit(&ItemPatch::default(), &Scope::Series).unwrap();
        assert!(kept.to_ical().contains("DESCRIPTION:お茶"));
        let removed = doc
            .edit(
                &serde_json::from_str(r#"{"description":null}"#).unwrap(),
                &Scope::Series,
            )
            .unwrap();
        assert!(!removed.to_ical().contains("DESCRIPTION:"));
    }
}
#[cfg(test)]
mod recurrence_tests {
    use super::*;
    fn recurring() -> Document {
        Document::create(ComponentKind::Event,"weekly",&serde_json::from_value(json!({"summary":"Weekly","start":{"kind":"zoned","value":"2026-10-26T10:00:00","timezone":"America/New_York"},"end":{"kind":"zoned","value":"2026-10-26T11:00:00","timezone":"America/New_York"},"recurrence_rule":"FREQ=WEEKLY;COUNT=5"})).unwrap()).unwrap()
    }
    #[test]
    fn individual_exception_across_dst_keeps_master_and_duration() {
        let doc = recurring();
        assert!(doc.to_ical().contains("BEGIN:VTIMEZONE"));
        let scope=serde_json::from_value(json!({"kind":"occurrence","recurrence_id":{"kind":"utc","value":"2026-11-02T15:00:00Z"}})).unwrap();
        let result = doc
            .edit(
                &serde_json::from_value(json!({"summary":"Changed"})).unwrap(),
                &scope,
            )
            .unwrap();
        let exception = result
            .members()
            .find(|c| c.get("RECURRENCE-ID").is_some())
            .unwrap();
        assert_eq!(
            exception.time("DTSTART").unwrap().unwrap().instant(),
            "2026-11-02T15:00:00Z"
                .parse::<chrono::DateTime<Utc>>()
                .unwrap()
        );
        assert_eq!(
            exception.time("DTEND").unwrap().unwrap().instant(),
            "2026-11-02T16:00:00Z"
                .parse::<chrono::DateTime<Utc>>()
                .unwrap()
        );
        assert!(exception.get("RRULE").is_none());
        assert_eq!(result.members().count(), 2);
    }
    #[test]
    fn invalid_recurrence_and_due_duration_conflict_are_rejected() {
        assert!(Document::create(ComponentKind::Event,"bad",&serde_json::from_value(json!({"start":{"kind":"utc","value":"2026-11-02T10:00:00Z"},"recurrence_rule":"FREQ=WEEKLY;INTERVAL=0"})).unwrap()).is_err());
        assert!(Document::create(ComponentKind::Task,"bad",&serde_json::from_value(json!({"start":{"kind":"date","value":"2026-11-02"},"due":{"kind":"date","value":"2026-11-03"},"duration":"P1D"})).unwrap()).is_err());
    }
}
#[cfg(test)]
mod dst_duration_tests {
    use super::*;
    #[test]
    fn recurrence_preserves_exact_dtend_duration_when_original_crosses_dst() {
        let fields=serde_json::from_value(json!({"start":{"kind":"zoned","value":"2026-11-01T00:30:00","timezone":"America/New_York"},"end":{"kind":"zoned","value":"2026-11-01T02:30:00","timezone":"America/New_York"},"recurrence_rule":"FREQ=DAILY;COUNT=2"})).unwrap();
        let doc = Document::create(ComponentKind::Event, "dst-duration", &fields).unwrap();
        let scope=serde_json::from_value(json!({"kind":"occurrence","recurrence_id":{"kind":"utc","value":"2026-11-02T05:30:00Z"}})).unwrap();
        let result = doc.edit(&ItemPatch::default(), &scope).unwrap();
        let c = result
            .members()
            .find(|c| c.get("RECURRENCE-ID").is_some())
            .unwrap();
        assert_eq!(
            c.time("DTEND").unwrap().unwrap().instant(),
            "2026-11-02T08:30:00Z"
                .parse::<chrono::DateTime<Utc>>()
                .unwrap()
        );
    }
}

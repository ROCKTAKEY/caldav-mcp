use crate::{
    error::{Error, Result},
    temporal::TimeValue,
};
use serde_json::{Value, json};
/// Keep the original logical line: editing SUMMARY must not rewrite vendor parameters.
#[derive(Debug, Clone)]
pub struct ContentLine {
    raw: String,
    parsed: ical::property::Property,
}
impl ContentLine {
    pub fn parse(raw: &str) -> Result<Self> {
        if raw.contains(['\r', '\n', '\0']) {
            return Err(Error::input("Invalid content line"));
        }
        let parsed = ical::PropertyParser::from_reader(raw.as_bytes())
            .next()
            .ok_or_else(|| Error::input("Empty content line"))?
            .map_err(|_| Error::new("invalid_calendar", "Malformed content line"))?;
        Ok(Self {
            raw: raw.into(),
            parsed,
        })
    }
    pub fn name(&self) -> &str {
        &self.parsed.name
    }
    pub fn value(&self) -> &str {
        self.parsed.value.as_deref().unwrap_or("")
    }
    pub fn parameter(&self, name: &str) -> Option<&str> {
        self.parsed
            .params
            .as_ref()?
            .iter()
            .find(|(k, _)| k == name)?
            .1
            .first()
            .map(String::as_str)
    }
    pub fn time(&self) -> Result<TimeValue> {
        TimeValue::from_ical(self.value(), self.parameter("TZID"))
    }
}
#[derive(Debug, Clone)]
pub struct Component {
    pub name: String,
    pub lines: Vec<ContentLine>,
    pub children: Vec<Component>,
}
impl Component {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.into(),
            lines: vec![],
            children: vec![],
        }
    }
    pub fn get(&self, name: &str) -> Option<&ContentLine> {
        self.lines.iter().find(|p| p.name() == name)
    }
    pub fn value(&self, name: &str) -> Option<&str> {
        self.get(name).map(ContentLine::value)
    }
    pub fn remove(&mut self, name: &str) {
        self.lines.retain(|p| p.name() != name);
    }
    pub fn push(&mut self, line: &str) -> Result<()> {
        self.lines.push(ContentLine::parse(line)?);
        Ok(())
    }
    pub fn set(&mut self, name: &str, value: &str) -> Result<()> {
        self.remove(name);
        self.push(&format!("{name}:{value}"))
    }
    pub fn set_time(&mut self, name: &str, time: &TimeValue) -> Result<()> {
        self.remove(name);
        self.push_time(name, time)
    }
    pub fn push_time(&mut self, name: &str, time: &TimeValue) -> Result<()> {
        let (p, v) = time.ical();
        self.push(&format!("{name}{p}:{v}"))
    }
    pub fn time(&self, name: &str) -> Result<Option<TimeValue>> {
        self.get(name).map(ContentLine::time).transpose()
    }
    pub fn to_ical(&self) -> String {
        fn folded(s: &str) -> String {
            let mut out = String::new();
            let mut n = 0;
            for c in s.chars() {
                if n + c.len_utf8() > 75 {
                    out.push_str("\r\n ");
                    n = 1;
                }
                out.push(c);
                n += c.len_utf8();
            }
            out.push_str("\r\n");
            out
        }
        let mut s = folded(&format!("BEGIN:{}", self.name));
        for p in &self.lines {
            s.push_str(&folded(&p.raw));
        }
        for c in &self.children {
            s.push_str(&c.to_ical());
        }
        s.push_str(&folded(&format!("END:{}", self.name)));
        s
    }
    pub fn parse(s: &str) -> Result<Self> {
        let bad = || Error::new("invalid_calendar", "Malformed component nesting");
        let mut stack: Vec<Self> = vec![];
        let mut result = None;
        for line in ical::LineReader::new(s.as_bytes()) {
            let p = ContentLine::parse(line.as_str())?;
            match p.name() {
                "BEGIN" => {
                    if stack.len() > 32 || result.is_some() {
                        return Err(bad());
                    }
                    stack.push(Self::new(&p.value().to_ascii_uppercase()));
                }
                "END" => {
                    let c = stack.pop().ok_or_else(bad)?;
                    if !c.name.eq_ignore_ascii_case(p.value()) {
                        return Err(bad());
                    }
                    if let Some(parent) = stack.last_mut() {
                        parent.children.push(c)
                    } else {
                        result = Some(c)
                    }
                }
                _ => stack.last_mut().ok_or_else(bad)?.lines.push(p),
            }
        }
        if !stack.is_empty() {
            return Err(bad());
        }
        result.ok_or_else(bad)
    }
    pub fn describe(&self) -> Result<Value> {
        let mut v = json!({"kind":self.name,"uid":unescape(self.value("UID").unwrap_or(""))});
        for (p, key) in [
            ("SUMMARY", "summary"),
            ("STATUS", "status"),
            ("LOCATION", "location"),
            ("PERCENT-COMPLETE", "percent_complete"),
            ("PRIORITY", "priority"),
            ("RRULE", "recurrence_rule"),
        ] {
            if let Some(s) = self.value(p) {
                v[key] = json!(unescape(s));
            }
        }
        for (p, key) in [
            ("DTSTART", "start"),
            ("DTEND", "end"),
            ("DUE", "due"),
            ("RECURRENCE-ID", "recurrence_id"),
        ] {
            if let Some(t) = self.time(p)? {
                v[key] = json!(t);
            }
        }
        Ok(v)
    }
}
pub fn escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .replace('\n', "\\n")
        .replace(';', "\\;")
        .replace(',', "\\,")
}
fn unescape(s: &str) -> String {
    let mut out = String::new();
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c == '\\' {
            match it.next() {
                Some('n' | 'N') => out.push('\n'),
                Some(c) => out.push(c),
                None => out.push('\\'),
            }
        } else {
            out.push(c)
        }
    }
    out
}
pub(super) fn parameter(s: &str) -> Result<String> {
    if s.contains(['\r', '\n', '\0']) {
        return Err(Error::input("Invalid parameter"));
    }
    Ok(format!("\"{}\"", s.replace('^', "^^").replace('"', "^'")))
}

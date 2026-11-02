use super::{document::Document, models::*, timezones::timezone_document};
use crate::{
    calendars::{CalendarRef, Calendars},
    dav::{C, D, envelope, escape, properties},
    error::{Error, Result},
    temporal::{Period, dav_time},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub struct RetrievedItem {
    target: ResourceRef,
    revision: Revision,
    document: Document,
}
pub struct EditedItem {
    target: ResourceRef,
    revision: Revision,
    document: Document,
}
impl RetrievedItem {
    pub fn check_revision(&self, r: &Revision) -> Result<()> {
        if &self.revision != r {
            return Err(Error::new("conflict", "Item changed; retrieve it again"));
        }
        Ok(())
    }
    pub fn is_task(&self) -> bool {
        self.document
            .members()
            .next()
            .is_some_and(|c| c.name == "VTODO")
    }
    pub fn describe(&self) -> Result<Value> {
        Ok(
            json!({"ref":self.target,"revision":self.revision,"components":self.document.members().map(|c|c.describe()).collect::<Result<Vec<_>>>()?,"icalendar":self.document.to_ical()}),
        )
    }
    pub fn edit(self, p: &ItemPatch, s: &Scope) -> Result<EditedItem> {
        let document = self.document.edit(p, s)?;
        Ok(EditedItem {
            target: self.target,
            revision: self.revision,
            document,
        })
    }
    pub fn exclude(self, t: &crate::temporal::TimeValue) -> Result<EditedItem> {
        let document = self.document.exclude(t)?;
        Ok(EditedItem {
            target: self.target,
            revision: self.revision,
            document,
        })
    }
}
pub struct Items<'a> {
    pub calendars: Calendars<'a>,
    pub timezone: &'a str,
}
impl Items<'_> {
    fn check_ref(&self, r: &ResourceRef) -> Result<url::Url> {
        let parent = self.calendars.check_ref(&r.calendar)?;
        let target = self.calendars.client.resolve(&r.url, None)?;
        let parent = percent_encoding::percent_decode_str(parent.path())
            .decode_utf8()
            .map_err(|_| Error::input("Invalid path"))?;
        let path = percent_encoding::percent_decode_str(target.path())
            .decode_utf8()
            .map_err(|_| Error::input("Invalid path"))?;
        if path
            .strip_prefix(parent.as_ref())
            .is_none_or(|rest| rest.is_empty() || rest.contains('/'))
        {
            return Err(Error::new(
                "invalid_target",
                "Resource must be a direct child of its calendar",
            ));
        }
        Ok(target)
    }
    pub async fn get(&self, r: ResourceRef) -> Result<RetrievedItem> {
        let url = self.check_ref(&r)?;
        let response = self
            .calendars
            .client
            .request("GET", url.as_str(), None, &[])
            .await?;
        let revision = Revision::parse(
            response
                .headers
                .get("ETag")
                .and_then(|v| v.to_str().ok())
                .unwrap_or(""),
        )
        .map_err(|_| Error::new("invalid_response", "Resource lacks a strong ETag"))?;
        let document = Document::parse(&response.body)?;
        Ok(RetrievedItem {
            target: r,
            revision,
            document,
        })
    }
    pub async fn create(
        &self,
        calendar: CalendarRef,
        kind: ComponentKind,
        uid: &str,
        fields: &ItemPatch,
    ) -> Result<RetrievedItem> {
        let doc = Document::create(kind, uid, fields)?;
        self.calendars
            .get(&calendar)
            .await?
            .require("write-content", Some(kind))?;
        let url = format!("{}{:x}.ics", calendar.url, Sha256::digest(uid.as_bytes()));
        let target = ResourceRef { calendar, url };
        self.write(target, doc, "If-None-Match", "*").await
    }
    pub async fn save(&self, item: EditedItem) -> Result<RetrievedItem> {
        self.check_ref(&item.target)?;
        self.calendars
            .get(&item.target.calendar)
            .await?
            .require("write-content", None)?;
        self.write(item.target, item.document, "If-Match", &item.revision.etag)
            .await
    }
    async fn write(
        &self,
        target: ResourceRef,
        document: Document,
        condition: &str,
        value: &str,
    ) -> Result<RetrievedItem> {
        let url = self.check_ref(&target)?;
        self.calendars
            .client
            .request(
                "PUT",
                url.as_str(),
                Some(document.to_ical()),
                &[
                    ("Content-Type", "text/calendar; charset=utf-8"),
                    (condition, value),
                ],
            )
            .await
            .map_err(|e| e.detail("target", &target))?;
        self.get(target.clone()).await.map_err(|_| {
            Error::new(
                "readback_failed",
                "Write succeeded; retrieve target before retrying",
            )
            .detail("write_succeeded", true)
            .detail("target", target)
        })
    }
    pub async fn delete(&self, target: &ResourceRef, revision: &Revision) -> Result<Value> {
        self.check_ref(target)?;
        self.calendars
            .get(&target.calendar)
            .await?
            .require("write-content", None)?;
        self.calendars
            .client
            .request("DELETE", &target.url, None, &[("If-Match", &revision.etag)])
            .await?;
        Ok(json!({"status":"deleted"}))
    }
    pub async fn search(&self, calendar: &CalendarRef, q: &SearchQuery) -> Result<SearchResult> {
        let info = self.calendars.get(calendar).await?;
        info.require("read", Some(q.kind))?;
        let mut data = "<C:calendar-data>".to_string();
        if q.expand {
            let p = q.period.as_ref().expect("validated expansion period");
            data.push_str(&format!(
                "<C:expand start=\"{}\" end=\"{}\"/>",
                dav_time(p.start),
                dav_time(p.end)
            ));
        }
        data.push_str("</C:calendar-data>");
        let mut filter = String::new();
        if let Some(p) = &q.period {
            filter.push_str(&format!(
                "<C:time-range start=\"{}\" end=\"{}\"/>",
                dav_time(p.start),
                dav_time(p.end)
            ));
        }
        if let Some(text) = &q.text {
            filter.push_str(&format!("<C:prop-filter name=\"SUMMARY\"><C:text-match collation=\"i;unicode-casemap\">{}</C:text-match></C:prop-filter>",escape(text)));
        }
        let timezone = if self.timezone != "UTC" {
            format!(
                "<C:timezone>{}</C:timezone>",
                escape(&timezone_document(self.timezone)?)
            )
        } else {
            String::new()
        };
        let body = envelope(
            "C:calendar-query",
            &format!(
                "<D:prop><D:getetag/>{data}</D:prop><C:filter><C:comp-filter name=\"VCALENDAR\"><C:comp-filter name=\"{}\">{filter}</C:comp-filter></C:comp-filter></C:filter>{timezone}",
                q.kind.as_str()
            ),
        );
        let response = self
            .calendars
            .client
            .xml("REPORT", &calendar.url, body, "1")
            .await?;
        let mut rows = properties(&response.body)?;
        rows.sort_by(|a, b| a.href.cmp(&b.href));
        let mut result = SearchResult::default();
        for row in rows {
            let parse = || -> Result<Vec<Value>> {
                if row.status >= 400 || !row.failed.is_empty() {
                    return Err(Error::new(
                        "partial_response",
                        "Calendar query returned failed properties",
                    )
                    .detail("properties", &row.failed));
                }
                let data = row
                    .text(C, "calendar-data")
                    .ok_or_else(|| Error::new("invalid_response", "Missing calendar-data"))?;
                let revision = Revision::parse(row.text(D, "getetag").unwrap_or(""))?;
                let base = self.calendars.check_ref(calendar)?;
                let target = ResourceRef {
                    calendar: calendar.clone(),
                    url: self
                        .calendars
                        .client
                        .resolve(&row.href, Some(&base))?
                        .into(),
                };
                self.check_ref(&target)?;
                let doc = Document::parse(data)?;
                let mut found = vec![];
                for c in doc.members().filter(|c| c.name == q.kind.as_str()) {
                    if q.completed
                        .is_some_and(|v| v != (c.value("STATUS") == Some("COMPLETED")))
                    {
                        continue;
                    }
                    let mut value = c.describe()?;
                    value["ref"] = json!(target);
                    value["revision"] = json!(revision);
                    found.push(value);
                }
                Ok(found)
            };
            match parse() {
                Ok(found) => result.items.extend(found),
                Err(e) => result.errors.push(json!({"href":row.href,"error":e})),
            }
        }
        result.truncated = result.items.len() > q.limit;
        result.items.truncate(q.limit);
        Ok(result)
    }
}
#[derive(Default)]
pub struct SearchResult {
    pub items: Vec<Value>,
    pub errors: Vec<Value>,
    pub truncated: bool,
}
pub struct SearchQuery {
    pub(crate) kind: ComponentKind,
    pub(crate) period: Option<Period>,
    pub(crate) text: Option<String>,
    pub(crate) completed: Option<bool>,
    pub(crate) expand: bool,
    pub(crate) limit: usize,
}
impl SearchQuery {
    pub fn new(
        kind: ComponentKind,
        period: Option<Period>,
        text: Option<String>,
        completed: Option<bool>,
        expand: bool,
        limit: usize,
    ) -> Result<Self> {
        if !(1..=500).contains(&limit) || expand && period.is_none() {
            return Err(Error::input(
                "Limit must be 1..500 and expansion requires a bounded period",
            ));
        }
        Ok(Self {
            kind,
            period,
            text,
            completed,
            expand,
            limit,
        })
    }
}

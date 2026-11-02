use super::models::*;
use crate::{
    dav::{A, C, D, DavClient, Properties, envelope, escape, properties},
    error::{Error, Result},
    items::{models::Change, timezones::timezone_document},
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
const PROPS: &[&str] = &[
    "D:resourcetype",
    "D:displayname",
    "C:calendar-description",
    "C:calendar-timezone",
    "A:calendar-color",
    "C:supported-calendar-component-set",
    "D:current-user-privilege-set",
    "D:supported-report-set",
    "D:owner",
];
pub struct Calendars<'a> {
    pub client: &'a DavClient,
}
impl<'a> Calendars<'a> {
    pub fn check_ref(&self, r: &CalendarRef) -> Result<url::Url> {
        if r.account != self.client.config.id {
            return Err(Error::new(
                "invalid_target",
                "Calendar belongs to different account",
            ));
        }
        let u = self.client.resolve(&r.url, None)?;
        if !u.path().ends_with('/') {
            return Err(Error::new("invalid_target", "Calendar URL must end with /"));
        }
        Ok(u)
    }
    fn info(&self, row: Properties, base: &url::Url) -> Result<Option<CalendarInfo>> {
        if row
            .get(D, "resourcetype")
            .and_then(|n| n.child(C, "calendar"))
            .is_none()
        {
            return Ok(None);
        }
        let url = self.client.resolve(&row.href, Some(base))?;
        Ok(Some(CalendarInfo {
            target: CalendarRef {
                account: self.client.config.id.clone(),
                url: url.into(),
            },
            name: row.text(D, "displayname").map(str::to_owned),
            description: row.text(C, "calendar-description").map(str::to_owned),
            color: row.text(A, "calendar-color").map(str::to_owned),
            timezone_icalendar: row.text(C, "calendar-timezone").map(str::to_owned),
            components: row.get(C, "supported-calendar-component-set").map(|n| {
                n.children
                    .iter()
                    .filter_map(|c| c.attrs.get("name").cloned())
                    .collect()
            }),
            privileges: row
                .get(D, "current-user-privilege-set")
                .map(|n| n.descendant_keys("privilege")),
            reports: row
                .get(D, "supported-report-set")
                .map(|n| n.descendant_keys("report")),
            owner: row
                .get(D, "owner")
                .and_then(|n| n.child(D, "href"))
                .map(|n| n.text.clone()),
            unavailable_properties: row.failed,
        }))
    }
    pub async fn get(&self, r: &CalendarRef) -> Result<CalendarInfo> {
        let url = self.check_ref(r)?;
        for row in self.client.propfind(url.as_str(), PROPS, "0").await? {
            if self.client.resolve(&row.href, Some(&url))? == url
                && let Some(info) = self.info(row, &url)?
            {
                return Ok(info);
            }
        }
        Err(Error::new(
            "not_calendar",
            "Target is not an accessible calendar",
        ))
    }
    fn hrefs(
        &self,
        rows: &[Properties],
        ns: &str,
        name: &str,
        base: &url::Url,
    ) -> Result<Vec<url::Url>> {
        let mut values = vec![];
        for r in rows {
            if let Some(n) = r.get(ns, name) {
                for h in n.children.iter().filter(|c| c.ns == D && c.name == "href") {
                    values.push(self.client.resolve(&h.text, Some(base))?);
                }
            }
        }
        Ok(values)
    }
    pub async fn homes(&self) -> Result<Vec<url::Url>> {
        let base = &self.client.config.url;
        let rows = self
            .client
            .propfind(
                base.as_str(),
                &["D:current-user-principal", "C:calendar-home-set"],
                "0",
            )
            .await?;
        let homes = self.hrefs(&rows, C, "calendar-home-set", base)?;
        if !homes.is_empty() {
            return Ok(homes);
        }
        let principals = self.hrefs(&rows, D, "current-user-principal", base)?;
        let principal = principals
            .first()
            .ok_or_else(|| Error::new("discovery_failed", "Missing current-user-principal"))?;
        let rows = self
            .client
            .propfind(principal.as_str(), &["C:calendar-home-set"], "0")
            .await?;
        let homes = self.hrefs(&rows, C, "calendar-home-set", principal)?;
        if homes.is_empty() {
            return Err(Error::new("discovery_failed", "Missing calendar-home-set"));
        }
        Ok(homes)
    }
    pub async fn list(&self) -> Result<Vec<CalendarInfo>> {
        let mut found = BTreeMap::new();
        for home in self.homes().await? {
            for row in self.client.propfind(home.as_str(), PROPS, "1").await? {
                if row.status >= 400 {
                    return Err(Error::new("partial_response", "Calendar listing failed")
                        .detail("href", row.href));
                }
                if let Some(info) = self.info(row, &home)? {
                    found.insert(info.target.url.clone(), info);
                }
            }
        }
        for url in &self.client.config.calendars {
            let info = self
                .get(&CalendarRef {
                    account: self.client.config.id.clone(),
                    url: url.to_string(),
                })
                .await?;
            found.insert(info.target.url.clone(), info);
        }
        Ok(found.into_values().collect())
    }
    pub async fn create(&self, spec: NewCalendar, default_tz: &str) -> Result<CalendarInfo> {
        if spec.account != self.client.config.id
            || spec.name.is_empty()
            || spec.components.is_empty()
        {
            return Err(Error::input(
                "Account, name and supported components are required",
            ));
        }
        let home = self.homes().await?.remove(0);
        let url = format!(
            "{}{}/",
            home.as_str().trim_end_matches('/').to_owned() + "/",
            uuid::Uuid::new_v4()
        );
        let r = CalendarRef {
            account: spec.account,
            url,
        };
        let mut prop = format!("<D:displayname>{}</D:displayname>", escape(&spec.name));
        if let Some(d) = spec.description {
            prop.push_str(&format!(
                "<C:calendar-description>{}</C:calendar-description>",
                escape(&d)
            ));
        }
        prop.push_str(&format!(
            "<C:calendar-timezone>{}</C:calendar-timezone><C:supported-calendar-component-set>",
            escape(&timezone_document(
                spec.timezone.as_deref().unwrap_or(default_tz)
            )?)
        ));
        for kind in spec.components {
            prop.push_str(&format!("<C:comp name=\"{}\"/>", kind.as_str()));
        }
        prop.push_str("</C:supported-calendar-component-set>");
        self.client
            .xml(
                "MKCALENDAR",
                &r.url,
                envelope(
                    "C:mkcalendar",
                    &format!("<D:set><D:prop>{prop}</D:prop></D:set>"),
                ),
                "0",
            )
            .await
            .map_err(|e| e.detail("target", &r))?;
        self.readback(&r).await
    }
    async fn readback(&self, r: &CalendarRef) -> Result<CalendarInfo> {
        self.get(r).await.map_err(|_| {
            Error::new(
                "readback_failed",
                "Write succeeded; retrieve metadata before retrying",
            )
            .detail("write_succeeded", true)
            .detail("target", r)
        })
    }
    pub async fn update(&self, r: &CalendarRef, patch: CalendarPatch) -> Result<CalendarInfo> {
        let info = self.get(r).await?;
        info.require("write-properties", None)?;
        let mut body = String::new();
        for (n, change) in [
            ("D:displayname", patch.name),
            ("C:calendar-description", patch.description),
            ("A:calendar-color", patch.color),
            ("C:calendar-timezone", patch.timezone),
        ] {
            match change {
                Change::Keep => {}
                Change::Remove => {
                    body.push_str(&format!("<D:remove><D:prop><{n}/></D:prop></D:remove>"))
                }
                Change::Set(v) => {
                    if n == "A:calendar-color"
                        && (!(v.len() == 7 || v.len() == 9)
                            || !v.starts_with('#')
                            || !v[1..].bytes().all(|b| b.is_ascii_hexdigit()))
                    {
                        return Err(Error::input("Invalid calendar color"));
                    }
                    let v = if n == "C:calendar-timezone" {
                        timezone_document(&v)?
                    } else {
                        v
                    };
                    body.push_str(&format!(
                        "<D:set><D:prop><{n}>{}</{n}></D:prop></D:set>",
                        escape(&v)
                    ));
                }
            }
        }
        if body.is_empty() {
            return Ok(info);
        }
        let response = self
            .client
            .xml(
                "PROPPATCH",
                &r.url,
                envelope("D:propertyupdate", &body),
                "0",
            )
            .await?;
        if response.status == 207 {
            for row in properties(&response.body)? {
                if row.status >= 400 || !row.failed.is_empty() {
                    return Err(Error::new(
                        "property_update_failed",
                        "Calendar properties were not all updated",
                    )
                    .detail("properties", row.failed));
                }
            }
        }
        self.readback(r).await
    }
    pub async fn delete(&self, r: &CalendarRef) -> Result<Value> {
        self.get(r).await?;
        let response = self.client.request("DELETE", &r.url, None, &[]).await?;
        if response.status == 207 {
            let failures: Vec<_> = properties(&response.body)?
                .into_iter()
                .filter(|row| !(200..300).contains(&row.status) || !row.failed.is_empty())
                .map(
                    |row| json!({"href": row.href, "status": row.status, "properties": row.failed}),
                )
                .collect();
            if !failures.is_empty() {
                return Err(Error::new(
                    "calendar_delete_failed",
                    "Calendar deletion failed for one or more resources",
                )
                .detail("resources", failures));
            }
        }
        Ok(json!({"status":"deleted"}))
    }
}

use crate::{
    calendars::{CalendarRef, Calendars},
    error::{Error, Result},
    operations::{Access, UpdateShare},
};
use serde_json::{Value, json};
pub struct Sharing<'a> {
    pub calendars: Calendars<'a>,
}
#[derive(Debug)]
struct SharePath(String);
impl SharePath {
    fn parse(path: &str) -> Result<Self> {
        let decoded = percent_encoding::percent_decode_str(path)
            .decode_utf8()
            .map_err(|_| Error::input("Invalid share path"))?;
        if !path.starts_with('/')
            || path.starts_with("//")
            || !path.ends_with('/')
            || path.contains(['?', '#'])
            || decoded.contains(['\\', '\r', '\n', '\0'])
            || decoded.split('/').any(|s| matches!(s, "." | ".."))
        {
            return Err(Error::input(
                "Share path must be an absolute collection path ending in /",
            ));
        }
        Ok(Self(path.into()))
    }
}
impl Sharing<'_> {
    async fn request(&self, action: &str, data: Value) -> Result<Value> {
        let base = self
            .calendars
            .client
            .config
            .sharing_url
            .as_ref()
            .ok_or_else(|| {
                Error::new(
                    "unsupported",
                    "Configure sharing_url for Radicale map sharing",
                )
            })?;
        let url = format!("{}/map/{action}", base.as_str().trim_end_matches('/'));
        let response = self
            .calendars
            .client
            .request(
                "POST",
                &url,
                Some(data.to_string()),
                &[
                    ("Content-Type", "application/json"),
                    ("Accept", "application/json"),
                ],
            )
            .await
            .map_err(|e| match e.code.as_str() {
                "not_found" => Error::new("unsupported", "Share or sharing API unavailable"),
                "outcome_unknown" if matches!(action, "list" | "info") => {
                    Error::new("unavailable", "Could not read sharing information")
                }
                _ => e,
            })?;
        let data: Value = serde_json::from_str(&response.body)
            .map_err(|_| Error::new("invalid_response", "Invalid sharing response"))?;
        match data.get("Status").and_then(Value::as_str) {
            Some("success") => Ok(data),
            Some("not-found") => Err(Error::new("not_found", "Share not found")),
            _ => Err(Error::new(
                "invalid_response",
                "Unexpected sharing API status",
            )),
        }
    }
    pub async fn capabilities(&self) -> Result<Value> {
        let v = self.request("info", json!({})).await?;
        Ok(
            json!({"enabled":v["FeatureEnabledCollectionByMap"].as_bool().unwrap_or(false),"creation_enabled_by_default":v["PermittedCreateCollectionByMap"].as_bool().unwrap_or(false)}),
        )
    }
    pub async fn list(&self) -> Result<Value> {
        let v = self.request("list", json!({})).await?;
        let rows = v["Content"]
            .as_array()
            .ok_or_else(|| Error::new("invalid_response", "Missing sharing list"))?;
        let mut found = vec![];
        for r in rows {
            for k in ["PathOrToken", "PathMapped", "Owner", "User", "Permissions"] {
                if !r[k].is_string() {
                    return Err(Error::new("invalid_response", "Malformed sharing entry"));
                }
            }
            found.push(json!({"path":r["PathOrToken"],"source_path":r["PathMapped"],"owner":r["Owner"],"user":r["User"],"permissions":r["Permissions"],"enabled_by_owner":r["EnabledByOwner"],"enabled_by_user":r["EnabledByUser"],"hidden_by_owner":r["HiddenByOwner"],"hidden_by_user":r["HiddenByUser"]}));
        }
        Ok(json!({"shares":found}))
    }
    pub async fn create(
        &self,
        calendar: &CalendarRef,
        user: &str,
        path: &str,
        access: Access,
    ) -> Result<Value> {
        let path = SharePath::parse(path)?;
        if user.is_empty() || user.contains(['/', '\\', '\r', '\n', '\0']) {
            return Err(Error::input("Expected Radicale username"));
        }
        let url = self.calendars.check_ref(calendar)?;
        self.calendars.get(calendar).await?;
        self.request("create",json!({"PathMapped":url.path(),"PathOrToken":path.0,"User":user,"Permissions":access.permissions(),"Enabled":true,"Hidden":false})).await?;
        Ok(
            json!({"status":"created","path":path.0,"user":user,"recipient_action":"Enable and unhide using recipient account"}),
        )
    }
    pub async fn update(&self, spec: UpdateShare) -> Result<Value> {
        let path = SharePath::parse(&spec.path)?;
        if spec.access.is_none() && spec.enabled.is_none() && spec.hidden.is_none() {
            return Err(Error::input("Specify access, enabled or hidden"));
        }
        let mut data = json!({"PathOrToken":path.0});
        if let Some(a) = spec.access {
            data["Permissions"] = json!(a.permissions());
        }
        if let Some(b) = spec.enabled {
            data["Enabled"] = json!(b);
        }
        if let Some(b) = spec.hidden {
            data["Hidden"] = json!(b);
        }
        self.request("update", data).await?;
        Ok(json!({"status":"updated","path":path.0}))
    }
    pub async fn delete(&self, path: &str) -> Result<Value> {
        let path = SharePath::parse(path)?;
        self.request("delete", json!({"PathOrToken":path.0}))
            .await?;
        Ok(json!({"status":"deleted","path":path.0}))
    }
}

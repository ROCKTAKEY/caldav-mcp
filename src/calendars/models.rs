use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CalendarRef {
    pub account: String,
    pub url: String,
}

use crate::{
    error::{Error, Result},
    items::models::{Change, ComponentKind},
};
use std::collections::BTreeMap;
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NewCalendar {
    pub account: String,
    pub name: String,
    #[serde(default = "all_components")]
    pub components: Vec<ComponentKind>,
    pub description: Option<String>,
    pub timezone: Option<String>,
}
fn all_components() -> Vec<ComponentKind> {
    vec![
        ComponentKind::Event,
        ComponentKind::Task,
        ComponentKind::Journal,
    ]
}
#[derive(Debug, Clone, Deserialize, JsonSchema, Default)]
#[serde(default, deny_unknown_fields)]
pub struct CalendarPatch {
    pub name: Change<String>,
    pub description: Change<String>,
    pub color: Change<String>,
    pub timezone: Change<String>,
}
#[derive(Debug, Clone, Serialize)]
pub struct CalendarInfo {
    #[serde(rename = "ref")]
    pub target: CalendarRef,
    pub name: Option<String>,
    pub description: Option<String>,
    pub color: Option<String>,
    pub timezone_icalendar: Option<String>,
    pub components: Option<Vec<String>>,
    pub privileges: Option<Vec<String>>,
    pub reports: Option<Vec<String>>,
    pub owner: Option<String>,
    pub unavailable_properties: BTreeMap<String, u16>,
}
impl CalendarInfo {
    pub fn require(&self, privilege: &str, kind: Option<ComponentKind>) -> Result<()> {
        if let Some(k) = kind
            && self
                .components
                .as_ref()
                .is_some_and(|v| !v.iter().any(|s| s == k.as_str()))
        {
            return Err(Error::new(
                "unsupported",
                "Calendar does not support component",
            ));
        }
        if let Some(privileges) = &self.privileges {
            let allowed = privileges.iter().any(|s| {
                s == &format!("{{DAV:}}{privilege}")
                    || s == "{DAV:}all"
                    || s == "{DAV:}write"
                        && matches!(
                            privilege,
                            "write-content" | "write-properties" | "bind" | "unbind"
                        )
            });
            if !allowed {
                return Err(Error::new(
                    "forbidden",
                    format!("Calendar does not grant {privilege}"),
                ));
            }
        }
        Ok(())
    }
}

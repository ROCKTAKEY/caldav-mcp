use crate::{calendars::CalendarRef, temporal::TimeValue};
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};

#[derive(Debug, Clone, Default)]
pub enum Change<T> {
    #[default]
    Keep,
    Remove,
    Set(T),
}
impl<'de, T: Deserialize<'de>> Deserialize<'de> for Change<T> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        Ok(match Option::<T>::deserialize(d)? {
            None => Self::Remove,
            Some(v) => Self::Set(v),
        })
    }
}
impl<T: JsonSchema> JsonSchema for Change<T> {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        Option::<T>::schema_name()
    }
    fn json_schema(g: &mut schemars::SchemaGenerator) -> schemars::Schema {
        Option::<T>::json_schema(g)
    }
    fn inline_schema() -> bool {
        true
    }
}
#[derive(Debug, Clone, Deserialize, JsonSchema, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ItemPatch {
    pub summary: Change<String>,
    pub description: Change<String>,
    pub location: Change<String>,
    pub start: Change<TimeValue>,
    pub end: Change<TimeValue>,
    pub due: Change<TimeValue>,
    pub duration: Change<String>,
    pub status: Change<String>,
    pub percent_complete: Change<u8>,
    pub priority: Change<u8>,
    pub categories: Change<Vec<String>>,
    pub classification: Change<String>,
    pub transparency: Change<String>,
    pub recurrence_rule: Change<String>,
    pub recurrence_dates: Change<Vec<TimeValue>>,
    pub exception_dates: Change<Vec<TimeValue>>,
    pub related_to: Change<Vec<Relation>>,
    pub attachments: Change<Vec<Attachment>>,
    pub alarms: Change<Vec<Alarm>>,
}
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Relation {
    pub uid: String,
    #[serde(default = "parent")]
    pub relationship: String,
}
fn parent() -> String {
    "PARENT".into()
}
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Attachment {
    pub uri: String,
    pub media_type: Option<String>,
}
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Alarm {
    pub description: String,
    pub trigger: Trigger,
    #[serde(default = "start")]
    pub relative_to: String,
}
fn start() -> String {
    "START".into()
}
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Trigger {
    Relative(String),
    Absolute(TimeValue),
}
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Scope {
    Series,
    Occurrence { recurrence_id: TimeValue },
}
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ResourceRef {
    pub calendar: CalendarRef,
    pub url: String,
}
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(try_from = "RevisionInput")]
pub struct Revision {
    pub(crate) etag: String,
}
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RevisionInput {
    etag: String,
}
impl TryFrom<RevisionInput> for Revision {
    type Error = crate::error::Error;
    fn try_from(r: RevisionInput) -> crate::error::Result<Self> {
        Self::parse(&r.etag)
    }
}
impl Revision {
    pub fn parse(s: &str) -> crate::error::Result<Self> {
        if s.len() < 2
            || !s.starts_with('"')
            || !s.ends_with('"')
            || s[1..s.len() - 1]
                .chars()
                .any(|c| c == '"' || c.is_control())
        {
            return Err(crate::error::Error::input(
                "A strong quoted ETag is required",
            ));
        }
        Ok(Self { etag: s.into() })
    }
}
#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
pub enum ComponentKind {
    #[serde(rename = "VEVENT")]
    Event,
    #[serde(rename = "VTODO")]
    Task,
    #[serde(rename = "VJOURNAL")]
    Journal,
}
impl ComponentKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Event => "VEVENT",
            Self::Task => "VTODO",
            Self::Journal => "VJOURNAL",
        }
    }
}

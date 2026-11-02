use crate::{
    calendars::{CalendarPatch, CalendarRef, NewCalendar},
    items::models::{ItemPatch, ResourceRef, Revision, Scope},
    temporal::Period,
};
use schemars::JsonSchema;
use serde::Deserialize;
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(tag = "tool", content = "arguments", rename_all = "snake_case")]
pub enum Command {
    ListAccounts(Empty),
    ListCalendars(ListCalendars),
    GetCalendar(Calendar),
    CreateCalendar(CreateCalendar),
    UpdateCalendar(UpdateCalendar),
    DeleteCalendar(Calendar),
    SearchEvents(SearchEvents),
    SearchTasks(SearchTasks),
    SearchJournals(SearchJournals),
    GetItem(Item),
    CreateEvent(CreateItem),
    CreateTask(CreateItem),
    CreateJournal(CreateItem),
    UpdateItem(UpdateItem),
    DeleteItem(EditItem),
    CompleteTask(EditItem),
    ReopenTask(EditItem),
    FreeBusy(FreeBusy),
    SharingCapabilities(Account),
    ListShares(Account),
    CreateShare(CreateShare),
    UpdateShare(UpdateShare),
    DeleteShare(DeleteShare),
}
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Empty {}
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListCalendars {
    pub accounts: Option<Vec<String>>,
}
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Calendar {
    pub calendar: CalendarRef,
}
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateCalendar {
    pub spec: NewCalendar,
}
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateCalendar {
    pub calendar: CalendarRef,
    pub changes: CalendarPatch,
}
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Item {
    #[serde(rename = "ref")]
    pub target: ResourceRef,
}
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateItem {
    pub calendar: CalendarRef,
    pub uid: String,
    pub fields: ItemPatch,
}
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EditItem {
    #[serde(rename = "ref")]
    pub target: ResourceRef,
    pub revision: Revision,
    pub scope: Scope,
}
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateItem {
    #[serde(rename = "ref")]
    pub target: ResourceRef,
    pub revision: Revision,
    pub scope: Scope,
    pub changes: ItemPatch,
}
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FreeBusy {
    pub calendars: Vec<CalendarRef>,
    pub period: Period,
}
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Account {
    pub account: String,
}
#[derive(Debug, Clone, Copy, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum Access {
    #[default]
    Read,
    ReadWrite,
    FreeBusy,
}
impl Access {
    pub fn permissions(self) -> &'static str {
        match self {
            Self::Read => "r",
            Self::ReadWrite => "rw",
            Self::FreeBusy => "f",
        }
    }
}
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateShare {
    pub calendar: CalendarRef,
    pub user: String,
    pub path: String,
    #[serde(default)]
    pub access: Access,
}
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateShare {
    pub account: String,
    pub path: String,
    pub access: Option<Access>,
    pub enabled: Option<bool>,
    pub hidden: Option<bool>,
}
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DeleteShare {
    pub account: String,
    pub path: String,
}
fn yes() -> bool {
    true
}
fn limit() -> usize {
    100
}
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchEvents {
    pub calendars: Vec<CalendarRef>,
    pub period: Period,
    pub text: Option<String>,
    #[serde(default = "yes")]
    pub expand: bool,
    #[serde(default = "limit")]
    pub limit: usize,
}
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchTasks {
    pub calendars: Vec<CalendarRef>,
    pub period: Option<Period>,
    pub text: Option<String>,
    pub completed: Option<bool>,
    #[serde(default)]
    pub expand: bool,
    #[serde(default = "limit")]
    pub limit: usize,
}
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchJournals {
    pub calendars: Vec<CalendarRef>,
    pub period: Option<Period>,
    pub text: Option<String>,
    #[serde(default = "limit")]
    pub limit: usize,
}

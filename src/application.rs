use crate::{
    accounts::Settings,
    calendars::{CalendarRef, Calendars},
    dav::DavClient,
    error::{Error, Result},
    items::{
        models::*,
        service::{Items, SearchQuery},
    },
    operations::*,
};
use serde_json::{Value, json};
use std::collections::BTreeMap;
pub struct Application {
    pub settings: Settings,
    clients: BTreeMap<String, DavClient>,
}
impl Application {
    pub fn new(settings: Settings) -> Result<Self> {
        let clients = settings
            .accounts
            .iter()
            .cloned()
            .map(|a| Ok((a.id.clone(), DavClient::new(a)?)))
            .collect::<Result<_>>()?;
        Ok(Self { settings, clients })
    }
    pub fn calendars(&self, account: &str) -> Result<Calendars<'_>> {
        Ok(Calendars {
            client: self.clients.get(account).ok_or_else(|| {
                Error::new("unknown_account", "Account is not configured")
                    .detail("account", account)
            })?,
        })
    }
    pub fn items(&self, account: &str) -> Result<Items<'_>> {
        Ok(Items {
            calendars: self.calendars(account)?,
            timezone: &self.settings.timezone,
        })
    }
    pub async fn invoke(&self, name: &str, arguments: Value) -> Result<Value> {
        let command: Command = serde_json::from_value(json!({"tool":name,"arguments":arguments}))
            .map_err(|e| Error::input(e.to_string()))?;
        self.execute(command).await
    }
    async fn execute(&self, c: Command) -> Result<Value> {
        match c {
            Command::ListAccounts(_) => Ok(
                json!({"accounts":self.settings.accounts.iter().map(|a|json!({"id":a.id,"url":a.url,"username":a.username,"default_calendar":a.default_calendar,"sharing_configured":a.sharing_url.is_some()})).collect::<Vec<_>>(),"timezone":self.settings.timezone,"now":chrono::Utc::now().with_timezone(&self.settings.timezone.parse::<chrono_tz::Tz>().unwrap()).to_rfc3339()}),
            ),
            Command::ListCalendars(a) => {
                let ids = a.accounts.unwrap_or_else(|| {
                    self.settings
                        .accounts
                        .iter()
                        .map(|a| a.id.clone())
                        .collect()
                });
                let mut found = vec![];
                let mut errors = vec![];
                for id in ids {
                    let result = async { self.calendars(&id)?.list().await }.await;
                    match result {
                        Ok(calendars) => found.extend(calendars),
                        Err(e) => errors.push(json!({"account":id,"error":e})),
                    }
                }
                Ok(json!({"calendars":found,"complete":errors.is_empty(),"errors":errors}))
            }
            Command::GetCalendar(a) => Ok(json!(
                self.calendars(&a.calendar.account)?
                    .get(&a.calendar)
                    .await?
            )),
            Command::CreateCalendar(a) => Ok(json!(
                self.calendars(&a.spec.account)?
                    .create(a.spec, &self.settings.timezone)
                    .await?
            )),
            Command::UpdateCalendar(a) => Ok(json!(
                self.calendars(&a.calendar.account)?
                    .update(&a.calendar, a.changes)
                    .await?
            )),
            Command::DeleteCalendar(a) => {
                self.calendars(&a.calendar.account)?
                    .delete(&a.calendar)
                    .await
            }
            Command::GetItem(a) => self
                .items(&a.target.calendar.account)?
                .get(a.target)
                .await?
                .describe(),
            Command::CreateEvent(a) => self.create(a, ComponentKind::Event).await,
            Command::CreateTask(a) => self.create(a, ComponentKind::Task).await,
            Command::CreateJournal(a) => self.create(a, ComponentKind::Journal).await,
            Command::UpdateItem(a) => {
                let service = self.items(&a.target.calendar.account)?;
                let item = service.get(a.target).await?;
                item.check_revision(&a.revision)?;
                service
                    .save(item.edit(&a.changes, &a.scope)?)
                    .await?
                    .describe()
            }
            Command::DeleteItem(a) => {
                let service = self.items(&a.target.calendar.account)?;
                let item = service.get(a.target.clone()).await?;
                item.check_revision(&a.revision)?;
                match a.scope {
                    Scope::Series => service.delete(&a.target, &a.revision).await,
                    Scope::Occurrence { recurrence_id } => service
                        .save(item.exclude(&recurrence_id)?)
                        .await?
                        .describe(),
                }
            }
            Command::CompleteTask(a) => self.task_status(a, "COMPLETED").await,
            Command::ReopenTask(a) => self.task_status(a, "NEEDS-ACTION").await,
            Command::SearchEvents(a) => {
                self.search(
                    a.calendars,
                    SearchQuery::new(
                        ComponentKind::Event,
                        Some(a.period),
                        a.text,
                        None,
                        a.expand,
                        a.limit,
                    )?,
                )
                .await
            }
            Command::SearchTasks(a) => {
                self.search(
                    a.calendars,
                    SearchQuery::new(
                        ComponentKind::Task,
                        a.period,
                        a.text,
                        a.completed,
                        a.expand,
                        a.limit,
                    )?,
                )
                .await
            }
            Command::SearchJournals(a) => {
                self.search(
                    a.calendars,
                    SearchQuery::new(
                        ComponentKind::Journal,
                        a.period,
                        a.text,
                        None,
                        false,
                        a.limit,
                    )?,
                )
                .await
            }
            Command::SharingCapabilities(a) => {
                crate::sharing::Sharing {
                    calendars: self.calendars(&a.account)?,
                }
                .capabilities()
                .await
            }
            Command::ListShares(a) => {
                crate::sharing::Sharing {
                    calendars: self.calendars(&a.account)?,
                }
                .list()
                .await
            }
            Command::CreateShare(a) => {
                crate::sharing::Sharing {
                    calendars: self.calendars(&a.calendar.account)?,
                }
                .create(&a.calendar, &a.user, &a.path, a.access)
                .await
            }
            Command::UpdateShare(a) => {
                crate::sharing::Sharing {
                    calendars: self.calendars(&a.account)?,
                }
                .update(a)
                .await
            }
            Command::DeleteShare(a) => {
                crate::sharing::Sharing {
                    calendars: self.calendars(&a.account)?,
                }
                .delete(&a.path)
                .await
            }
            Command::FreeBusy(a) => {
                if a.calendars.is_empty() {
                    return Err(Error::input("Select at least one calendar"));
                }
                let mut results = vec![];
                let mut errors = vec![];
                let mut busy = vec![];
                for calendar in a.calendars {
                    match async {
                        crate::availability::query(
                            self.calendars(&calendar.account)?,
                            &calendar,
                            &a.period,
                        )
                        .await
                    }
                    .await
                    {
                        Ok(r) => {
                            results.push(r.value);
                            busy.extend(r.intervals);
                        }
                        Err(e) => errors.push(json!({"calendar":calendar,"error":e})),
                    }
                }
                Ok(
                    json!({"calendars":results,"period":a.period,"free":if errors.is_empty(){Some(crate::availability::free_intervals(&a.period,busy))}else{None},"complete":errors.is_empty(),"errors":errors}),
                )
            }
        }
    }
    async fn create(&self, a: CreateItem, kind: ComponentKind) -> Result<Value> {
        self.items(&a.calendar.account)?
            .create(a.calendar, kind, &a.uid, &a.fields)
            .await?
            .describe()
    }
    async fn task_status(&self, a: EditItem, status: &str) -> Result<Value> {
        let service = self.items(&a.target.calendar.account)?;
        let item = service.get(a.target).await?;
        item.check_revision(&a.revision)?;
        if !item.is_task() {
            return Err(Error::input("This operation requires a task"));
        }
        let patch = ItemPatch {
            status: Change::Set(status.into()),
            ..Default::default()
        };
        service.save(item.edit(&patch, &a.scope)?).await?.describe()
    }
    async fn search(&self, calendars: Vec<CalendarRef>, q: SearchQuery) -> Result<Value> {
        if calendars.is_empty() {
            return Err(Error::input("Select at least one calendar"));
        }
        let mut items = vec![];
        let mut errors = vec![];
        let mut truncated = false;
        for calendar in calendars {
            let result = async { self.items(&calendar.account)?.search(&calendar, &q).await }.await;
            match result {
                Ok(r) => {
                    items.extend(r.items);
                    errors.extend(r.errors);
                    truncated |= r.truncated;
                }
                Err(e) => errors.push(json!({"calendar":calendar,"error":e})),
            }
        }
        truncated |= items.len() > q.limit;
        items.truncate(q.limit);
        Ok(
            json!({"items":items,"errors":errors,"truncated":truncated,"complete":errors.is_empty()&&!truncated}),
        )
    }
}

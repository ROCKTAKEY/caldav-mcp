mod support;
use serde_json::json;
use support::Radicale;
#[tokio::test]
async fn calendar_task_lifecycle_and_conflict_detection() {
    let s = Radicale::start().await;
    let created = s
        .app
        .invoke(
            "create_calendar",
            json!({"spec":{"account":"alice","name":"Tasks"}}),
        )
        .await
        .unwrap();
    let calendar = created["ref"].clone();
    let item=s.app.invoke("create_task",json!({"calendar":calendar,"uid":"shopping","fields":{"summary":"お茶","due":{"kind":"date","value":"2026-11-09"}}})).await.unwrap();
    let duplicate = s
        .app
        .invoke(
            "create_task",
            json!({"calendar":calendar,"uid":"shopping","fields":{"summary":"duplicate"}}),
        )
        .await
        .unwrap_err();
    assert_eq!(duplicate.code, "conflict");
    let completed = s
        .app
        .invoke(
            "complete_task",
            json!({"ref":item["ref"],"revision":item["revision"],"scope":{"kind":"series"}}),
        )
        .await
        .unwrap();
    assert_eq!(completed["components"][0]["status"], "COMPLETED");
    let stale=s.app.invoke("update_item",json!({"ref":item["ref"],"revision":item["revision"],"scope":{"kind":"series"},"changes":{"summary":"stale"}})).await.unwrap_err();
    assert_eq!(stale.code, "conflict");
    let reopened=s.app.invoke("reopen_task",json!({"ref":completed["ref"],"revision":completed["revision"],"scope":{"kind":"series"}})).await.unwrap();
    assert_eq!(reopened["components"][0]["percent_complete"], "0");
    let found = s
        .app
        .invoke(
            "search_tasks",
            json!({"calendars":[calendar],"completed":false}),
        )
        .await
        .unwrap();
    assert_eq!(found["items"].as_array().unwrap().len(), 1);
    assert_eq!(found["complete"], true);
    let service = s.app.items("alice").unwrap();
    let target = serde_json::from_value(reopened["ref"].clone()).unwrap();
    let first = service.get(target).await.unwrap();
    let second = service
        .get(serde_json::from_value(reopened["ref"].clone()).unwrap())
        .await
        .unwrap();
    let patch = serde_json::from_value(json!({"summary":"Concurrent update"})).unwrap();
    let scope = caldav_mcp::items::models::Scope::Series;
    let latest = service
        .save(first.edit(&patch, &scope).unwrap())
        .await
        .unwrap()
        .describe()
        .unwrap();
    let conflict = service.save(second.edit(&patch, &scope).unwrap()).await;
    assert!(matches!(conflict,Err(ref e) if e.code=="conflict"));
    let reopened = s
        .app
        .invoke("get_item", json!({"ref":latest["ref"]}))
        .await
        .unwrap();
    s.app.invoke("delete_item",json!({"ref":reopened["ref"],"revision":reopened["revision"],"scope":{"kind":"series"}})).await.unwrap();
    s.app
        .invoke("delete_calendar", json!({"calendar":calendar}))
        .await
        .unwrap();
}

#[tokio::test]
async fn shares_and_free_busy_work_without_event_read_access() {
    let s = Radicale::start().await;
    let calendar = s
        .app
        .invoke(
            "create_calendar",
            json!({"spec":{"account":"alice","name":"Team"}}),
        )
        .await
        .unwrap()["ref"]
        .clone();
    s.app.invoke("create_event",json!({"calendar":calendar,"uid":"meeting","fields":{"summary":"Private title","start":{"kind":"utc","value":"2026-11-09T10:00:00Z"},"end":{"kind":"utc","value":"2026-11-09T11:00:00Z"}}})).await.unwrap();
    let capabilities = s
        .app
        .invoke("sharing_capabilities", json!({"account":"alice"}))
        .await
        .unwrap();
    assert_eq!(capabilities["enabled"], true);
    s.app
        .invoke(
            "create_share",
            json!({"calendar":calendar,"user":"bob","path":"/bob/team/","access":"read_write"}),
        )
        .await
        .unwrap();
    s.app
        .invoke(
            "update_share",
            json!({"account":"bob","path":"/bob/team/","enabled":true,"hidden":false}),
        )
        .await
        .unwrap();
    let shares = s
        .app
        .invoke("list_shares", json!({"account":"alice"}))
        .await
        .unwrap();
    assert_eq!(shares["shares"].as_array().unwrap().len(), 1);
    let shared = json!({"account":"bob","url":format!("{}bob/team/",s.base)});
    let items=s.app.invoke("search_events",json!({"calendars":[shared],"period":{"start":"2026-11-09T00:00:00Z","end":"2026-11-10T00:00:00Z"}})).await.unwrap();
    assert_eq!(items["items"].as_array().unwrap().len(), 1);
    s.app
        .invoke(
            "create_task",
            json!({"calendar":shared,"uid":"bob-task","fields":{"summary":"Shared task"}}),
        )
        .await
        .unwrap();
    s.app
        .invoke(
            "update_share",
            json!({"account":"alice","path":"/bob/team/","access":"free_busy"}),
        )
        .await
        .unwrap();
    let result=s.app.invoke("free_busy",json!({"calendars":[shared],"period":{"start":"2026-11-09T09:00:00Z","end":"2026-11-09T12:00:00Z"}})).await.unwrap();
    assert_eq!(result["complete"], true, "{result}");
    assert_eq!(result["free"].as_array().unwrap().len(), 2, "{result}");
    let denied = s
        .app
        .invoke("search_tasks", json!({"calendars":[shared]}))
        .await
        .unwrap();
    assert_eq!(denied["complete"], false);
    assert!(!denied["errors"].as_array().unwrap().is_empty());
    s.app
        .invoke(
            "delete_share",
            json!({"account":"alice","path":"/bob/team/"}),
        )
        .await
        .unwrap();
    s.app
        .invoke("delete_calendar", json!({"calendar":calendar}))
        .await
        .unwrap();
}

#[tokio::test]
async fn recurring_events_journals_metadata_and_partial_results() {
    let s = Radicale::start().await;
    let calendar = s
        .app
        .invoke(
            "create_calendar",
            json!({"spec":{"account":"alice","name":"Planning","timezone":"America/New_York"}}),
        )
        .await
        .unwrap()["ref"]
        .clone();
    let updated=s.app.invoke("update_calendar",json!({"calendar":calendar,"changes":{"name":"Work","description":"Notes","color":"#112233"}})).await.unwrap();
    assert_eq!(updated["name"], "Work");
    let info = s
        .app
        .invoke("get_calendar", json!({"calendar":calendar}))
        .await
        .unwrap();
    assert_eq!(info["name"], "Work");
    assert!(
        updated["timezone_icalendar"]
            .as_str()
            .unwrap()
            .contains("America/New_York")
    );
    let listed = s
        .app
        .invoke("list_calendars", json!({"accounts":["alice"]}))
        .await
        .unwrap();
    assert!(
        listed["calendars"]
            .as_array()
            .unwrap()
            .iter()
            .any(|v| v["ref"] == calendar)
    );
    let event=s.app.invoke("create_event",json!({"calendar":calendar,"uid":"weekly","fields":{
  "summary":"Weekly","start":{"kind":"zoned","value":"2026-10-26T10:00:00","timezone":"America/New_York"},"end":{"kind":"zoned","value":"2026-10-26T11:00:00","timezone":"America/New_York"},"recurrence_rule":"FREQ=WEEKLY;COUNT=5",
  "categories":["Work","Planning"],"classification":"PRIVATE","attachments":[{"uri":"https://example.org/agenda.pdf","media_type":"application/pdf"}],"related_to":[{"uid":"parent","relationship":"PARENT"}],"alarms":[{"description":"Meeting","trigger":"-PT15M"}]
 }})).await.unwrap();
    let changed=s.app.invoke("update_item",json!({"ref":event["ref"],"revision":event["revision"],"scope":{"kind":"occurrence","recurrence_id":{"kind":"utc","value":"2026-11-02T15:00:00Z"}},"changes":{"start":{"kind":"utc","value":"2026-11-02T16:00:00Z"},"end":{"kind":"utc","value":"2026-11-02T17:00:00Z"}}})).await.unwrap();
    assert!(
        changed["icalendar"]
            .as_str()
            .unwrap()
            .contains("BEGIN:VALARM")
    );
    assert!(
        changed["icalendar"]
            .as_str()
            .unwrap()
            .contains("https://example.org/agenda.pdf")
    );
    let search = json!({"calendars":[calendar],"period":{"start":"2026-10-26T00:00:00Z","end":"2026-11-24T00:00:00Z"}});
    let expanded = s.app.invoke("search_events", search.clone()).await.unwrap();
    assert_eq!(expanded["complete"], true, "{expanded}");
    let mut starts = expanded["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| {
            v["start"]["value"]
                .as_str()
                .unwrap()
                .parse::<chrono::DateTime<chrono::Utc>>()
                .unwrap()
        })
        .collect::<Vec<_>>();
    starts.sort();
    use chrono::Timelike;
    assert_eq!(
        starts.iter().map(|t| t.hour()).collect::<Vec<_>>(),
        vec![14, 16, 15, 15, 15]
    );
    let nonexistent=s.app.invoke("update_item",json!({"ref":changed["ref"],"revision":changed["revision"],"scope":{"kind":"occurrence","recurrence_id":{"kind":"utc","value":"2026-11-03T15:00:00Z"}},"changes":{"summary":"wrong"}})).await.unwrap_err();
    assert_eq!(nonexistent.code, "not_found");
    s.app.invoke("delete_item",json!({"ref":changed["ref"],"revision":changed["revision"],"scope":{"kind":"occurrence","recurrence_id":{"kind":"utc","value":"2026-11-09T15:00:00Z"}}})).await.unwrap();
    let expanded = s.app.invoke("search_events", search.clone()).await.unwrap();
    assert_eq!(expanded["items"].as_array().unwrap().len(), 4);
    let mut limited = search.clone();
    limited["limit"] = json!(1);
    let limited = s.app.invoke("search_events", limited).await.unwrap();
    assert_eq!(limited["truncated"], true);
    assert_eq!(limited["complete"], false);
    s.app.invoke("create_journal",json!({"calendar":calendar,"uid":"journal","fields":{"summary":"Notes","start":{"kind":"date","value":"2026-11-02"},"status":"FINAL"}})).await.unwrap();
    let journals = s
        .app
        .invoke("search_journals", json!({"calendars":[calendar]}))
        .await
        .unwrap();
    assert_eq!(journals["items"].as_array().unwrap().len(), 1);
    let bad = json!({"account":"missing","url":calendar["url"]});
    let partial = s
        .app
        .invoke(
            "free_busy",
            json!({"calendars":[calendar,bad],"period":search["period"]}),
        )
        .await
        .unwrap();
    assert_eq!(partial["complete"], false);
    assert!(partial["free"].is_null());
    assert_eq!(partial["calendars"].as_array().unwrap().len(), 1);
    s.app
        .invoke("delete_calendar", json!({"calendar":calendar}))
        .await
        .unwrap();
}

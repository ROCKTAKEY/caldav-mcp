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

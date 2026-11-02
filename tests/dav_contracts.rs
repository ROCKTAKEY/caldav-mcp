use caldav_mcp::{
    accounts::AccountConfig,
    calendars::{CalendarRef, Calendars},
    dav::{DavClient, escape},
    items::{
        models::{ComponentKind, ItemPatch},
        service::{Items, SearchQuery},
    },
};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn calendar(privileges: &str) -> String {
    format!(
        r#"<D:multistatus xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav"><D:response><D:href>/calendar/</D:href><D:propstat><D:prop><D:resourcetype><C:calendar/></D:resourcetype>{privileges}</D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response></D:multistatus>"#
    )
}

async fn server(
    responses: Vec<(&'static str, String)>,
) -> (DavClient, CalendarRef, tokio::task::JoinHandle<Vec<String>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/", listener.local_addr().unwrap());
    let peer = tokio::spawn(async move {
        let mut requests = vec![];
        for (status, body) in responses {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = vec![];
            loop {
                let mut buf = [0; 4096];
                let n = socket.read(&mut buf).await.unwrap();
                assert_ne!(n, 0);
                bytes.extend_from_slice(&buf[..n]);
                if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    let header = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
                    let len: usize = header
                        .lines()
                        .find_map(|s| s.strip_prefix("content-length:"))
                        .map(|s| s.trim().parse().unwrap())
                        .unwrap_or(0);
                    if bytes.len() >= end + 4 + len {
                        break;
                    }
                }
            }
            requests.push(String::from_utf8(bytes).unwrap());
            socket.write_all(format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
        }
        requests
    });
    let config: AccountConfig =
        serde_json::from_value(json!({"id":"test","url":base,"timeout_seconds":2})).unwrap();
    (
        DavClient::new(config).unwrap(),
        CalendarRef {
            account: "test".into(),
            url: format!("{base}calendar/"),
        },
        peer,
    )
}

#[tokio::test]
async fn utc_query_explicitly_overrides_calendar_timezone() {
    let (client, target, peer) = server(vec![
        ("207 Multi-Status", calendar("")),
        (
            "207 Multi-Status",
            r#"<D:multistatus xmlns:D="DAV:"/>"#.into(),
        ),
    ])
    .await;
    let items = Items {
        calendars: Calendars { client: &client },
        timezone: "UTC",
    };
    let query = SearchQuery::new(ComponentKind::Event, None, None, None, false, 10).unwrap();
    items.search(&target, &query).await.unwrap();
    let requests = peer.await.unwrap();
    let body = requests[1].split_once("\r\n\r\n").unwrap().1;
    let xml = roxmltree::Document::parse(body).unwrap();
    let timezone = xml
        .descendants()
        .find(|n| n.has_tag_name(("urn:ietf:params:xml:ns:caldav", "timezone")))
        .expect("UTC must override the calendar timezone");
    assert!(timezone.text().unwrap().contains("TZID:UTC"));
}

#[tokio::test]
async fn collection_delete_reports_failed_members() {
    let failure = r#"<D:multistatus xmlns:D="DAV:"><D:response><D:href>/calendar/locked.ics</D:href><D:status>HTTP/1.1 423 Locked</D:status></D:response></D:multistatus>"#;
    let (client, target, peer) = server(vec![
        ("207 Multi-Status", calendar("")),
        ("207 Multi-Status", failure.into()),
    ])
    .await;
    let result = Calendars { client: &client }.delete(&target).await;
    peer.await.unwrap();
    let error = result.expect_err("A failed child DELETE must not report success");
    assert_eq!(error.code, "calendar_delete_failed");
    assert_eq!(
        error.details["resources"][0]["href"],
        "/calendar/locked.ics"
    );
    assert_eq!(error.details["resources"][0]["status"], 423);
}

#[tokio::test]
async fn bind_only_privilege_allows_creating_members() {
    let privileges = "<D:current-user-privilege-set><D:privilege><D:bind/></D:privilege></D:current-user-privilege-set>";
    let (client, target, peer) = server(vec![
        ("207 Multi-Status", calendar(privileges)),
        ("201 Created", String::new()),
        ("404 Not Found", String::new()),
    ])
    .await;
    let result = Items {
        calendars: Calendars { client: &client },
        timezone: "UTC",
    }
    .create(target, ComponentKind::Task, "unique", &ItemPatch::default())
    .await;
    // Readback can fail on a create-only calendar, but the PUT must be allowed.
    assert_eq!(result.err().unwrap().code, "readback_failed");
    let requests = peer.await.unwrap();
    assert!(requests[1].starts_with("PUT "));
}

#[tokio::test]
async fn search_filters_each_occurrence_before_applying_limit() {
    let data = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//test//EN\r\nBEGIN:VEVENT\r\nUID:series\r\nDTSTART:20261102T100000Z\r\nSUMMARY:Special meeting\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nUID:series\r\nRECURRENCE-ID:20261103T100000Z\r\nDTSTART:20261103T100000Z\r\nSUMMARY:Standup\\, team\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
    let body = format!(
        r#"<D:multistatus xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav"><D:response><D:href>/calendar/series.ics</D:href><D:propstat><D:prop><D:getetag>"rev"</D:getetag><C:calendar-data>{}</C:calendar-data></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response></D:multistatus>"#,
        escape(data)
    );
    let (client, target, peer) = server(vec![
        ("207 Multi-Status", calendar("")),
        ("207 Multi-Status", body),
    ])
    .await;
    let query = SearchQuery::new(
        ComponentKind::Event,
        None,
        Some("STANDUP, TEAM".into()),
        None,
        false,
        1,
    )
    .unwrap();
    let result = Items {
        calendars: Calendars { client: &client },
        timezone: "UTC",
    }
    .search(&target, &query)
    .await
    .unwrap();
    peer.await.unwrap();
    assert!(result.errors.is_empty());
    assert_eq!(result.items.len(), 1);
    assert_eq!(result.items[0]["summary"], "Standup, team");
    assert!(!result.truncated);
}

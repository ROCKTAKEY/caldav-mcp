use caldav_mcp::{
    accounts::AccountConfig,
    calendars::{CalendarRef, Calendars},
    dav::DavClient,
    items::{
        models::{ComponentKind, ItemPatch},
        service::Items,
    },
};
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn exercise_write(put_response: Option<&'static str>) -> caldav_mcp::error::Error {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/", listener.local_addr().unwrap());
    let peer = tokio::spawn(async move {
        let prop = r#"<D:multistatus xmlns:D="DAV:" xmlns:C="urn:ietf:params:xml:ns:caldav"><D:response><D:href>/calendar/</D:href><D:propstat><D:prop><D:resourcetype><D:collection/><C:calendar/></D:resourcetype></D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response></D:multistatus>"#;
        let responses = if let Some(status) = put_response {
            vec![
                Some(("207 Multi-Status", prop)),
                Some((status, "")),
                Some(("404 Not Found", "")),
            ]
        } else {
            vec![Some(("207 Multi-Status", prop)), None]
        };
        for (i, response) in responses.into_iter().enumerate() {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut bytes = Vec::new();
            let header_end;
            loop {
                let mut buf = [0; 4096];
                let n = socket.read(&mut buf).await.unwrap();
                assert_ne!(n, 0);
                bytes.extend_from_slice(&buf[..n]);
                if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                    header_end = end + 4;
                    break;
                }
            }
            let header = String::from_utf8(bytes[..header_end].to_vec())
                .unwrap()
                .to_ascii_lowercase();
            let length = header
                .lines()
                .find_map(|l| l.strip_prefix("content-length:"))
                .map(|s| s.trim().parse::<usize>().unwrap())
                .unwrap_or(0);
            while bytes.len() < header_end + length {
                let mut buf = [0; 4096];
                let n = socket.read(&mut buf).await.unwrap();
                assert_ne!(n, 0);
                bytes.extend_from_slice(&buf[..n]);
            }
            if i == 1 {
                assert!(header.starts_with("put "));
                assert!(header.contains("if-none-match: *"));
            }
            if let Some((status, body)) = response {
                socket.write_all(format!("HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
            }
        }
    });
    let config: AccountConfig =
        serde_json::from_value(json!({"id":"test","url":base,"timeout_seconds":2})).unwrap();
    let client = DavClient::new(config).unwrap();
    let items = Items {
        calendars: Calendars { client: &client },
        timezone: "UTC",
    };
    let result = items
        .create(
            CalendarRef {
                account: "test".into(),
                url: format!("{base}calendar/"),
            },
            ComponentKind::Task,
            "unique",
            &ItemPatch::default(),
        )
        .await;
    peer.await.unwrap();
    match result {
        Err(e) => e,
        Ok(_) => panic!("Expected explicit failed/unknown write outcome"),
    }
}
#[tokio::test]
async fn successful_write_with_failed_readback_is_not_reported_as_failed_write() {
    let e = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        exercise_write(Some("201 Created")),
    )
    .await
    .unwrap();
    assert_eq!(e.code, "readback_failed");
    assert_eq!(e.details["write_succeeded"], true);
    assert!(
        e.details["target"]["url"]
            .as_str()
            .unwrap()
            .ends_with(".ics")
    );
}
#[tokio::test]
async fn dropped_write_response_is_unknown_and_not_automatically_retried() {
    let e = tokio::time::timeout(std::time::Duration::from_secs(10), exercise_write(None))
        .await
        .unwrap();
    assert_eq!(e.code, "outcome_unknown");
    assert!(e.details.contains_key("target"));
}

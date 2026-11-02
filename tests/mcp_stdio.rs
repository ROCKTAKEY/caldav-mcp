use rmcp::{ServiceExt, model::CallToolRequestParams, transport::TokioChildProcess};
use serde_json::json;
#[tokio::test]
async fn official_client_initializes_lists_and_calls_stdio_server() {
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config.toml");
    std::fs::write(
        &config,
        "timezone = 'Asia/Tokyo'\n[[accounts]]\nid = 'personal'\nurl = 'http://127.0.0.1:9/'\n",
    )
    .unwrap();
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_caldav-mcp"));
    command.arg("--config").arg(config);
    let transport = TokioChildProcess::new(command).unwrap();
    let client = ().serve(transport).await.unwrap();
    let tools = client.list_all_tools().await.unwrap();
    assert_eq!(tools.len(), 23);
    let result = client
        .call_tool(
            CallToolRequestParams::new("list_accounts")
                .with_arguments(json!({}).as_object().unwrap().clone()),
        )
        .await
        .unwrap();
    assert_eq!(result.is_error, Some(false));
    assert_eq!(
        result.structured_content.unwrap()["accounts"][0]["id"],
        "personal"
    );
    let invalid=client.call_tool(CallToolRequestParams::new("get_item").with_arguments(json!({"ref":{"calendar":{"account":"missing","url":"http://127.0.0.1:9/c/"},"url":"http://127.0.0.1:9/c/a.ics"}}).as_object().unwrap().clone())).await.unwrap();
    assert_eq!(invalid.is_error, Some(true));
    assert_eq!(
        invalid.structured_content.unwrap()["error"]["code"],
        "unknown_account"
    );
    client.cancel().await.unwrap();
}

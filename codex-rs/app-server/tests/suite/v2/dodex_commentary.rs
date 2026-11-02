//! Commentary checkpoints use the ordinary thread writer and fork lineage.

use anyhow::Result;
use app_test_support::{create_mock_responses_server_sequence, MockResponsesConfig, TestAppServer};
use codex_app_server_protocol::{ThreadCommentaryGetResponse, ThreadCommentarySetResponse, ThreadForkResponse, ThreadStartParams, ThreadStartResponse};
use serde_json::json;
use tempfile::TempDir;

#[tokio::test]
async fn commentary_checkpoints_survive_fork_without_following_later_parent_changes() -> Result<()> {
    let server = create_mock_responses_server_sequence(Vec::new()).await;
    let home = TempDir::new()?;
    MockResponsesConfig::new(&server.uri()).with_model("gpt-5.4").write(home.path())?;
    let mut app = TestAppServer::builder().with_codex_home(home.path()).without_managed_config().build_initialized().await?;
    let request = app.send_thread_start_request_with_auto_env(ThreadStartParams::default()).await?;
    let ThreadStartResponse { thread, .. } = app.read_response(request).await?;
    let parent_id = thread.id;
    let get = |id: &str| json!({ "threadId": id });
    let state = json!({ "version": 1, "enabled": true, "comments": [{ "note": "private draft" }] });
    let set = app.send_raw_request("thread/dodex/commentary/set", Some(json!({ "threadId": parent_id, "state": state }))).await?;
    let _: ThreadCommentarySetResponse = app.read_response(set).await?;
    let request = app.send_raw_request("thread/dodex/commentary/get", Some(get(&parent_id))).await?;
    let saved: ThreadCommentaryGetResponse = app.read_response(request).await?;
    assert_eq!(saved.state, Some(state.clone()));

    let request = app.send_raw_request("thread/fork", Some(get(&parent_id))).await?;
    let fork: ThreadForkResponse = app.read_response(request).await?;
    let child_id = fork.thread.id;
    let newer = json!({ "version": 1, "enabled": false, "comments": [{ "note": "parent after fork" }] });
    let set = app.send_raw_request("thread/dodex/commentary/set", Some(json!({ "threadId": parent_id, "state": newer }))).await?;
    let _: ThreadCommentarySetResponse = app.read_response(set).await?;
    let request = app.send_raw_request("thread/dodex/commentary/get", Some(get(&child_id))).await?;
    let inherited: ThreadCommentaryGetResponse = app.read_response(request).await?;
    assert_eq!(inherited.state, Some(state));
    Ok(())
}

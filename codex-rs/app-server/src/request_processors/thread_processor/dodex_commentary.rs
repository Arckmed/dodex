//! Session-local Commentary checkpoints, written through the thread's normal rollout writer.

use codex_app_server_protocol::{JSONRPCErrorError, ThreadCommentaryGetParams, ThreadCommentaryGetResponse, ThreadCommentarySetParams, ThreadCommentarySetResponse};
use codex_protocol::ThreadId;
use codex_protocol::protocol::{DodexCommentaryStateEvent, EventMsg};
use codex_rollout::RolloutItem;
use crate::error_code::{internal_error, invalid_request};
use super::ThreadRequestProcessor;

impl ThreadRequestProcessor {
    pub(crate) async fn thread_commentary_get(&self, params: ThreadCommentaryGetParams) -> Result<ThreadCommentaryGetResponse, JSONRPCErrorError> {
        let id = ThreadId::from_string(&params.thread_id).map_err(|error| invalid_request(format!("invalid thread id: {error}")))?;
        let thread = self.thread_manager.get_thread(id).await.map_err(|error| invalid_request(format!("thread is not loaded: {error}")))?;
        let state = thread.load_commentary_state().await.map_err(|error| internal_error(format!("failed to read Commentary state: {error}")))?;
        Ok(ThreadCommentaryGetResponse { state })
    }

    pub(crate) async fn thread_commentary_set(&self, params: ThreadCommentarySetParams) -> Result<ThreadCommentarySetResponse, JSONRPCErrorError> {
        let valid = params.state.get("version").and_then(serde_json::Value::as_u64) == Some(1)
            && params.state.get("enabled").is_some_and(serde_json::Value::is_boolean)
            && params.state.get("comments").is_some_and(serde_json::Value::is_array);
        if !valid { return Err(invalid_request("invalid Commentary state")); }
        let id = ThreadId::from_string(&params.thread_id).map_err(|error| invalid_request(format!("invalid thread id: {error}")))?;
        let thread = self.thread_manager.get_thread(id).await.map_err(|error| invalid_request(format!("thread is not loaded: {error}")))?;
        thread.append_rollout_items(&[RolloutItem::EventMsg(EventMsg::DodexCommentaryState(DodexCommentaryStateEvent { state: params.state }))]).await
            .map_err(|error| internal_error(format!("failed to append Commentary state: {error}")))?;
        thread.flush_rollout().await.map_err(|error| internal_error(format!("failed to persist Commentary state: {error}")))?;
        Ok(ThreadCommentarySetResponse {})
    }
}

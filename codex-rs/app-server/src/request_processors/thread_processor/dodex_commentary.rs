//! Session-local Commentary checkpoints, written through the thread's normal rollout writer.

use codex_app_server_protocol::{JSONRPCErrorError, ThreadCommentaryGetParams, ThreadCommentaryGetResponse, ThreadCommentarySetParams, ThreadCommentarySetResponse};
use codex_protocol::ThreadId;
use crate::error_code::{internal_error, invalid_request};
use super::ThreadRequestProcessor;

impl ThreadRequestProcessor {
    pub(crate) async fn thread_outline(&self, params: codex_app_server_protocol::ThreadOutlineParams) -> Result<codex_app_server_protocol::ThreadOutlineResponse, JSONRPCErrorError> {
        let id = ThreadId::from_string(&params.thread_id).map_err(|e| invalid_request(format!("invalid thread id: {e}")))?;
        let thread = self.thread_manager.get_thread(id).await.map_err(|e| invalid_request(format!("thread is not loaded: {e}")))?;
        let result = thread.outline_request(params.request).await.map_err(invalid_request)?;
        Ok(codex_app_server_protocol::ThreadOutlineResponse { result })
    }

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
        thread.save_commentary_state(params.state).await
            .map_err(|error| internal_error(format!("failed to save Commentary state: {error}")))?;
        Ok(ThreadCommentarySetResponse {})
    }
}

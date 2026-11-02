//! Read Commentary checkpoints through the same frozen lineage used for thread history.

use codex_protocol::protocol::{EventMsg, ThreadHistoryMode};
use codex_rollout::{ReverseJsonlScanner, RolloutItem, ScanOutcome};
use crate::{LoadThreadHistoryParams, ThreadStoreError, ThreadStoreResult};
use super::{LocalThreadStore, thread_rollout_resolver};

pub(super) async fn load(store: &LocalThreadStore, params: LoadThreadHistoryParams) -> ThreadStoreResult<Option<serde_json::Value>> {
    let resolved = if params.include_archived {
        thread_rollout_resolver::resolve_current_including_archived(store, params.thread_id).await?
    } else {
        thread_rollout_resolver::resolve_current(store, params.thread_id).await?
    };
    let Some(resolved) = resolved else { return Ok(None); };
    let meta = codex_rollout::read_session_meta_line(&resolved.path).await.map_err(|error| ThreadStoreError::Internal { message: format!("failed to read Commentary session metadata: {error}") })?;
    if meta.meta.history_mode != ThreadHistoryMode::Paginated {
        let history = store.load_history(params).await?;
        return Ok(history.items.into_iter().rev().find_map(checkpoint));
    }
    let lineage = store.resolve_rollout_lineage_for_history_page(params.thread_id).await?;
    let (stop, keep_running) = tokio::sync::oneshot::channel::<()>();
    let result = tokio::task::spawn_blocking(move || -> std::io::Result<Option<serde_json::Value>> {
        for segment in lineage.segments().iter().rev() {
            let file = codex_rollout::open_rollout_seekable_reader(&segment.rollout_path, || stop.is_closed())?;
            let mut scanner = match segment.end {
                Some(end) => ReverseJsonlScanner::new_at(file, end.end_byte_offset)?,
                None => ReverseJsonlScanner::new(file)?,
            };
            while let Some(outcome) = scanner.scan_next_rollout_line()? {
                let ScanOutcome::Parsed(line) = outcome else { continue; };
                if matches!(line.item, RolloutItem::SessionMeta(_)) { break; }
                let ordinal = line.ordinal.ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidData, "paginated Commentary history has a record without an ordinal"))?;
                if ordinal < segment.start_ordinal() { break; }
                if segment.end_ordinal().is_some_and(|end| ordinal >= end) { continue; }
                if let Some(state) = checkpoint(line.item) { return Ok(Some(state)); }
            }
        }
        Ok(None)
    }).await.map_err(|error| ThreadStoreError::Internal { message: format!("failed to join Commentary history scan: {error}") })?;
    drop(keep_running);
    result.map_err(|error| ThreadStoreError::Internal { message: format!("failed to read Commentary history: {error}") })
}

fn checkpoint(item: RolloutItem) -> Option<serde_json::Value> {
    match item {
        RolloutItem::EventMsg(EventMsg::DodexCommentaryState(event)) => Some(event.state),
        _ => None,
    }
}

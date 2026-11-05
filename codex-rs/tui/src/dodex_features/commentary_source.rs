//! Translate a persisted assistant item into a Commentary source reference.

use chrono::{DateTime, SecondsFormat, Utc};
use codex_app_server_protocol::{ThreadItem, ThreadItemPayload};

use super::commentary::Source;

pub(crate) fn assistant_source(item: &ThreadItem, turn_id: &str, display_offset: Option<usize>, display_row: Option<usize>) -> Option<Source> {
    let ThreadItemPayload::AgentMessage { id, .. } = &item.payload else { return None; };
    let timestamp = item.started_at_ms.or(item.completed_at_ms)
        .and_then(DateTime::<Utc>::from_timestamp_millis)
        .map(|time| time.to_rfc3339_opts(SecondsFormat::Millis, true));
    Some(Source { entry_id: id.clone(), turn_id: Some(turn_id.to_owned()), timestamp, display_offset, display_row, roadmap: None })
}

/// Use raw, completed assistant sources; rendered terminal rows are never source identity.
pub(crate) fn transcript_sources(cells: &[std::sync::Arc<dyn crate::history_cell::HistoryCell>]) -> Vec<(String, ThreadItem)> {
    let mut sources: Vec<(String, ThreadItem)> = Vec::new();
    for cell in cells {
        let Some(cell) = cell.as_any().downcast_ref::<crate::history_cell::AgentMarkdownCell>() else { continue; };
        let (Some(turn), Some(id)) = (cell.source_turn_id(), cell.source_item_id()) else { continue; };
        let item = ThreadItem::untimed(ThreadItemPayload::AgentMessage {
            id: id.to_owned(), text: cell.commentary_source_text().to_owned(), phase: None,
            memory_citation: None, delivery: None, questions: None,
        });
        if let Some((_, previous)) = sources.iter_mut().find(|(t, previous)| t == turn && previous.id() == id) {
            *previous = item;
        } else { sources.push((turn.to_owned(), item)); }
    }
    sources.reverse();
    sources
}

pub(crate) fn matches_submission(payload: &ThreadItemPayload, expected: &str) -> bool {
    matches!(payload, ThreadItemPayload::UserMessage { content, .. }
        if matches!(content.as_slice(), [codex_app_server_protocol::UserInput::Text { text, .. }] if text == expected))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_recorded_time_and_does_not_invent_missing_time() {
        let payload = ThreadItemPayload::AgentMessage { id: "item".into(), text: "text".into(), phase: None, memory_citation: None, delivery: None, questions: None };
        let mut item = ThreadItem::untimed(payload);
        let source = assistant_source(&item, "turn", Some(4), Some(2)).unwrap();
        assert_eq!(source.timestamp, None);
        assert_eq!(source.turn_id.as_deref(), Some("turn"));
        item.started_at_ms = Some(1_700_000_000_123);
        let source = assistant_source(&item, "turn", Some(4), Some(2)).unwrap();
        assert_eq!(source.timestamp.as_deref(), Some("2023-11-14T22:13:20.123Z"));
        assert_eq!(source.display_offset, Some(4));
        assert_eq!(source.display_row, Some(2));
    }
    #[test]
    fn temporary_sources_preserve_identity_and_original_markdown() {
        use crate::history_cell::{AgentMarkdownCell, HistoryCell};
        use std::sync::Arc;
        let cell = |turn: &str, id: &str, text: &str| -> Arc<dyn HistoryCell> {
            Arc::new(AgentMarkdownCell::new("rendered subset".into(), std::path::Path::new("."))
                .with_source_turn_id(Some(turn.into())).with_source_item_id(Some(id.into()))
                .with_copy_source(Some(text.into())))
        };
        let cells = vec![cell("one", "item", "partial"), cell("one", "item", "**complete**"), cell("two", "item", "other turn")];
        let sources = transcript_sources(&cells);
        assert_eq!(sources.len(), 2);
        assert_eq!(sources[1].0, "one");
        assert!(matches!(&sources[1].1.payload, ThreadItemPayload::AgentMessage { text, .. } if text == "**complete**"));
        assert_eq!(sources[0].0, "two");
        assert_eq!(assistant_source(&sources[0].1, "one", None, None).unwrap().timestamp, None);
    }

    #[test]
    fn receipt_requires_exact_user_message() {
        let user = |text: &str| ThreadItemPayload::UserMessage {
            id: "user".into(), client_id: None, content: vec![codex_app_server_protocol::UserInput::Text { text: text.into(), text_elements: Vec::new() }],
        };
        assert!(matches_submission(&user("batch-unique"), "batch-unique"));
        assert!(!matches_submission(&user("batch-unique-extra"), "batch-unique"));
        let assistant = ThreadItemPayload::AgentMessage { id: "item".into(), text: "batch-unique".into(), phase: None, memory_citation: None, delivery: None, questions: None };
        assert!(!matches_submission(&assistant, "batch-unique"));
    }

}

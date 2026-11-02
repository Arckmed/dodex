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
}

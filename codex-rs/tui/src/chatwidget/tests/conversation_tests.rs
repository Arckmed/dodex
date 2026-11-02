//! Visibility remains a local preference; no inference operation is sent by the toggle.
use super::*;

#[tokio::test]
async fn thoughts_default_hidden_and_toggle_is_local_and_persisted() {
    let (mut chat, _rx, mut ops) = make_chatwidget_manual(None).await;
    let id = ThreadId::new(); chat.thread_id = Some(id);
    chat.sync_conversation_view(Some(id));
    assert!(!chat.conversation_view.thoughts_visible);
    let effort = chat.current_reasoning_effort();
    while ops.try_recv().is_ok() {}
    chat.toggle_thoughts();
    assert!(chat.conversation_view.thoughts_visible);
    assert!(ops.try_recv().is_err());
    assert_eq!(chat.current_reasoning_effort(), effort);
    chat.sync_conversation_view(None); chat.sync_conversation_view(Some(id));
    assert!(chat.conversation_view.thoughts_visible);
    chat.toggle_thoughts();
    assert!(!chat.conversation_view.thoughts_visible);
    assert!(ops.try_recv().is_err());
}

#[tokio::test]
async fn hidden_thoughts_keep_generic_live_status_and_retain_received_text() {
    let (mut chat, _rx, mut ops) = make_chatwidget_manual(None).await;
    let id = ThreadId::new(); chat.thread_id = Some(id); chat.sync_conversation_view(Some(id));
    chat.on_task_started();
    chat.on_agent_reasoning_delta("**PRIVATE HEADER**\nRetained reasoning.".into());
    assert_eq!(chat.status_state.current_status.header, "Thinking");
    assert!(chat.reasoning_buffer.contains("Retained reasoning"));
    while ops.try_recv().is_ok() {}
    chat.toggle_thoughts();
    assert_eq!(chat.status_state.current_status.header, "Retained reasoning.");
    chat.toggle_thoughts();
    assert_eq!(chat.status_state.current_status.header, "Thinking");
    assert!(chat.reasoning_buffer.contains("Retained reasoning"));
    assert!(ops.try_recv().is_err());
}

#[tokio::test]
async fn default_statusline_preserves_requested_order_and_includes_both_timers() {
    let (mut chat, _rx, _ops) = make_chatwidget_manual(None).await;
    chat.local_settings.tui.status_line = None;
    assert_eq!(chat.configured_status_line_items(), DEFAULT_STATUS_LINE_ITEMS.map(str::to_string));
    let id = ThreadId::new(); chat.thread_id = Some(id); chat.sync_conversation_view(Some(id));
    assert!(chat.status_line_value_for_item(StatusLineItem::SessionCurrent).unwrap().starts_with("Current "));
    assert!(chat.status_line_value_for_item(StatusLineItem::SessionTotal).unwrap().starts_with("Total "));
}

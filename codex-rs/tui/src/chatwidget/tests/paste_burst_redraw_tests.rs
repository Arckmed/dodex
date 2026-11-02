//! Successful typing flushes render immediately; incomplete paste bursts still defer drawing.

use super::*;

#[tokio::test]
async fn paste_burst_redraw_does_not_starve_continuous_typing() {
    let (mut chat, _rx, mut ops) = make_chatwidget_manual(None).await;
    tokio::time::pause();
    let mut expected = String::new();
    for character in "continuous typing stays visible".chars() {
        chat.handle_key_event(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
        assert!(chat.is_in_paste_burst());
        tokio::time::advance(Duration::from_millis(12)).await;
        assert!(!chat.handle_paste_burst_tick(FrameRequester::test_dummy()));
        expected.push(character);
        assert_eq!(chat.composer_text_with_pending(), expected);
        tokio::time::advance(Duration::from_millis(28)).await;
    }
    assert!(ops.try_recv().is_err(), "editing must not submit a turn");
}

#[tokio::test]
async fn paste_burst_redraw_waits_for_multiline_paste_then_renders_the_flush() {
    let (mut chat, _rx, mut ops) = make_chatwidget_manual(None).await;
    tokio::time::pause();
    for character in "ab".chars() {
        chat.handle_key_event(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE));
    }
    chat.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    chat.handle_key_event(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
    assert!(chat.handle_paste_burst_tick(FrameRequester::test_dummy()));
    tokio::time::advance(Duration::from_millis(65)).await;
    assert!(!chat.handle_paste_burst_tick(FrameRequester::test_dummy()));
    assert_eq!(chat.composer_text_with_pending(), "ab\nc");
    assert!(ops.try_recv().is_err(), "paste newlines must not submit a turn");
}

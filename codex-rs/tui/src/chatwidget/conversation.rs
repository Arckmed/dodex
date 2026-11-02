//! Conversation-local UI state. Toggling thoughts never submits input or changes inference settings.
use super::*;

impl ChatWidget {
    pub(crate) fn sync_conversation_view(&mut self, thread: Option<ThreadId>) {
        let previous = self.conversation_view.thoughts_visible;
        match self.conversation_view.sync(self.config.codex_home.as_path(), thread, Instant::now()) {
            Ok(true) => { self.bump_active_cell_revision(); self.refresh_status_line(); }
            Ok(false) => {}
            Err(error) => self.add_error_message(format!("Could not load conversation UI state: {error}")),
        }
        if previous != self.conversation_view.thoughts_visible { self.refresh_thoughts_status(); }
    }

    pub(crate) fn toggle_thoughts(&mut self) {
        match self.conversation_view.toggle(Instant::now()) {
            Ok(()) => {
                self.bump_active_cell_revision();
                self.refresh_thoughts_status();
                self.request_redraw();
            }
            Err(error) => self.add_error_message(format!("Could not save thought visibility: {error}")),
        }
    }

    fn refresh_thoughts_status(&mut self) {
        if self.bottom_pane.is_task_running()
            && self.status_state.terminal_title_status_kind == TerminalTitleStatusKind::Thinking
        {
            // Preserve the existing safety, approval, compaction, and explicit-wait guards.
            self.restore_reasoning_status_header();
        }
    }

    pub(super) fn tick_conversation_view(&mut self) {
        if self.thread_id.is_some() {
            self.sync_conversation_view(self.thread_id);
        }
        let previous = self.conversation_view.thoughts_visible;
        if let Err(error) = self.conversation_view.tick(Instant::now()) {
            tracing::warn!(%error, "Could not save conversation UI state");
        }
        if previous != self.conversation_view.thoughts_visible {
            self.bump_active_cell_revision(); self.refresh_thoughts_status();
        }
        if self.conversation_view.elapsed(Instant::now()).is_some() {
            let timers = self.configured_status_line_items().iter().any(|item| item == "session-current" || item == "session-total");
            if timers && self.conversation_view.timers_changed(Instant::now()) { self.refresh_status_line(); }
            self.frame_requester.schedule_frame_in(Duration::from_secs(if timers { 1 } else { 10 }));
        }
    }
}

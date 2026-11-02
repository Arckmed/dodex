//! Thread-owned Commentary editing and checkpoint scheduling.

use super::*;
use crate::dodex_features::commentary::{Commentary, Draft, Source};
use crate::dodex_features::commentary_editor::{CommentaryEditor, EditorEvent};
use crate::dodex_features::commentary_source::assistant_source;
use uuid::Uuid;
use crate::dodex_features::commentary_outbox::{CommentaryOutbox, OutboxAction};
use crate::dodex_features::commentary_reader::{CommentaryReader, ReaderEvent};
use crate::dodex_features::commentary_review::{CommentaryReview, ReviewEvent};

pub(super) struct CommentarySession {
    state: Commentary,
    revision: Uuid,
    editor_id: Option<Uuid>,
    review_id: Option<Uuid>,
    review_submission: Option<String>,
    dirty: bool,
    return_to_outbox: bool,
}

impl App {
    pub(super) async fn flush_current_commentary_checkpoint(&mut self, server: &mut AppServerSession) -> bool {
        let Some(thread_id) = self.current_displayed_thread_id() else { return true; };
        let Some(session) = self.commentary_sessions.get(&thread_id) else { return true; };
        let revision = session.revision;
        self.save_commentary_checkpoint(server, thread_id, revision).await;
        self.commentary_sessions.get(&thread_id).is_none_or(|session| !session.dirty)
    }

    pub(super) async fn begin_commentary_draft(&mut self, server: &mut AppServerSession, thread_id: ThreadId, draft: Draft, replace: bool) {
        if self.current_displayed_thread_id() != Some(thread_id) { return; }
        if !self.unlock_commentary(server, thread_id, None, Some(draft.clone())).await { return; }
        let Some(session) = self.commentary_sessions.get_mut(&thread_id) else { return; };
        if session.state.draft.is_some() && !replace {
            self.chat_widget.show_selection_view(SelectionViewParams {
                title: Some("An unfinished comment exists".into()),
                items: vec![
                    SelectionItem { name: "Resume unfinished comment".into(), dismiss_on_select: true, actions: vec![Box::new(move |tx| tx.send(AppEvent::ResumeCommentaryDraft { thread_id }))], ..Default::default() },
                    SelectionItem { name: "Keep it and cancel".into(), dismiss_on_select: true, ..Default::default() },
                    SelectionItem { name: "Discard it and start this comment".into(), dismiss_on_select: true, actions: vec![Box::new(move |tx| tx.send(AppEvent::BeginCommentaryDraft { thread_id, draft: draft.clone(), replace: true }))], ..Default::default() },
                ], ..SelectionViewParams::confirmation()
            }); return;
        }
        if replace { session.state.draft = None; }
        if let Err(error) = session.state.start_draft(draft) { self.chat_widget.add_error_message(error.into()); return; }
        session.revision = Uuid::new_v4(); session.dirty = true;
        self.edit_commentary_draft(server, thread_id).await;
    }

    async fn unlock_commentary(&mut self, server: &mut AppServerSession, thread_id: ThreadId, action: Option<OutboxAction>, draft: Option<Draft>) -> bool {
        if !self.reconcile_commentary_receipt(server, thread_id).await { return false; }
        let Some(submission) = self.commentary_sessions.get(&thread_id).and_then(|session| session.state.submission.as_ref()) else { return true; };
        if !self.chat_widget.commentary_can_send() { self.chat_widget.add_error_message("Wait for the submitted batch and queued messages to settle before editing.".into()); return false; }
        let expected_id = submission.id.clone();
        self.chat_widget.show_selection_view(SelectionViewParams {
            title: Some("Reset unacknowledged send?".into()),
            subtitle: Some("The batch was not found in saved history. Keep its comments and clear the send marker so they can be edited?".into()),
            items: vec![SelectionItem { name: "Cancel".into(), dismiss_on_select: true, ..Default::default() }, SelectionItem {
                name: "Keep comments and reset marker".into(), dismiss_on_select: true,
                actions: vec![Box::new(move |tx| tx.send(AppEvent::ResetCommentarySubmission { thread_id, expected_id: expected_id.clone(), action: action.clone(), draft: draft.clone() }))], ..Default::default()
            }], ..SelectionViewParams::confirmation()
        }); false
    }

    pub(super) async fn reset_commentary_submission(&mut self, server: &mut AppServerSession, thread_id: ThreadId, expected_id: &str, action: Option<OutboxAction>, draft: Option<Draft>) {
        if self.current_displayed_thread_id() != Some(thread_id) || !self.chat_widget.commentary_can_send() { return; }
        if !self.reconcile_commentary_receipt(server, thread_id).await { return; }
        let Some(session) = self.commentary_sessions.get_mut(&thread_id) else { return; };
        // Confirmation applies to precisely the marker that was shown; an acknowledged batch is never reset.
        if !session.state.reset_submission(expected_id) { return; }
        session.dirty = true; session.revision = Uuid::new_v4(); let revision = session.revision;
        self.save_commentary_checkpoint(server, thread_id, revision).await;
        if self.commentary_sessions[&thread_id].dirty { return; }
        if let Some(draft) = draft { self.begin_commentary_draft(server, thread_id, draft, false).await; }
        else { match action { Some(action) => self.handle_commentary_outbox(server, thread_id, action).await, None => self.open_commentary_selection(server).await } }
    }

    pub(super) fn commentary_passages(&self) -> Vec<crate::dodex_features::commentary_highlights::Passage> {
        use crate::dodex_features::commentary_highlights::Passage;
        let Some(session) = self.current_displayed_thread_id().and_then(|id| self.commentary_sessions.get(&id)).filter(|session| session.state.enabled) else { return Vec::new(); };
        session.state.comments.iter().filter_map(Passage::comment).chain(session.state.draft.iter().filter_map(Passage::draft)).collect()
    }
    async fn browse_commentary_sources(&mut self, server: &mut AppServerSession) {
        let Some(thread_id) = self.current_displayed_thread_id() else { return; };
        if !self.load_commentary_session(server, thread_id).await { return; }
        let sources = match server.thread_commentary_sources(thread_id).await {
            Ok(sources) => sources,
            Err(error) => { self.chat_widget.add_error_message(format!("Could not browse assistant sources: {error}")); return; }
        };
        if self.current_displayed_thread_id() != Some(thread_id) { return; }
        let mut items = Vec::new();
        for (turn_id, item) in sources {
            let codex_app_server_protocol::ThreadItemPayload::AgentMessage { text, .. } = &item.payload else { continue; };
            if text.trim().is_empty() { continue; }
            let Some(source) = assistant_source(&item, &turn_id, None, None) else { continue; };
            let name = text.split_whitespace().collect::<Vec<_>>().join(" ").chars().filter(|ch| !ch.is_control()).take(160).collect::<String>();
            let description = format!("{} · {}", source.entry_id, source.timestamp.as_deref().unwrap_or("timestamp unavailable"));
            items.push(SelectionItem { name, description: Some(description), dismiss_on_select: true,
                actions: vec![Box::new(move |tx| tx.send(AppEvent::OpenCommentarySource { thread_id, source: source.clone() }))], ..Default::default()
            });
        }
        if items.is_empty() { self.chat_widget.add_info_message("No earlier assistant text messages are available on this branch.".into(), None); return; }
        self.chat_widget.show_selection_view(SelectionViewParams { title: Some("Earlier assistant responses".into()), items,
            on_cancel: Some(Box::new(move |tx| tx.send(AppEvent::CommentaryReader { thread_id, event: ReaderEvent::Close }))), ..Default::default() });
    }

    pub(super) async fn flush_commentary_checkpoints(&mut self, server: &mut AppServerSession) -> bool {
        let pending = self.commentary_sessions.iter().filter(|(_, session)| session.dirty).map(|(id, session)| (*id, session.revision)).collect::<Vec<_>>();
        for (thread_id, revision) in pending { self.save_commentary_checkpoint(server, thread_id, revision).await; }
        self.commentary_sessions.values().all(|session| !session.dirty)
    }

    pub(super) async fn sync_commentary_thread(&mut self, server: &mut AppServerSession) {
        let current = self.current_displayed_thread_id();
        if self.commentary_active_thread == current { return; }
        self.flush_commentary_checkpoints(server).await;
        self.commentary_active_thread = current;
        self.commentary_load_id = None;
        if self.reconnect.offline { return; }
        let Some(thread_id) = current else { return; };
        // Clean cached state can be reloaded after a switch; retain failed writes in memory.
        if self.commentary_sessions.get(&thread_id).is_some_and(|session| !session.dirty) { self.commentary_sessions.remove(&thread_id); }
        if self.commentary_sessions.contains_key(&thread_id) { return; }
        let handle = server.request_handle(); let tx = self.app_event_tx.clone();
        let load_id = Uuid::new_v4(); self.commentary_load_id = Some(load_id);
        tokio::spawn(async move {
            let response: Result<codex_app_server_protocol::ThreadCommentaryGetResponse, _> = handle.request_typed(ClientRequest::ThreadCommentaryGet {
                request_id: codex_app_server_protocol::RequestId::String(format!("commentary-load-{}", Uuid::new_v4())),
                params: codex_app_server_protocol::ThreadCommentaryGetParams { thread_id: thread_id.to_string() },
            }).await;
            tx.send(AppEvent::CommentaryLoaded { thread_id, load_id, result: response.map(|response| response.state).map_err(|error| error.to_string()) });
        });
    }

    pub(super) async fn commentary_loaded(&mut self, server: &mut AppServerSession, thread_id: ThreadId, load_id: Uuid, result: Result<Option<serde_json::Value>, String>) {
        if self.commentary_load_id != Some(load_id) || self.commentary_active_thread != Some(thread_id) || self.commentary_sessions.contains_key(&thread_id) { return; }
        self.commentary_load_id = None;
        let state = match result {
            Ok(None) => Commentary::default(),
            Ok(Some(value)) => match Commentary::from_saved(&value) { Some(state) => state, None => { self.chat_widget.add_error_message("Saved Commentary is damaged or unsupported and has been retained unchanged.".into()); return; } },
            Err(error) => { self.chat_widget.add_error_message(format!("Could not restore Commentary: {error}")); return; }
        };
        self.commentary_sessions.insert(thread_id, CommentarySession { state, revision: Uuid::new_v4(), editor_id: None, review_id: None, review_submission: None, dirty: false, return_to_outbox: false });
        self.reconcile_commentary_receipt(server, thread_id).await;
    }

    pub(super) async fn open_commentary_review(&mut self, server: &mut AppServerSession, thread_id: ThreadId, retry_confirmed: bool) {
        if self.current_displayed_thread_id() != Some(thread_id) || !self.load_commentary_session(server, thread_id).await { return; }
        if !self.reconcile_commentary_receipt(server, thread_id).await { return; }
        if !self.chat_widget.commentary_can_send() { self.chat_widget.add_error_message("Wait until the current turn and queued messages finish before reviewing a Commentary submission.".into()); return; }
        let session = self.commentary_sessions.get_mut(&thread_id).expect("session loaded");
        if session.state.comments.is_empty() && session.state.final_consideration.as_deref().is_none_or(|text| text.trim().is_empty()) { self.chat_widget.add_error_message("No comments or final consideration to send.".into()); return; }
        if session.state.submission.is_some() && !retry_confirmed {
            self.chat_widget.show_selection_view(SelectionViewParams {
                title: Some("Retry the unacknowledged batch?".into()),
                subtitle: Some("The exact batch was not found in saved history. Review and retry its original text?".into()),
                items: vec![SelectionItem { name: "Cancel".into(), dismiss_on_select: true, ..Default::default() }, SelectionItem {
                    name: "Review original batch".into(), dismiss_on_select: true,
                    actions: vec![Box::new(move |tx| tx.send(AppEvent::OpenCommentaryReview { thread_id, retry_confirmed: true }))], ..Default::default()
                }], ..SelectionViewParams::confirmation()
            }); return;
        }
        let review_id = Uuid::new_v4(); session.review_id = Some(review_id);
        session.review_submission = session.state.submission.as_ref().map(|submission| submission.id.clone());
        let (preview, final_note, locked) = match &session.state.submission {
            Some(submission) => (submission.text.clone(), submission.final_consideration.clone().unwrap_or_default(), true),
            None => (crate::dodex_features::commentary::batch_text(&session.state.comments, &review_id.to_string(), None), session.state.final_consideration.clone().unwrap_or_default(), false),
        };
        let tx = self.app_event_tx.clone();
        self.chat_widget.show_bottom_pane_view(Box::new(CommentaryReview::new(preview, &final_note, locked, &self.keymap, Box::new(move |event| tx.send(AppEvent::CommentaryReview { thread_id, review_id, event })))));
    }

    pub(super) async fn handle_commentary_review(&mut self, server: &mut AppServerSession, thread_id: ThreadId, review_id: Uuid, event: ReviewEvent) {
        let Some(session) = self.commentary_sessions.get_mut(&thread_id).filter(|session| session.review_id == Some(review_id)) else { return; };
        if session.review_submission.as_ref().is_some_and(|id| session.state.submission.as_ref().is_none_or(|submission| &submission.id != id)) { return; }
        let retrying = session.review_submission.is_some();
        let (note, send, close) = match event { ReviewEvent::Changed(note) => (note, false, false), ReviewEvent::Close(note) => (note, false, true), ReviewEvent::Send(note) => (note, true, true) };
        if session.state.submission.is_none() { session.state.final_consideration = Some(note); }
        session.dirty = true; session.revision = Uuid::new_v4();
        let revision = session.revision;
        if !close {
            let tx = self.app_event_tx.clone();
            tokio::spawn(async move { tokio::time::sleep(Duration::from_millis(600)).await; tx.send(AppEvent::SaveCommentaryCheckpoint { thread_id, revision }); });
            return;
        }
        session.review_id = None;
        self.save_commentary_checkpoint(server, thread_id, revision).await;
        if !send {
            if self.current_displayed_thread_id() == Some(thread_id) && self.commentary_sessions[&thread_id].return_to_outbox { self.open_commentary_outbox(server).await; }
            return;
        }
        if self.current_displayed_thread_id() != Some(thread_id) || !self.chat_widget.commentary_can_send() { self.chat_widget.add_error_message("The conversation became busy. Commentary remains saved.".into()); return; }
        if !self.reconcile_commentary_receipt(server, thread_id).await { return; }
        let session = self.commentary_sessions.get_mut(&thread_id).expect("session loaded");
        if session.dirty { return; }
        if retrying && session.state.submission.is_none() { return; }
        if session.state.submission.is_none() {
            if let Err(error) = session.state.prepare_submission(review_id.to_string()) { self.chat_widget.add_error_message(error.into()); return; }
            session.dirty = true; session.revision = Uuid::new_v4();
        }
        let revision = session.revision;
        self.save_commentary_checkpoint(server, thread_id, revision).await;
        let session = &self.commentary_sessions[&thread_id];
        if session.dirty { return; }
        let text = session.state.submission.as_ref().expect("submission frozen").text.clone();
        if !self.chat_widget.submit_commentary_batch(text) { self.chat_widget.add_error_message("The Commentary batch was not submitted. Its comments and send marker remain saved.".into()); return; }
        self.commentary_sessions.get_mut(&thread_id).expect("session loaded").return_to_outbox = false;
        self.schedule_commentary_receipt(thread_id, 0);
    }

    fn schedule_commentary_receipt(&self, thread_id: ThreadId, attempt: u8) {
        let tx = self.app_event_tx.clone();
        tokio::spawn(async move { tokio::time::sleep(Duration::from_secs(1)).await; tx.send(AppEvent::CheckCommentaryReceipt { thread_id, attempt }); });
    }

    pub(super) async fn check_commentary_receipt(&mut self, server: &mut AppServerSession, thread_id: ThreadId, attempt: u8) {
        if !self.reconcile_commentary_receipt(server, thread_id).await { return; }
        if attempt < 29 && self.commentary_sessions.get(&thread_id).is_some_and(|session| session.state.submission.is_some()) { self.schedule_commentary_receipt(thread_id, attempt + 1); }
    }

    async fn reconcile_commentary_receipt(&mut self, server: &mut AppServerSession, thread_id: ThreadId) -> bool {
        let Some(text) = self.commentary_sessions.get(&thread_id).and_then(|session| session.state.submission.as_ref()).map(|submission| submission.text.clone()) else { return true; };
        match server.thread_commentary_received(thread_id, &text).await {
            Ok(false) => true,
            Ok(true) => {
                let session = self.commentary_sessions.get_mut(&thread_id).expect("session loaded");
                session.state.acknowledge(&text); session.dirty = true; session.revision = Uuid::new_v4();
                let revision = session.revision;
                self.save_commentary_checkpoint(server, thread_id, revision).await; true
            }
            Err(error) => { self.chat_widget.add_error_message(format!("Could not confirm the Commentary receipt; comments retained: {error}")); false }
        }
    }

    pub(super) fn commentary_enabled(&self) -> bool {
        self.current_displayed_thread_id().and_then(|id| self.commentary_sessions.get(&id)).is_some_and(|session| session.state.enabled)
    }

    pub(super) async fn handle_commentary_command(&mut self, server: &mut AppServerSession, argument: &str) {
        match argument.trim().to_ascii_lowercase().as_str() {
            "add" => self.open_commentary_selection(server).await,
            "review" => self.open_commentary_outbox(server).await,
            "browse" => self.browse_commentary_sources(server).await,
            "send" => { if let Some(thread_id) = self.current_displayed_thread_id() { self.open_commentary_review(server, thread_id, false).await; } }
            mode @ ("" | "on" | "off") => {
                let Some(thread_id) = self.current_displayed_thread_id() else { return; };
                if !self.load_commentary_session(server, thread_id).await { return; }
                let session = self.commentary_sessions.get_mut(&thread_id).expect("session loaded");
                session.state.enabled = match mode { "on" => true, "off" => false, _ => !session.state.enabled };
                let enabled = session.state.enabled;
                session.revision = Uuid::new_v4(); session.dirty = true;
                let revision = session.revision;
                self.save_commentary_checkpoint(server, thread_id, revision).await;
                self.chat_widget.add_info_message(format!("Commentary {}. Pending annotations are retained.", if enabled { "on" } else { "off" }), None);
            }
            _ => self.chat_widget.add_error_message("Usage: /commentary [on|off|add|browse|review|send] or /comments".into()),
        }
    }

    pub(super) async fn open_commentary_outbox(&mut self, server: &mut AppServerSession) {
        let Some(thread_id) = self.current_displayed_thread_id() else { return; };
        if !self.chat_widget.no_modal_or_popup_active() || !self.load_commentary_session(server, thread_id).await { return; }
        if !self.reconcile_commentary_receipt(server, thread_id).await { return; }
        if self.current_displayed_thread_id() != Some(thread_id) { return; }
        self.commentary_sessions.get_mut(&thread_id).expect("session loaded").return_to_outbox = true;
        let tx = self.app_event_tx.clone();
        let state = self.commentary_sessions[&thread_id].state.clone();
        self.chat_widget.show_bottom_pane_view(Box::new(CommentaryOutbox::new(state, Box::new(move |action| tx.send(AppEvent::CommentaryOutboxAction { thread_id, action })))));
    }

    pub(super) async fn handle_commentary_outbox(&mut self, server: &mut AppServerSession, thread_id: ThreadId, action: OutboxAction) {
        if self.current_displayed_thread_id() != Some(thread_id) { return; }
        if matches!(&action, OutboxAction::Resume | OutboxAction::Edit(_) | OutboxAction::Delete(_) | OutboxAction::DiscardDraft)
            && !self.unlock_commentary(server, thread_id, Some(action.clone()), None).await { return; }
        let Some(session) = self.commentary_sessions.get_mut(&thread_id) else { return; };
        match action {
            OutboxAction::Close => session.return_to_outbox = false,
            OutboxAction::Resume => { if session.state.draft.is_some() { self.edit_commentary_draft(server, thread_id).await; } }
            OutboxAction::Edit(id) => {
                if session.state.submission.is_some() { self.chat_widget.add_error_message("Resolve the unacknowledged Commentary submission before editing.".into()); return; }
                if let Some(draft) = &session.state.draft {
                    if draft.comment_id.as_deref() == Some(id.as_str()) { self.edit_commentary_draft(server, thread_id).await; }
                    else { self.chat_widget.add_error_message("Resume or discard the unfinished comment first (R / X).".into()); self.open_commentary_outbox(server).await; }
                    return;
                }
                let Some(comment) = session.state.comments.iter().find(|comment| comment.id == id).cloned() else { return; };
                let draft = Draft { comment_id: Some(comment.id), source: comment.source, quotation: comment.quotation, note: comment.note, hue: comment.hue };
                self.begin_commentary_draft(server, thread_id, draft, false).await;
            }
            OutboxAction::Delete(id) => self.confirm_commentary_removal(thread_id, Some(id)),
            OutboxAction::DiscardDraft => self.confirm_commentary_removal(thread_id, None),
            OutboxAction::Add => self.browse_commentary_sources(server).await,
            OutboxAction::Source(id) => self.open_commentary_source(server, thread_id, &id).await,
            OutboxAction::Review => self.open_commentary_review(server, thread_id, false).await,
        }
    }

    async fn open_commentary_source(&mut self, server: &mut AppServerSession, thread_id: ThreadId, id: &str) {
        let Some(session) = self.commentary_sessions.get(&thread_id) else { return; };
        let Some(comment) = session.state.comments.iter().find(|comment| comment.id == id).cloned() else { return; };
        if let Some(index) = self.transcript_cells.iter().position(|cell| cell.as_any().downcast_ref::<crate::history_cell::AgentMarkdownCell>().is_some_and(|cell| {
            cell.source_item_id() == Some(comment.source.entry_id.as_str()) && cell.source_turn_id() == comment.source.turn_id.as_deref()
        })).filter(|_| self.local_settings.transcript_mode.is_owned()) {
            self.commentary_sessions.get_mut(&thread_id).expect("session loaded").return_to_outbox = false;
            self.transcript_view.jump_to_entry_offset(&self.transcript_cells, index, comment.source.display_offset.unwrap_or(0));
            return;
        }
        self.show_commentary_source(server, thread_id, comment.source).await;
    }

    pub(super) async fn show_commentary_source(&mut self, server: &mut AppServerSession, thread_id: ThreadId, source: Source) {
        if self.current_displayed_thread_id() != Some(thread_id) { return; }
        let Some(session) = self.commentary_sessions.get(&thread_id) else { return; };
        let automatic = session.state.enabled;
        let Some(turn_id) = source.turn_id.as_deref() else { self.chat_widget.add_error_message("This source does not identify an assistant turn.".into()); return; };
        let item = match server.thread_commentary_source(thread_id, turn_id, &source.entry_id).await {
            Ok(Some(item)) => item,
            Ok(None) => { self.chat_widget.add_error_message("The saved assistant source is unavailable.".into()); return; }
            Err(error) => { self.chat_widget.add_error_message(format!("Could not read the saved source: {error}")); return; }
        };
        if self.current_displayed_thread_id() != Some(thread_id) { return; }
        let codex_app_server_protocol::ThreadItemPayload::AgentMessage { text, .. } = item.payload else { return; };
        let tx = self.app_event_tx.clone();
        let mut reader = CommentaryReader::new(source, text, &self.config.cwd, &self.keymap, automatic, Box::new(move |event| tx.send(AppEvent::CommentaryReader { thread_id, event })));
        reader.set_passages(self.commentary_passages());
        self.chat_widget.show_bottom_pane_view(Box::new(reader));
    }

    pub(super) async fn handle_commentary_reader(&mut self, server: &mut AppServerSession, thread_id: ThreadId, event: ReaderEvent) {
        if self.current_displayed_thread_id() != Some(thread_id) { return; }
        match event {
            ReaderEvent::Close => { if self.commentary_sessions.get(&thread_id).is_some_and(|session| session.return_to_outbox) { self.open_commentary_outbox(server).await; } }
            ReaderEvent::Quote { source, quotation } => {
                let Some(session) = self.commentary_sessions.get_mut(&thread_id) else { return; };
                let used = session.state.comments.iter().filter_map(|comment| comment.hue).collect::<Vec<_>>();
                let hue = crate::dodex_features::commentary_colors::choose_hue(&used, crate::terminal_palette::effective_stdout_color_level(), rand::random::<f64>());
                self.begin_commentary_draft(server, thread_id, Draft { comment_id: None, source, quotation, note: String::new(), hue: Some(hue) }, false).await;
            }
        }
    }

    fn confirm_commentary_removal(&mut self, thread_id: ThreadId, comment_id: Option<String>) {
        let deleting_comment = comment_id.is_some();
        self.chat_widget.show_selection_view(SelectionViewParams {
            title: Some(if deleting_comment { "Delete this comment?" } else { "Discard the unfinished draft?" }.into()),
            items: vec![
                SelectionItem { name: "Cancel".into(), dismiss_on_select: true, actions: vec![Box::new(move |tx| tx.send(AppEvent::CommentaryReader { thread_id, event: ReaderEvent::Close }))], ..Default::default() },
                SelectionItem { name: if deleting_comment { "Delete comment" } else { "Discard draft" }.into(), dismiss_on_select: true,
                    actions: vec![Box::new(move |tx| tx.send(AppEvent::ConfirmCommentaryRemoval { thread_id, comment_id: comment_id.clone() }))], ..Default::default() },
            ],
            on_cancel: Some(Box::new(move |tx| tx.send(AppEvent::CommentaryReader { thread_id, event: ReaderEvent::Close }))),
            ..SelectionViewParams::confirmation()
        });
    }

    pub(super) async fn remove_commentary_annotation(&mut self, server: &mut AppServerSession, thread_id: ThreadId, comment_id: Option<String>) {
        if self.current_displayed_thread_id() != Some(thread_id) { return; }
        let Some(session) = self.commentary_sessions.get_mut(&thread_id) else { return; };
        if session.state.submission.is_some() { self.chat_widget.add_error_message("Resolve the unacknowledged Commentary submission before deleting annotations.".into()); return; }
        if let Some(id) = comment_id {
            if session.state.draft.as_ref().is_some_and(|draft| draft.comment_id.as_ref() == Some(&id)) {
                session.state.draft = None;
            }
            if let Err(error) = session.state.delete_comment(&id) { self.chat_widget.add_error_message(error.into()); return; }
        } else { session.state.draft = None; }
        session.revision = Uuid::new_v4(); session.dirty = true;
        let revision = session.revision;
        self.save_commentary_checkpoint(server, thread_id, revision).await;
        self.open_commentary_outbox(server).await;
    }

    async fn load_commentary_session(&mut self, server: &mut AppServerSession, thread_id: ThreadId) -> bool {
        if !self.commentary_sessions.contains_key(&thread_id) {
            let state = match server.thread_commentary_get(thread_id).await {
                Ok(None) => Commentary::default(),
                Ok(Some(value)) => match Commentary::from_saved(&value) {
                    Some(state) => state,
                    None => { self.chat_widget.add_error_message("The saved Commentary checkpoint is damaged or unsupported. It has been retained unchanged.".into()); return false; }
                },
                Err(error) => { self.chat_widget.add_error_message(format!("Could not load Commentary: {error}")); return false; }
            };
            self.commentary_sessions.insert(thread_id, CommentarySession { state, revision: Uuid::new_v4(), editor_id: None, review_id: None, review_submission: None, dirty: false, return_to_outbox: false });
        }
        true
    }

    pub(super) async fn open_commentary_selection(&mut self, server: &mut AppServerSession) {
        let Some(thread_id) = self.current_displayed_thread_id() else { return; };
        if !self.chat_widget.no_modal_or_popup_active() { return; }
        if !self.load_commentary_session(server, thread_id).await { return; }
        self.commentary_sessions.get_mut(&thread_id).expect("session loaded").return_to_outbox = false;
        if self.current_displayed_thread_id() != Some(thread_id) { return; }
        if !self.unlock_commentary(server, thread_id, None, None).await { return; }
        {
            let selection = match self.transcript_view.selected_assistant() {
                Ok(selection) => selection,
                Err(error) => { self.chat_widget.add_error_message(format!("Select a passage in one completed assistant response before pressing Alt+C ({error:?}).")); return; }
            };
            let item = match server.thread_commentary_source(thread_id, &selection.source_turn_id, &selection.source_item_id).await {
                Ok(Some(item)) => item,
                Ok(None) => { self.chat_widget.add_error_message("The selected assistant response could not be found in saved history.".into()); return; }
                Err(error) => { self.chat_widget.add_error_message(format!("Could not read the Commentary source: {error}")); return; }
            };
            if self.current_displayed_thread_id() != Some(thread_id) { return; }
            let Some(source) = assistant_source(&item, &selection.source_turn_id, Some(selection.display_range.start), Some(selection.display_row)) else { return; };
            self.transcript_view.end_select_mode();
            self.transcript_view.end_selection(&self.transcript_cells);
            self.transcript_view.cancel_search();
            let session = self.commentary_sessions.get_mut(&thread_id).expect("session loaded");
            let hue = crate::dodex_features::commentary_colors::choose_hue(&session.state.comments.iter().filter_map(|comment| comment.hue).collect::<Vec<_>>(), crate::terminal_palette::effective_stdout_color_level(), rand::random::<f64>());
            self.begin_commentary_draft(server, thread_id, Draft { comment_id: None, source, quotation: selection.quotation, note: String::new(), hue: Some(hue) }, false).await;
        }
    }

    pub(super) async fn edit_commentary_draft(&mut self, server: &mut AppServerSession, thread_id: ThreadId) {
        if self.current_displayed_thread_id() != Some(thread_id) || self.commentary_sessions.get(&thread_id).is_none_or(|session| session.state.draft.is_none()) { return; }
        if self.commentary_sessions[&thread_id].state.submission.is_some() { self.chat_widget.add_error_message("Resolve the unacknowledged Commentary submission before editing.".into()); return; }
        let revision = self.commentary_sessions[&thread_id].revision;
        self.save_commentary_checkpoint(server, thread_id, revision).await;
        let session = self.commentary_sessions.get_mut(&thread_id).expect("session loaded");
        if session.dirty { return; }
        let draft = session.state.draft.as_ref().expect("draft created");
        let editor_id = Uuid::new_v4();
        session.editor_id = Some(editor_id);
        let tx = self.app_event_tx.clone();
        let source = format!("Source {} · {}", draft.source.entry_id, draft.source.timestamp.as_deref().unwrap_or("timestamp unavailable"));
        let mut view = CommentaryEditor::new(source, draft.quotation.clone(), &draft.note, draft.comment_id.is_some(), Box::new(move |event| tx.send(AppEvent::CommentaryEditor { thread_id, editor_id, event })));
        view.set_keymap_bindings(&self.keymap);
        self.chat_widget.show_bottom_pane_view(Box::new(view));
    }

    pub(super) async fn handle_commentary_editor(&mut self, server: &mut AppServerSession, thread_id: ThreadId, editor_id: Uuid, event: EditorEvent) {
        let Some(session) = self.commentary_sessions.get_mut(&thread_id).filter(|session| session.editor_id == Some(editor_id)) else { return; };
        let closing = !matches!(event, EditorEvent::Changed(_));
        let mut notification = match &event {
            EditorEvent::Save(_) => "Comment saved locally. Alt+Shift+C reviews pending comments.",
            EditorEvent::Keep(_) => "Unfinished comment kept as a draft. Nothing sent.",
            EditorEvent::Cancel => "Comment cancelled. Unsaved changes discarded. Nothing sent.",
            EditorEvent::Changed(_) => "",
        };
        match event {
            EditorEvent::Changed(note) | EditorEvent::Keep(note) => { if let Some(draft) = &mut session.state.draft { draft.note = note; } }
            EditorEvent::Save(note) => {
                if let Some(draft) = &mut session.state.draft { draft.note = note; }
                if let Err(error) = session.state.save_draft(Uuid::new_v4().to_string(), chrono::Utc::now().to_rfc3339()) {
                    self.chat_widget.add_error_message(error.into());
                    notification = "The comment could not be saved; its unfinished draft was retained. Nothing sent.";
                }
            }
            EditorEvent::Cancel => session.state.draft = None,
        }
        session.dirty = true;
        session.revision = Uuid::new_v4();
        let revision = session.revision;
        if closing {
            session.editor_id = None;
            self.save_commentary_checkpoint(server, thread_id, revision).await;
            if self.current_displayed_thread_id() == Some(thread_id) && !self.commentary_sessions[&thread_id].dirty {
                self.chat_widget.add_info_message(notification.into(), None);
                if self.commentary_sessions[&thread_id].return_to_outbox { self.open_commentary_outbox(server).await; }
            }
        } else {
            let tx = self.app_event_tx.clone();
            tokio::spawn(async move {
                tokio::time::sleep(Duration::from_millis(600)).await;
                tx.send(AppEvent::SaveCommentaryCheckpoint { thread_id, revision });
            });
        }
    }

    pub(super) async fn save_commentary_checkpoint(&mut self, server: &mut AppServerSession, thread_id: ThreadId, revision: Uuid) {
        let Some(session) = self.commentary_sessions.get(&thread_id).filter(|session| session.revision == revision && session.dirty) else { return; };
        let state = match serde_json::to_value(&session.state) {
            Ok(state) => state,
            Err(error) => { self.chat_widget.add_error_message(format!("Could not encode Commentary: {error}")); return; }
        };
        match server.thread_commentary_set(thread_id, state).await {
            Ok(()) => { if let Some(session) = self.commentary_sessions.get_mut(&thread_id).filter(|session| session.revision == revision) { session.dirty = false; } }
            Err(error) => self.chat_widget.add_error_message(format!("Could not save Commentary; the draft remains in memory: {error}")),
        }
    }
}

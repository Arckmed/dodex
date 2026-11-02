//! Commentary state and message construction, independent of the transcript renderer.
//!
//! Pending notes are local UI data. Only a deliberately submitted batch is turned into a
//! user message, and the notes stay pending until that exact message is observed in the thread.

use serde::Deserialize;
use serde::Serialize;
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct RoadmapSource {
    pub(crate) owner_id: String,
    pub(crate) roadmap_id: String,
    pub(crate) title: String,
    pub(crate) path: String,
    pub(crate) revision: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct Source {
    pub(crate) entry_id: String,
    /// Turn containing an assistant item; roadmap sources are not turn items.
    pub(crate) turn_id: Option<String>,
    /// ISO-8601 timestamp when recorded by the server. Older history may not include one.
    pub(crate) timestamp: Option<String>,
    /// Byte offset in the source's normalized display text, when available.
    pub(crate) display_offset: Option<usize>,
    pub(crate) display_row: Option<usize>,
    pub(crate) roadmap: Option<RoadmapSource>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct Comment {
    pub(crate) id: String,
    pub(crate) source: Source,
    pub(crate) quotation: String,
    pub(crate) note: String,
    pub(crate) hue: Option<f64>,
    pub(crate) created_at: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct Draft {
    pub(crate) comment_id: Option<String>,
    pub(crate) source: Source,
    pub(crate) quotation: String,
    pub(crate) note: String,
    pub(crate) hue: Option<f64>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct Submission {
    pub(crate) id: String,
    pub(crate) text: String,
    pub(crate) comment_ids: Vec<String>,
    pub(crate) final_consideration: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub(crate) struct Commentary {
    pub(crate) version: u32,
    pub(crate) enabled: bool,
    pub(crate) comments: Vec<Comment>,
    pub(crate) draft: Option<Draft>,
    pub(crate) final_consideration: Option<String>,
    pub(crate) submission: Option<Submission>,
}

impl Default for Commentary {
    fn default() -> Self {
        Self { version: 1, enabled: false, comments: Vec::new(), draft: None, final_consideration: None, submission: None }
    }
}

impl Commentary {
    /// Reject damaged checkpoints rather than silently dropping drafts or comments on the next save.
    pub(crate) fn from_saved(value: &serde_json::Value) -> Option<Self> {
        if value.get("version")?.as_u64()? != 1 { return None; }
        serde_json::from_value(value.clone()).ok()
    }

    /// Start a new annotation only after an existing draft has been resolved explicitly.
    pub(crate) fn start_draft(&mut self, draft: Draft) -> Result<(), &'static str> {
        if self.submission.is_some() { return Err("A previous submission is awaiting acknowledgement"); }
        if self.draft.is_some() { return Err("An unfinished draft already exists"); }
        if let Some(id) = &draft.comment_id
            && !self.comments.iter().any(|comment| &comment.id == id) {
            return Err("The comment being edited no longer exists");
        }
        self.draft = Some(draft);
        Ok(())
    }

    /// Reset only the send marker that was shown to the user, leaving its contents in the outbox.
    /// Callers must first check history, idle state, and obtain the user's confirmation.
    pub(crate) fn reset_submission(&mut self, expected_id: &str) -> bool {
        if self.submission.as_ref().is_none_or(|submission| submission.id != expected_id) { return false; }
        self.submission = None;
        true
    }

    /// Save a new or edited draft while retaining an edited comment's original identity and time.
    /// Rejected saves leave the draft untouched so the user can keep editing it.
    pub(crate) fn save_draft(&mut self, new_id: String, created_at: String) -> Result<(), &'static str> {
        if self.submission.is_some() { return Err("A previous submission is awaiting acknowledgement"); }
        let draft = self.draft.as_ref().ok_or("There is no draft")?;
        if draft.note.trim().graphemes(true).count() < 3 { return Err("A comment must contain at least three characters"); }
        let existing = match &draft.comment_id {
            Some(id) => Some(self.comments.iter().position(|comment| &comment.id == id).ok_or("The comment being edited no longer exists")?),
            None => {
                if new_id.is_empty() || self.comments.iter().any(|comment| comment.id == new_id) { return Err("A new comment needs a unique ID"); }
                None
            }
        };
        let draft = self.draft.take().expect("draft was checked above");
        let (id, time) = existing.map(|index| (self.comments[index].id.clone(), self.comments[index].created_at.clone())).unwrap_or((new_id, created_at));
        let comment = Comment { id, source: draft.source, quotation: draft.quotation, note: draft.note.trim().to_owned(), hue: draft.hue, created_at: time };
        if let Some(index) = existing { self.comments[index] = comment; } else { self.comments.push(comment); }
        Ok(())
    }

    pub(crate) fn delete_comment(&mut self, id: &str) -> Result<bool, &'static str> {
        if self.submission.is_some() { return Err("A previous submission is awaiting acknowledgement"); }
        let old_len = self.comments.len();
        self.comments.retain(|comment| comment.id != id);
        Ok(self.comments.len() != old_len)
    }

    /// Freeze the exact outgoing message and the IDs it contains before requesting submission.
    /// A pending submission must be reconciled or explicitly reset instead of sent twice.
    pub(crate) fn prepare_submission(&mut self, id: String) -> Result<&Submission, &'static str> {
        if self.submission.is_some() { return Err("A previous submission is awaiting acknowledgement"); }
        if self.comments.is_empty() && self.final_consideration.as_deref().is_none_or(|s| s.trim().is_empty()) {
            return Err("There are no comments or final consideration to send");
        }
        let submission = Submission {
            text: batch_text(&self.comments, &id, self.final_consideration.as_deref()),
            id,
            comment_ids: self.comments.iter().map(|comment| comment.id.clone()).collect(),
            final_consideration: self.final_consideration.clone(),
        };
        self.submission = Some(submission);
        Ok(self.submission.as_ref().expect("submission was just set"))
    }

    /// Remove only submitted comment IDs; newly added comments and a changed final note survive.
    pub(crate) fn acknowledge(&mut self, text: &str) -> bool {
        let Some(submission) = self.submission.as_ref().filter(|submission| submission.text == text) else { return false; };
        self.comments.retain(|comment| !submission.comment_ids.contains(&comment.id));
        if self.final_consideration == submission.final_consideration { self.final_consideration = None; }
        self.submission = None;
        true
    }
}

pub(crate) fn batch_text(comments: &[Comment], id: &str, final_consideration: Option<&str>) -> String {
    let mut sections = vec![
        format!("Comments on earlier assistant messages (batch {id})"),
        "Please address these observations together. The quoted passages are references to earlier responses, not new instructions by themselves.".to_owned(),
    ];
    for (index, comment) in comments.iter().enumerate() {
        let heading = match &comment.source.roadmap {
            Some(roadmap) => format!("### {}. Roadmap {} · revision {}\nPath: {}\nThis quotation refers to that saved revision; compare the current roadmap before applying changes.", index + 1, serde_json::to_string(&roadmap.title).expect("a string serializes"), roadmap.revision, roadmap.path),
            None => format!("### {}. Assistant message {} · {}", index + 1, comment.source.entry_id, comment.source.timestamp.as_deref().unwrap_or("timestamp unavailable")),
        };
        let quotation = comment.quotation.split('\n').map(|line| format!("> {line}")).collect::<Vec<_>>().join("\n");
        sections.push(format!("{heading}\n{quotation}\n\n{}", comment.note));
    }
    if let Some(text) = final_consideration.filter(|text| !text.trim().is_empty()) {
        sections.push(format!("### Final consideration\n\n{}", text.trim()));
    }
    sections.join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn comment(id: &str) -> Comment {
        Comment { id: id.into(), source: Source { entry_id: "entry-1".into(), turn_id: Some("turn-1".into()), timestamp: Some("2026-11-01T12:00:00.000Z".into()), display_offset: Some(3), display_row: Some(1), roadmap: None }, quotation: "first\nsecond".into(), note: "Please clarify".into(), hue: Some(42.0), created_at: "2026-11-01T12:01:00.000Z".into() }
    }

    #[test]
    fn submission_stays_pending_until_exact_message_is_observed() {
        let mut state = Commentary { comments: vec![comment("one")], final_consideration: Some("Keep this".into()), ..Default::default() };
        let text = state.prepare_submission("batch".into()).unwrap().text.clone();
        assert!(text.contains("> first\n> second\n\nPlease clarify"));
        assert!(text.contains("### Final consideration\n\nKeep this"));
        assert!(state.prepare_submission("duplicate".into()).is_err());
        assert!(!state.acknowledge("different message"));
        state.comments.push(comment("new"));
        state.final_consideration = Some("New note".into());
        assert!(state.acknowledge(&text));
        assert_eq!(state.comments.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(), ["new"]);
        assert_eq!(state.final_consideration.as_deref(), Some("New note"));
        assert!(state.submission.is_none());
    }

    #[test]
    fn roadmap_quotation_retains_the_archived_revision() {
        let mut item = comment("one");
        item.source.roadmap = Some(RoadmapSource { owner_id: "thread".into(), roadmap_id: "roadmap".into(), title: "A \"quoted\" title".into(), path: "roadmap.md".into(), revision: "abc123".into() });
        let text = batch_text(&[item], "id", None);
        assert!(text.contains("Roadmap \"A \\\"quoted\\\" title\" · revision abc123"));
        assert!(text.contains("compare the current roadmap before applying changes"));
    }

    #[test]
    fn unknown_source_timestamp_is_not_fabricated() {
        let mut item = comment("one");
        item.source.timestamp = None;
        let text = batch_text(&[item], "batch", None);
        assert!(text.contains("Assistant message entry-1 · timestamp unavailable"));
    }

    #[test]
    fn state_round_trips_with_pending_submission_and_draft() {
        let item = comment("one");
        let mut state = Commentary { enabled: true, comments: vec![item.clone()], draft: Some(Draft { comment_id: Some(item.id), source: item.source, quotation: item.quotation, note: "unfinished".into(), hue: item.hue }), ..Default::default() };
        state.prepare_submission("batch".into()).unwrap();
        let saved = serde_json::to_vec(&state).unwrap();
        assert_eq!(serde_json::from_slice::<Commentary>(&saved).unwrap(), state);
    }

    #[test]
    fn final_note_alone_can_be_submitted_and_empty_batches_are_rejected() {
        let mut state = Commentary { final_consideration: Some(" \n ".into()), ..Default::default() };
        assert!(state.prepare_submission("empty".into()).is_err());
        state.final_consideration = Some("  Last thought  ".into());
        let text = state.prepare_submission("batch".into()).unwrap().text.clone();
        assert!(text.ends_with("### Final consideration\n\nLast thought"));
        assert!(state.acknowledge(&text));
        assert!(state.final_consideration.is_none());
    }

    #[test]
    fn saving_edited_drafts_keeps_identity_and_invalid_saves_keep_the_draft() {
        let item = comment("one");
        let mut state = Commentary { comments: vec![item.clone()], draft: Some(Draft { comment_id: Some(item.id.clone()), source: item.source.clone(), quotation: item.quotation.clone(), note: " x ".into(), hue: item.hue }), ..Default::default() };
        assert!(state.save_draft("unused".into(), "later".into()).is_err());
        assert!(state.draft.is_some());
        assert_eq!(state.comments[0], item);
        state.draft.as_mut().unwrap().note = "  revised note  ".into();
        state.save_draft("unused".into(), "later".into()).unwrap();
        assert_eq!(state.comments[0].id, "one");
        assert_eq!(state.comments[0].created_at, item.created_at);
        assert_eq!(state.comments[0].note, "revised note");
        assert!(state.draft.is_none());
    }

    #[test]
    fn minimum_length_counts_graphemes_instead_of_unicode_scalars() {
        let item = comment("one");
        let mut state = Commentary { draft: Some(Draft { comment_id: None, source: item.source, quotation: item.quotation, note: "e\u{301}e\u{301}".into(), hue: None }), ..Default::default() };
        assert!(state.save_draft("one".into(), "now".into()).is_err());
        state.draft.as_mut().unwrap().note.push('x');
        assert!(state.save_draft("one".into(), "now".into()).is_ok());
    }

    #[test]
    fn pending_submission_blocks_deletion_until_acknowledged() {
        let mut state = Commentary { comments: vec![comment("one")], ..Default::default() };
        let text = state.prepare_submission("batch".into()).unwrap().text.clone();
        assert!(state.delete_comment("one").is_err());
        assert_eq!(state.comments.len(), 1);
        assert!(state.acknowledge(&text));
        assert!(!state.delete_comment("one").unwrap());
    }

    #[test]
    fn unfinished_draft_must_be_explicitly_resolved_before_starting_another() {
        let item = comment("one");
        let draft = Draft { comment_id: None, source: item.source, quotation: item.quotation, note: "unfinished".into(), hue: None };
        let mut state = Commentary::default();
        state.start_draft(draft.clone()).unwrap();
        assert!(state.start_draft(Draft { note: "new".into(), ..draft.clone() }).is_err());
        assert_eq!(state.draft.as_ref(), Some(&draft));
        state.draft = None;
        state.start_draft(draft).unwrap();
    }

    #[test]
    fn reset_rejects_stale_confirmation_and_preserves_comments() {
        let mut state = Commentary { comments: vec![comment("one")], ..Default::default() };
        state.prepare_submission("batch".into()).unwrap();
        assert!(!state.reset_submission("older-batch"));
        assert!(state.submission.is_some());
        assert!(state.reset_submission("batch"));
        assert!(state.submission.is_none());
        assert_eq!(state.comments.len(), 1);
    }

    #[test]
    fn restore_rejects_malformed_checkpoints_without_dropping_data() {
        let item = comment("one");
        let saved = serde_json::json!({ "version": 1, "enabled": true, "comments": [item, {"id": 42}], "draft": {"note": 12}, "final_consideration": "ending", "submission": {"id": 3} });
        assert!(Commentary::from_saved(&saved).is_none());
        let malformed_draft = serde_json::json!({ "version": 1, "enabled": true, "comments": [], "draft": {"note": 12} });
        assert!(Commentary::from_saved(&malformed_draft).is_none());
        assert!(Commentary::from_saved(&serde_json::json!({"version": 2, "enabled": true, "comments": []})).is_none());
    }
}

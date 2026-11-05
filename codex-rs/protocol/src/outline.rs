//! Conversation-owned Outline documents. Revisions are immutable; deletion retains quotation sources.
use std::collections::BTreeMap;
use serde::{Deserialize, Serialize};

pub const MAX_BYTES: usize = 1024 * 1024;
pub const REFERENCE_CHARS: usize = 7000;
pub const EMPTY_REVISION: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
pub const GUIDANCE: &str = "Outline mode is active. Treat the active document's agreed requirements and decisions as the canonical implementation reference, ahead of older conversational proposals. System/developer and applicable repository instructions, and the user's latest explicit instructions, take precedence. Proposals and open questions are not approved requirements. Resolve material contradictions with the user. Work normally; this is not a planning gate. Consult the supplied reference before choosing implementation work. Do not reread content already supplied. Read omitted sections only when needed for the next decision or edit. Maintain the document at meaningful checkpoints: agreed decisions, scope changes, verified completion, or blockers that change the plan. Consolidate related changes in one precise outline_update; do not rewrite unchanged sections or update merely because another response occurred. Use expected_revision and reread on conflicts. Historical annotations refer to their original revision: compare current text before applying them. No background model calls are needed. Use Outline tools for the central document; arbitrary filesystem writes do not participate in revision checks.";

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct OutlineState {
    pub version: u32,
    pub generation: u64,
    pub enabled: bool,
    pub active_id: Option<String>,
    pub documents: Vec<OutlineDocument>,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct OutlineDocument {
    pub id: String,
    pub title: String,
    pub source_id: String,
    pub revision: String,
    pub deleted: bool,
    pub revisions: BTreeMap<String, OutlineRevision>,
}
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct OutlineRevision { pub text: String, pub saved_at: String }

impl OutlineState {
    pub fn empty() -> Self { Self { version: 1, ..Default::default() } }
    pub fn active(&self) -> Option<&OutlineDocument> { self.documents.iter().find(|d| Some(&d.id) == self.active_id.as_ref() && !d.deleted) }
    pub fn validate(&self) -> Result<(), String> {
        if self.version != 1 { return Err("Unsupported Outline state; retained unchanged".into()); }
        let mut ids = std::collections::BTreeSet::new();
        for doc in &self.documents {
            if !valid_id(&doc.id) || !ids.insert(&doc.id) || !doc.revisions.contains_key(&doc.revision) { return Err("Invalid Outline document".into()); }
            for rev in doc.revisions.values() { if rev.text.len() > MAX_BYTES { return Err("Outline exceeds 1 MiB".into()); } }
        }
        if self.enabled && self.active().is_none() { return Err("Active Outline is unavailable".into()); }
        Ok(())
    }
}
pub fn valid_id(id: &str) -> bool { !id.is_empty() && id.len() <= 128 && id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_') }
/// Historical bodies live in the checkpoint where they were current, rather than being copied forever.
pub fn revision_from_checkpoint(state: &serde_json::Value, id: &str, revision: &str) -> Option<String> {
    let doc = state.get("documents")?.as_array()?.iter().find(|doc| doc.get("id").and_then(serde_json::Value::as_str) == Some(id))?;
    let text = doc.get("revisions")?.get(revision)?.get("text")?.as_str()?;
    (!text.is_empty() || revision == EMPTY_REVISION).then(|| text.to_owned())
}
impl OutlineDocument {
    pub fn text(&self) -> &str { self.revisions.get(&self.revision).map_or("", |r| r.text.as_str()) }
    /// Local, deterministic excerpt selection; never invokes a model or edits the source document.
    pub fn reference(&self, prompt: &str) -> String {
        let header = format!("ACTIVE OUTLINE REFERENCE\nTitle: {}\nID: {}\nRevision: {}\n", self.title, self.id, self.revision);
        let text = self.text();
        if text.chars().count() <= REFERENCE_CHARS { return format!("{header}\nFull current document:\n{text}"); }
        let lines: Vec<_> = text.lines().collect();
        let mut outline = String::new();
        for (i, line) in lines.iter().enumerate().filter(|(_, l)| l.trim_start().starts_with('#')) {
            outline.push_str(&format!("L{}: {}\n", i + 1, line));
            if outline.chars().count() >= 1400 { break; }
        }
        outline = outline.chars().take(1400).collect();
        let words: Vec<_> = prompt.split(|c: char| !c.is_alphanumeric()).filter(|w| w.chars().count() >= 4).take(80).map(str::to_lowercase).collect();
        let mut blocks = Vec::new(); let mut start = 0;
        for end in 1..=lines.len() {
            if end != lines.len() && !lines[end].trim_start().starts_with('#') { continue; }
            let body = lines[start..end].join("\n"); let lower = body.to_lowercase();
            let score = words.iter().filter(|w| lower.contains(w.as_str())).count() + usize::from(lower.contains("agreed") || lower.contains("requirement")) * 4;
            blocks.push((score, start, end, body)); start = end;
        }
        blocks.sort_by_key(|(score, start, _, _)| (std::cmp::Reverse(*score), *start));
        let mut remaining = REFERENCE_CHARS.saturating_sub(outline.chars().count() + 350);
        let mut excerpts = String::new();
        for (_, start, end, body) in blocks {
            if remaining < 100 { break; }
            let excerpt: String = format!("L{}–{}:\n{}\n", start + 1, end, body).chars().take(remaining.min(2400)).collect();
            remaining = remaining.saturating_sub(excerpt.chars().count()); excerpts.push_str(&excerpt);
        }
        format!("{header}\nPARTIAL DOCUMENT. Use outline_read for omitted sections before decisions or edits that depend on them.\nHeadings:\n{outline}\nRelevant excerpts (may end within a section):\n{excerpts}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn document(text: String) -> OutlineDocument {
        OutlineDocument { id: "doc".into(), title: "Input controls".into(), source_id: "message".into(), revision: "revision".into(), deleted: false,
            revisions: [("revision".into(), OutlineRevision { text, saved_at: "2026-11-04".into() })].into() }
    }
    #[test]
    fn outline_small_reference_is_exact_and_not_partial() {
        let text = "# Agreed\n- Preserve clipboard selection.\n\n界 😀\n";
        let reference = document(text.into()).reference("clipboard");
        assert!(reference.ends_with(text)); assert!(reference.contains("Full current document")); assert!(!reference.contains("PARTIAL DOCUMENT"));
    }
    #[test]
    fn outline_large_reference_is_bounded_unicode_safe_and_prioritizes_requirements() {
        let doc = document(format!("# Background\n{}\n# Agreed requirements\n- Keep clipboard selection on failure.\n# Appendix\n{}", "界".repeat(8000), "😀".repeat(8000)));
        let reference = doc.reference("clipboard");
        assert!(reference.contains("PARTIAL DOCUMENT")); assert!(reference.contains("Keep clipboard selection on failure"));
        assert!(reference.chars().count() <= REFERENCE_CHARS + 500);
        assert_eq!(doc.text().matches('界').count(), 8000, "selection must not mutate the stored document");
    }
    #[test]
    fn outline_deleted_documents_keep_revisions_but_cannot_be_active() {
        let mut state = OutlineState::empty(); let mut doc = document("old quotation".into()); doc.deleted = true;
        state.documents.push(doc); state.active_id = Some("doc".into());
        assert!(state.validate().is_ok()); assert!(state.active().is_none());
        state.enabled = true; assert!(state.validate().is_err());
        assert_eq!(state.documents[0].revisions["revision"].text, "old quotation");
    }
    #[test]
    fn outline_rejects_duplicate_ids_and_unknown_state_versions() {
        let mut state = OutlineState::empty(); state.documents = vec![document("a".into()),document("b".into())];
        assert!(state.validate().is_err()); state.documents.clear(); state.version = 2; assert!(state.validate().is_err());
    }
}

//! Outline's sole mutation authority, shared by model tools and app-server clients.
use std::sync::atomic::{AtomicBool, Ordering};
use std::path::{Path, PathBuf};
use codex_protocol::outline::{OutlineDocument, OutlineRevision, OutlineState, MAX_BYTES};
use codex_protocol::protocol::{DodexCommentaryStateEvent, EventMsg};
use codex_rollout::RolloutItem;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;
use crate::session::session::Session;

#[derive(Default)]
pub(crate) struct OutlineService { lock: Mutex<()>, pub(crate) enabled: AtomicBool, warned: AtomicBool, pub(crate) active_id: std::sync::RwLock<Option<String>>, cache: Mutex<Option<(u64, OutlineState)>> }
fn hash(text: &str) -> String { format!("{:x}", Sha256::digest(text.as_bytes())) }
fn now() -> String { chrono::Utc::now().to_rfc3339() }
fn field<'a>(value: &'a Value, name: &str) -> Result<&'a str, String> { value.get(name).and_then(Value::as_str).ok_or_else(|| format!("Missing {name}")) }
fn storage(error: impl std::fmt::Display) -> String { format!("Outline storage: {error}") }
async fn atomic_write(path: &Path, text: &str) -> Result<(), String> {
    let parent = path.parent().ok_or("Invalid Outline path")?;
    tokio::fs::create_dir_all(parent).await.map_err(storage)?;
    let temp = parent.join(format!("{}.tmp", uuid::Uuid::new_v4()));
    tokio::fs::write(&temp, text).await.map_err(storage)?;
    let result = tokio::fs::rename(&temp, path).await.map_err(storage);
    if result.is_err() { let _ = tokio::fs::remove_file(temp).await; }
    result
}
impl Session {
    pub(crate) async fn outline_root(&self) -> PathBuf { self.get_config().await.codex_home.as_path().join("dodex").join("outlines").join(self.thread_id.to_string()) }
    async fn load_outline(&self) -> Result<OutlineState, String> {
        let reset = self.history_reset().await.0;
        if let Some((version,state)) = self.outline.cache.lock().await.as_ref() { if *version == reset { return Ok(state.clone()); } }
        let Some(live) = self.live_thread() else { return Ok(OutlineState::empty()); };
        let state = match live.load_outline_state().await.map_err(storage)? {
            Some(value) => serde_json::from_value(value).map_err(storage)?, None => OutlineState::empty(),
        };
        state.validate()?;
        for doc in &state.documents { for (revision, snapshot) in &doc.revisions { if ((!doc.deleted && *revision == doc.revision) || !snapshot.text.is_empty()) && hash(&snapshot.text) != *revision { return Err("Corrupt Outline revision; retained unchanged".into()); } } }
        self.outline.enabled.store(state.enabled, Ordering::Release);
        *self.outline.active_id.write().map_err(storage)? = state.active_id.clone();
        *self.outline.cache.lock().await = Some((reset, state.clone()));
        Ok(state)
    }
    async fn persist_outline(&self, state: &mut OutlineState) -> Result<(), String> {
        state.generation += 1;
        let event = EventMsg::DodexOutlineState(DodexCommentaryStateEvent { state: serde_json::to_value(view_state(state)).map_err(storage)? });
        let live = self.live_thread_for_persistence("save Outline").map_err(storage)?;
        live.append_items(&[RolloutItem::EventMsg(event.clone())]).await.map_err(storage)?;
        live.flush().await.map_err(storage)?;
        self.outline.enabled.store(state.enabled, Ordering::Release);
        *self.outline.active_id.write().map_err(storage)? = state.active_id.clone();
        *self.outline.cache.lock().await = Some((self.history_reset().await.0, view_state(state)));
        self.publish_outline_event(codex_protocol::protocol::Event { id: String::new(), msg: event }).await;
        Ok(())
    }
    async fn preserve_outline_draft(&self, id: &str, text: &str) -> Result<PathBuf, String> {
        if !codex_protocol::outline::valid_id(id) { return Err("Invalid Outline ID".into()); }
        let path = self.outline_root().await.join(id).join("drafts").join(format!("{}.md", uuid::Uuid::new_v4()));
        atomic_write(&path, text).await?; Ok(path)
    }
    /// Import external saves only against the revision last projected. Never overwrite a conflicting file.
    async fn sync_outline_projection(&self, state: &mut OutlineState, id: Option<&str>) -> Result<(), String> {
        let Some(doc) = target_document(state,id).cloned() else { return Ok(()); };
        let folder = self.outline_root().await.join(&doc.id);
        let path = folder.join("outline.md"); let base_path = folder.join("revision");
        let base = tokio::fs::read_to_string(&base_path).await.ok();
        let observed_file = tokio::fs::read(&path).await.ok();
        if let Ok(meta) = tokio::fs::metadata(&path).await {
            if meta.len() > MAX_BYTES as u64 { return Err(format!("External Outline exceeds 1 MiB; retained at {}", path.display())); }
            let external = tokio::fs::read_to_string(&path).await.map_err(storage)?;
            let external_revision = hash(&external);
            if external_revision != doc.revision && base.as_deref() != Some(external_revision.as_str()) {
                if base.as_deref() != Some(doc.revision.as_str()) {
                    let draft = self.preserve_outline_draft(&doc.id, &external).await?;
                    return Err(format!("External Outline conflicts with the current revision. Draft preserved at {}. Resolve the file before resuming.", draft.display()));
                }
                // Recheck immediately before acceptance; arbitrary writers cannot participate in our mutex.
                if tokio::fs::read_to_string(&path).await.map_err(storage)? != external { return Err("External Outline changed while importing; retry".into()); }
                let doc = state.documents.iter_mut().find(|d| d.id == doc.id).ok_or("Outline disappeared")?;
                doc.revision = external_revision.clone();
                archive(doc, external_revision, external);
                self.persist_outline(state).await?;
            }
        }
        let doc = target_document(state,id).ok_or("Outline disappeared")?;
        // Avoid writes and redraws when neither the file nor the accepted revision changed.
        if base.as_deref() != Some(doc.revision.as_str()) || !path.exists() {
            if tokio::fs::read(&path).await.ok() != observed_file { return Err(format!("Outline file changed during refresh; retained at {}", path.display())); }
            atomic_write(&path, doc.text()).await?;
            atomic_write(&base_path, &doc.revision).await?;
        }
        Ok(())
    }
    pub(crate) async fn outline_request(&self, request: Value) -> Result<Value, String> {
        let _guard = self.outline.lock.lock().await;
        let mut state = self.load_outline().await?;
        let action = request.get("action").and_then(Value::as_str).unwrap_or("get");
        tracing::debug!(action, generation = state.generation, "Outline operation");
        // Explicit off/delete must remain usable even when an external file is damaged or conflicting.
        let target = if matches!(action,"edit" | "restore") { request.get("id").and_then(Value::as_str) } else { None };
        if (state.enabled || matches!(action,"edit" | "restore")) && !matches!(action, "off" | "delete" | "revision" | "draft") {
            if let Err(error) = self.sync_outline_projection(&mut state,target).await {
                state.enabled = false; self.persist_outline(&mut state).await?; return Err(error);
            }
        }
        if action == "get" { return Ok(json!({"state": view_state(&state), "root": self.outline_root().await})); }
        if action == "draft" {
            let path = self.preserve_outline_draft(field(&request, "id")?, field(&request, "content")?).await?;
            return Ok(json!({"draft_path": path}));
        }
        if action == "revision" {
            let doc = state.documents.iter().find(|d| d.id == request["id"]).ok_or("Outline not found")?;
            let revision = field(&request, "revision")?;
            let text = self.outline_revision_text(doc,revision).await?;
            return Ok(json!({"text": text}));
        }
        if matches!(action, "read" | "update") && !state.enabled { return Err("Outline mode is off; no Outline maintenance is permitted".into()); }
        if action == "read" {
            if request.get("id").and_then(Value::as_str).is_some_and(|id| state.active_id.as_deref() != Some(id)) { return Err("Active Outline changed; retry with the current document".into()); }
            let doc = state.active().ok_or("No active Outline")?;
            let offset = request.get("offset").and_then(Value::as_u64).unwrap_or(1).max(1) as usize;
            let limit = request.get("limit").and_then(Value::as_u64).unwrap_or(150).clamp(1, 300) as usize;
            let lines: Vec<_> = doc.text().lines().collect();
            let mut body = String::new(); let mut count = 0; let mut long_line = false;
            for (i, line) in lines.iter().enumerate().skip(offset - 1).take(limit) {
                let next = format!("{}: {}\n", i + 1, line);
                if body.chars().count() + next.chars().count() > 24000 { body.extend(next.chars().take(24000usize.saturating_sub(body.chars().count()))); long_line = true; break; }
                body.push_str(&next); count += 1;
            }
            return Ok(json!({"id":doc.id,"revision":doc.revision,"total_lines":lines.len(),"partial":long_line || offset > 1 || offset - 1 + count < lines.len(),"long_line_truncated":long_line,"path":self.outline_root().await.join(&doc.id).join("outline.md"),"text":body}));
        }
        if action == "create" {
            check_generation(&state, &request)?;
            let text = field(&request, "content")?; check_size(text)?;
            let revision = hash(text); let id = uuid::Uuid::new_v4().to_string();
            let title: String = field(&request, "title")?.chars().filter(|c| !c.is_control()).take(160).collect();
            state.documents.push(OutlineDocument { id: id.clone(), title: if title.trim().is_empty() { "Untitled Outline".into() } else { title }, source_id: field(&request,"source_id")?.into(), revision: revision.clone(), deleted: false, revisions: [(revision, OutlineRevision { text: text.into(), saved_at: now() })].into() });
            state.active_id = Some(id); state.enabled = true;
        } else if action == "off" { check_generation(&state, &request)?; state.enabled = false; }
        else if action == "resume" {
            check_generation(&state, &request)?;
            let id = field(&request, "id")?;
            if !state.documents.iter().any(|d| d.id == id && !d.deleted) { return Err("Outline does not belong to this conversation or was deleted".into()); }
            state.active_id = Some(id.into()); state.enabled = true;
        } else if action == "delete" {
            check_generation(&state, &request)?;
            let id = request.get("id").and_then(Value::as_str).or(state.active_id.as_deref()).ok_or("No selected Outline")?.to_owned();
            let doc = state.documents.iter_mut().find(|d| d.id == id && !d.deleted).ok_or("Outline not found")?;
            doc.deleted = true;
            if state.active_id.as_deref() == Some(id.as_str()) { state.enabled = false; state.active_id = None; }
        } else if matches!(action, "update" | "edit" | "restore") {
            let id = request.get("id").and_then(Value::as_str).or(state.active_id.as_deref()).ok_or("No selected Outline")?.to_owned();
            if action == "update" && state.active_id.as_deref() != Some(id.as_str()) { return Err("Active Outline changed; reread before updating".into()); }
            let doc = state.documents.iter_mut().find(|d| d.id == id && !d.deleted).ok_or("Outline not found")?;
            if doc.revision != field(&request, "expected_revision")? {
                if action == "edit" { let draft = self.preserve_outline_draft(&id, field(&request,"content")?).await?; return Err(format!("Outline changed. Your draft was preserved at {}", draft.display())); }
                return Err("Outline changed; reread the latest revision before editing. Nothing was overwritten".into());
            }
            let text = if action == "restore" { self.outline_revision_text(doc,field(&request,"revision")?).await? }
                else if let Some(content) = request.get("content").and_then(Value::as_str) {
                    if request.get("old_text").is_some() || request.get("new_text").is_some() { return Err("Supply content OR old_text/new_text".into()); } content.to_owned()
                } else {
                    let old = field(&request,"old_text")?; let new = field(&request,"new_text")?;
                    if old.is_empty() || doc.text().matches(old).count() != 1 { return Err("old_text must match exactly once; read a unique passage".into()); }
                    doc.text().replacen(old,new,1)
                };
            check_size(&text)?; let revision = hash(&text);
            if revision == doc.revision { return Ok(if action == "update" { json!({"revision":revision,"changed":false}) } else { json!({"state":view_state(&state),"root":self.outline_root().await}) }); }
            archive(doc, revision.clone(), text); doc.revision = revision;
        } else { return Err(format!("Unknown Outline action: {action}")); }
        self.persist_outline(&mut state).await?;
        // Accepted text is durable even if projection I/O fails; report the warning separately.
        let warning = if state.enabled || matches!(action,"edit" | "restore") { self.sync_outline_projection(&mut state,target).await.err() } else { None };
        if warning.is_some() && state.enabled { state.enabled = false; self.persist_outline(&mut state).await?; }
        if action == "update" { return Ok(json!({"revision":state.active().map(|d| &d.revision),"changed":true,"warning":warning})); }
        Ok(json!({"state":view_state(&state),"root":self.outline_root().await,"warning":warning}))
    }
    async fn outline_revision_text(&self, doc: &OutlineDocument, revision: &str) -> Result<String,String> {
        let snapshot = doc.revisions.get(revision).ok_or("Saved Outline revision unavailable")?;
        let text = if !snapshot.text.is_empty() || revision == codex_protocol::outline::EMPTY_REVISION { snapshot.text.clone() }
            else { self.live_thread_for_persistence("read Outline revision").map_err(storage)?.load_outline_revision(doc.id.clone(), revision.into()).await.map_err(storage)?.ok_or("Outline revision body is unavailable in this conversation's history")? };
        if hash(&text) != revision { return Err("Saved Outline revision is corrupt; retained unchanged".into()); }
        Ok(text)
    }
    pub(crate) async fn outline_warning(&self, error: String) {
        self.outline.enabled.store(false, Ordering::Release);
        if !self.outline.warned.swap(true, Ordering::AcqRel) {
            self.send_event_raw(codex_protocol::protocol::Event { id: String::new(), msg: EventMsg::Warning(codex_protocol::protocol::WarningEvent { message: format!("Outline is unavailable; ordinary prompting continues. Saved history was retained. {error}") }) }).await;
        }
    }
    pub(crate) async fn outline_reference(&self, prompt: &str) -> Result<Option<String>, String> {
        let value = match self.outline_request(json!({"action":"get"})).await { Ok(value) => value, Err(error) => { self.outline_warning(error).await; return Ok(None); } };
        let state: OutlineState = serde_json::from_value(value["state"].clone()).map_err(storage)?;
        if !state.enabled { return Ok(None); }
        let doc = state.active().ok_or("Active Outline unavailable")?;
        Ok(Some(format!("Use this current revision over older Outline excerpts.\n{}", doc.reference(prompt))))
    }
}
fn check_size(text: &str) -> Result<(), String> { if text.len() > MAX_BYTES { Err("Outline exceeds 1 MiB; preserve the draft before reducing its size".into()) } else { Ok(()) } }
fn check_generation(state: &OutlineState, request: &Value) -> Result<(), String> {
    if request.get("expected_generation").and_then(Value::as_u64) != Some(state.generation) { return Err("Outline selection changed; refresh and retry".into()); } Ok(())
}

/// Archived bodies remain available through the revision operation, including after logical deletion.
fn view_state(state: &OutlineState) -> OutlineState {
    let mut view=state.clone();
    for doc in &mut view.documents { for (revision,snapshot) in &mut doc.revisions { if doc.deleted || *revision != doc.revision { snapshot.text.clear(); } } }
    view
}

fn archive(doc: &mut OutlineDocument, revision: String, text: String) {
    if let Some(snapshot) = doc.revisions.get_mut(&revision) { snapshot.text = text; }
    else { doc.revisions.insert(revision, OutlineRevision { text, saved_at: now() }); }
}

fn target_document<'a>(state: &'a OutlineState, id: Option<&str>) -> Option<&'a OutlineDocument> {
    match id { Some(id) => state.documents.iter().find(|doc| doc.id == id && !doc.deleted), None => state.active() }
}

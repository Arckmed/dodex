//! Outline UI controller. All durable operations go through the active app-server.
use super::*;
use crate::dodex_features::outline::{OutlineAction, OutlineEditor, EditEvent};
use crate::dodex_features::commentary_reader::CommentaryReader;
use crate::bottom_pane::BottomPaneView;
use serde_json::json;
use ratatui::widgets::Widget;

impl App {
    pub(super) fn sync_outline_thread(&mut self) {
        let thread = self.current_displayed_thread_id();
        if self.outline.thread == thread { return; }
        self.outline = Default::default(); self.outline.thread = thread;
        if let Some(thread_id) = thread { self.app_event_tx.send(AppEvent::Outline { thread_id, action: OutlineAction::Refresh }); }
    }
    pub(super) fn refresh_outline(&mut self, server: &AppServerSession, thread_id: ThreadId) {
        if self.current_displayed_thread_id() != Some(thread_id) || self.reconnect.offline { return; }
        if self.outline.loading { self.outline.refresh_again = true; return; }
        self.outline.loading = true;
        let load_id = uuid::Uuid::new_v4(); self.outline.load_id = Some(load_id);
        let handle = server.request_handle(); let tx = self.app_event_tx.clone();
        tokio::spawn(async move {
            let result: Result<codex_app_server_protocol::ThreadOutlineResponse, _> = handle.request_typed(ClientRequest::ThreadOutline {
                request_id: codex_app_server_protocol::RequestId::String(format!("outline-{}",uuid::Uuid::new_v4())),
                params: codex_app_server_protocol::ThreadOutlineParams { thread_id: thread_id.to_string(), request: json!({"action":"get"}) },
            }).await;
            tx.send(AppEvent::Outline { thread_id, action: OutlineAction::Loaded { load_id, result: result.map(|r|r.result).map_err(|e|e.to_string()) } });
        });
    }
    fn accept_outline(&mut self, thread_id: ThreadId, value: serde_json::Value) -> Result<(), String> {
        let state: codex_protocol::outline::OutlineState = serde_json::from_value(value["state"].clone()).map_err(|e|e.to_string())?;
        state.validate()?;
        // Invalidate older background responses after a completed mutation; history rollback may lower generation.
        self.outline.load_id = None; self.outline.loading = false;
        let changed = self.outline.state != state;
        self.outline.thread = Some(thread_id); self.outline.state = state;
        self.outline.root = value["root"].as_str().map(std::path::PathBuf::from).unwrap_or_default();
        if let Some(warning) = value["warning"].as_str() { self.chat_widget.add_error_message(warning.into()); }
        if changed && (!self.outline.dragging || !self.outline.state.enabled) { self.outline.reader = None; }
        if let Some(doc) = self.outline.state.active() {
            let source = crate::dodex_features::outline::source(doc, thread_id, &self.outline.root);
            self.chat_widget.update_outline_reader(&source, doc.text());
        }
        if self.outline.state.enabled && !self.outline.poll_scheduled {
            self.outline.poll_scheduled = true; let tx=self.app_event_tx.clone();
            tokio::spawn(async move { tokio::time::sleep(Duration::from_secs(2)).await; tx.send(AppEvent::Outline { thread_id, action: OutlineAction::Refresh }); });
        }
        Ok(())
    }
    pub(super) async fn outline_action(&mut self, tui: &mut tui::Tui, server: &mut AppServerSession, thread_id: ThreadId, action: OutlineAction) {
        if self.current_displayed_thread_id() != Some(thread_id) { return; }
        let background = matches!(&action, OutlineAction::Refresh | OutlineAction::Loaded { .. });
        let before = (self.outline.state.generation, self.outline.state.enabled);
        let result: Result<(), String> = async {
            match action {
                OutlineAction::Refresh => { self.outline.poll_scheduled = false; self.refresh_outline(server,thread_id); }
                OutlineAction::Loaded { load_id, result } => {
                    if self.outline.load_id != Some(load_id) { return Ok(()); }
                    self.outline.loading = false;
                    match result { Ok(value) => self.accept_outline(thread_id,value)?, Err(error) => {
                        self.outline.reader = None; self.outline.state.enabled = false;
                        // Report once, then allow explicit commands to retry; don't spam on every poll.
                        self.chat_widget.add_error_message(format!("Could not restore Outline: {error}")); return Ok(());
                    } }
                    if self.outline.refresh_again {
                        self.outline.refresh_again = false;
                        self.app_event_tx.send(AppEvent::Outline { thread_id, action: OutlineAction::Refresh });
                    }
                    if self.outline.state.enabled && !self.outline.poll_scheduled {
                        self.outline.poll_scheduled = true; let tx=self.app_event_tx.clone();
                        tokio::spawn(async move { tokio::time::sleep(Duration::from_secs(2)).await; tx.send(AppEvent::Outline { thread_id, action: OutlineAction::Refresh }); });
                    }
                }
                OutlineAction::Command(command) => self.outline_command(server,thread_id,&command).await?,
                OutlineAction::Apply(request) => {
                    let value=server.thread_outline(thread_id,request).await.map_err(|e|e.to_string())?;
                    self.accept_outline(thread_id,value)?;
                }
                OutlineAction::Seed { source_id, text } => {
                    let title = text.lines().find(|l|!l.trim().is_empty()).unwrap_or("Outline").trim_start_matches('#').trim().chars().take(100).collect::<String>();
                    let generation=self.outline.state.generation; let tx=self.app_event_tx.clone();
                    let editor=OutlineEditor::new("Name the Outline".into(),&title,true,&self.keymap,Box::new(move |event| {
                        if let EditEvent::Save(title)=event { tx.send(AppEvent::Outline { thread_id, action: OutlineAction::Create { source_id:source_id.clone(),text:text.clone(),title,generation } }); }
                    })); self.chat_widget.show_bottom_pane_view(Box::new(editor));
                }
                OutlineAction::Create { source_id,text,title,generation } => {
                    let value=server.thread_outline(thread_id,json!({"action":"create","source_id":source_id,"content":text,"title":title,"expected_generation":generation})).await.map_err(|e|e.to_string())?;
                    self.accept_outline(thread_id,value)?;
                }
                OutlineAction::Save { id,revision,text } => {
                    let response=server.thread_outline(thread_id,json!({"action":"edit","id":id,"expected_revision":revision,"content":text})).await;
                    match response {
                        Ok(value)=>self.accept_outline(thread_id,value)?,
                        Err(error)=>{
                            let draft=server.thread_outline(thread_id,json!({"action":"draft","id":id,"content":text})).await.map_err(|e|e.to_string())?;
                            return Err(format!("{error}. Draft retained at {}",draft["draft_path"]));
                        }
                    }
                }
                OutlineAction::External { id,revision,text } => {
                    let command=match external_editor::resolve_editor_command() {
                        Ok(command)=>command,
                        Err(error)=>{ self.open_outline_editor(thread_id,id,revision,&text); return Err(format!("{error}. Your draft remains in the editor.")); }
                    };
                    let config=self.chat_widget.config_ref(); let policy=config.permissions.file_system_sandbox_policy();
                    let result=tui.with_restored(tui::TerminalHandoff::KeepScreen,|| async {
                        external_editor::run_editor(&text,&command,config.codex_home.as_path(),&policy,config.cwd.as_path()).await
                    }).await;
                    match result {
                        Ok(text)=>self.app_event_tx.send(AppEvent::Outline { thread_id,action:OutlineAction::Save { id,revision,text } }),
                        Err(error)=>{
                            self.open_outline_editor(thread_id,id,revision,&text);
                            return Err(format!("External editor failed: {error}. Your draft remains in the editor."));
                        }
                    }
                }
                OutlineAction::Keep { id,text } => {
                    let value=server.thread_outline(thread_id,json!({"action":"draft","id":id,"content":text})).await.map_err(|e|e.to_string())?;
                    self.chat_widget.add_info_message(format!("Outline draft saved at {}",value["draft_path"]),None);
                }
                OutlineAction::Cancel => {}
            } Ok(())
        }.await;
        let failed = result.is_err();
        if let Err(error)=result { self.chat_widget.add_error_message(format!("Outline: {error}")); }
        if failed || !background || before != (self.outline.state.generation,self.outline.state.enabled) { tui.frame_requester().schedule_frame(); }
    }
    fn open_outline_editor(&mut self,thread_id:ThreadId,id:String,revision:String,text:&str) {
        let tx=self.app_event_tx.clone();
        let title=format!("Editing Outline - revision {}",&revision[..revision.len().min(12)]);
        let editor=OutlineEditor::new(title,text,false,&self.keymap,Box::new(move |event| {
            let action=match event {
                EditEvent::Save(text)=>OutlineAction::Save { id:id.clone(),revision:revision.clone(),text },
                EditEvent::External(text)=>OutlineAction::External { id:id.clone(),revision:revision.clone(),text },
                EditEvent::Keep(text)=>OutlineAction::Keep { id:id.clone(),text },
                EditEvent::Cancel=>OutlineAction::Cancel,
            }; tx.send(AppEvent::Outline { thread_id,action });
        })); self.chat_widget.show_bottom_pane_view(Box::new(editor));
    }
    async fn outline_command(&mut self,server:&mut AppServerSession,thread_id:ThreadId,command:&str)->Result<(),String> {
        let value=server.thread_outline(thread_id,json!({"action":"get"})).await.map_err(|e|e.to_string())?;
        self.accept_outline(thread_id,value)?;
        let mut args=command.split_whitespace(); let verb=args.next().unwrap_or(""); let arg=args.next(); let third=args.next();
        if args.next().is_some() { return Err("Usage: /outline [start|resume [id]|off|view|edit|history|path|delete]".into()); }
        let generation=self.outline.state.generation;
        match verb {
            "start" if arg.is_none()=>{
                let sources=server.thread_commentary_sources(thread_id).await.map_err(|e|e.to_string())?;
                let mut items=Vec::new();
                for (_,item) in sources {
                    let codex_app_server_protocol::ThreadItemPayload::AgentMessage { id,text,.. }=item.payload else { continue; };
                    if text.trim().is_empty() { continue; }
                    let name=text.split_whitespace().collect::<Vec<_>>().join(" ").chars().filter(|c|!c.is_control()).take(160).collect();
                    items.push(SelectionItem { name,description:Some(id.clone()),dismiss_on_select:true,actions:vec![Box::new(move |tx|tx.send(AppEvent::Outline { thread_id,action:OutlineAction::Seed { source_id:id.clone(),text:text.clone() } }))],..Default::default() });
                }
                if items.is_empty() { return Err("Discuss the idea first; there is no saved assistant message to copy.".into()); }
                self.chat_widget.show_selection_view(SelectionViewParams { title:Some("Choose the message to copy into Outline".into()),items,..Default::default() });
            }
            "resume" | "" if arg.is_none() && !(verb.is_empty() && self.outline.state.enabled)=>{
                let items=self.outline.state.documents.iter().filter(|d|!d.deleted).map(|doc| {
                    let id=doc.id.clone(); SelectionItem { name:doc.title.clone(),description:Some(doc.id.clone()),dismiss_on_select:true,actions:vec![Box::new(move |tx|tx.send(AppEvent::Outline { thread_id,action:OutlineAction::Command(format!("resume {id}")) }))],..Default::default() }
                }).collect::<Vec<_>>();
                if items.is_empty() { return Err("No resumable Outlines. Use /outline start.".into()); }
                self.chat_widget.show_selection_view(SelectionViewParams { title:Some("Resume Outline".into()),items,..Default::default() });
            }
            "resume" if arg.is_some() && third.is_none()=>{
                let value=server.thread_outline(thread_id,json!({"action":"resume","id":arg,"expected_generation":generation})).await.map_err(|e|e.to_string())?; self.accept_outline(thread_id,value)?;
            }
            "off" if arg.is_none()=>{
                let value=server.thread_outline(thread_id,json!({"action":"off","expected_generation":generation})).await.map_err(|e|e.to_string())?; self.accept_outline(thread_id,value)?;
                self.chat_widget.add_info_message("Outline off. Future requests use ordinary prompting; saved documents and existing context remain.".into(),None);
            }
            "view" | "" if arg.is_none()=>{
                let doc=self.outline.state.active().ok_or("Select an Outline first")?.clone();
                let source=crate::dodex_features::outline::source(&doc,thread_id,&self.outline.root);
                if !self.load_commentary_session(server,thread_id).await { return Err("Could not load Commentary".into()); }
                let tx=self.app_event_tx.clone();
                let mut reader=CommentaryReader::new(source,doc.text().into(),&self.config.cwd,&self.keymap,self.commentary_enabled(),Box::new(move |event|tx.send(AppEvent::CommentaryReader { thread_id,event })));
                reader.enable_live_outline(); reader.set_passages(self.commentary_passages());
                self.chat_widget.show_bottom_pane_view(Box::new(reader));
            }
            "edit" if third.is_none()=>{
                let doc=if let Some(id)=arg { self.outline.state.documents.iter().find(|d|d.id==id && !d.deleted) } else { self.outline.state.active() }.ok_or("Select an Outline first")?.clone();
                let text=doc.text().to_owned();
                self.open_outline_editor(thread_id,doc.id,doc.revision,&text);
            }
            "path" if arg.is_none()=>{
                let doc=self.outline.state.active().ok_or("Select an Outline first")?;
                self.chat_widget.add_info_message(self.outline.root.join(&doc.id).join("outline.md").display().to_string(),None);
            }
            "history" if arg.is_none()=>{
                let doc=self.outline.state.active().ok_or("Select an Outline first")?;
                let mut revisions=doc.revisions.iter().collect::<Vec<_>>(); revisions.sort_by(|a,b|b.1.saved_at.cmp(&a.1.saved_at));
                let items=revisions.into_iter().map(|(revision,snapshot)| {
                    let command=format!("restore {} {}",doc.id,revision);
                    SelectionItem { name:format!("{} - {}{}",&revision[..12],snapshot.saved_at,if *revision==doc.revision { " (current)" } else { "" }),dismiss_on_select:true,
                        actions:vec![Box::new(move |tx|tx.send(AppEvent::Outline { thread_id,action:OutlineAction::Command(command.clone()) }))],..Default::default() }
                }).collect();
                self.chat_widget.show_selection_view(SelectionViewParams { title:Some("Choose an Outline revision to restore".into()),items,..Default::default() });
            }
            "restore" if arg.is_some() && third.is_some()=>{
                let doc=self.outline.state.documents.iter().find(|d|Some(d.id.as_str())==arg && !d.deleted).ok_or("Outline not found")?;
                let request=json!({"action":"restore","id":doc.id,"revision":third,"expected_revision":doc.revision});
                self.confirm_outline_operation(thread_id,"Restore this Outline revision?",request);
            }
            "delete" if arg.is_none()=>{
                let doc=self.outline.state.active().ok_or("Select an Outline first")?;
                self.confirm_outline_operation(thread_id,"Delete Outline? Historical quotation sources will remain.",json!({"action":"delete","id":doc.id,"expected_generation":generation}));
            }
            _=>return Err("Usage: /outline [start|resume [id]|off|view|edit|history|path|delete]".into()),
        } Ok(())
    }
    fn confirm_outline_operation(&mut self,thread_id:ThreadId,title:&str,request:serde_json::Value) {
        self.chat_widget.show_selection_view(SelectionViewParams { title:Some(title.into()),items:vec![SelectionItem { name:"Cancel".into(),dismiss_on_select:true,..Default::default() },SelectionItem { name:"Confirm".into(),dismiss_on_select:true,actions:vec![Box::new(move |tx|tx.send(AppEvent::Outline { thread_id,action:OutlineAction::Apply(request.clone()) }))],..Default::default() }],..SelectionViewParams::confirmation() });
    }
    pub(super) fn paint_outline(&mut self,frame:&mut crate::custom_terminal::Frame<'_>) {
        self.outline.area=Rect::default();
        if self.chat_widget.has_active_modal() || self.overlay.is_some() || !self.outline.state.enabled { return; }
        let Some(thread_id)=self.outline.thread else { return; };
        let automatic=self.commentary_enabled();
        let Some(doc)=self.outline.state.active() else { return; };
        let reader_key = format!("{}:{}",doc.id,doc.revision);
        if self.outline.reader.is_none() || (!self.outline.dragging && (self.outline.automatic!=automatic || self.outline.reader_key != reader_key)) {
            let source=crate::dodex_features::outline::source(doc,thread_id,&self.outline.root); let tx=self.app_event_tx.clone();
            self.outline.reader=Some(CommentaryReader::new(source,doc.text().into(),&self.config.cwd,&self.keymap,automatic,Box::new(move |event|tx.send(AppEvent::CommentaryReader { thread_id,event }))));
            self.outline.automatic=automatic; self.outline.reader_key=reader_key;
        }
        let viewport=frame.area(); if viewport.width<12 || viewport.height<10 { return; }
        let height=(viewport.height/5).clamp(5,8).min(viewport.height.saturating_sub(5));
        let area=Rect::new(viewport.x,viewport.y,viewport.width,height); self.outline.area=area;
        let passages=self.commentary_passages();
        frame.image_placements.retain(|p|!area.intersects(p.area));
        if let Some(reader)=self.outline.reader.as_mut() { reader.set_passages(passages); ratatui::widgets::Clear.render(area,frame.buffer); reader.render(area,frame.buffer); }
    }
    pub(super) fn outline_mouse(&mut self,event:crossterm::event::MouseEvent)->bool {
        if self.outline.area.is_empty() { return false; }
        if !self.outline.area.contains((event.column,event.row).into()) && !self.outline.dragging { return false; }
        if event.kind==crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left) && (event.row==self.outline.area.y || event.row>=self.outline.area.bottom().saturating_sub(2)) {
            self.app_event_tx.send(AppEvent::OutlineCommand("view".into())); return true;
        }
        if event.kind == crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left) { self.outline.dragging = true; }
        let handled = self.outline.reader.as_mut().is_some_and(|r|r.handle_mouse_event(event));
        if event.kind == crossterm::event::MouseEventKind::Up(crossterm::event::MouseButton::Left) { self.outline.dragging = false; }
        handled
    }
}

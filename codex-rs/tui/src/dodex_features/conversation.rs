//! Local conversation presentation preferences and accumulated open time.
//! Files are locked across windows and atomically replaced. No state enters model requests.
//! Totals sum visits, including overlapping windows, and start when tracking is first installed.
//! Checkpoints bound unclean-exit loss to approximately ten seconds while the UI is responsive.

use codex_protocol::ThreadId;
use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
struct Saved {
    total_millis: u64,
    thoughts_visible: bool,
}

#[derive(Default)]
pub(crate) struct ConversationView {
    thread: Option<ThreadId>,
    directory: PathBuf,
    entered: Option<Instant>,
    checkpoint: Option<Instant>,
    total_millis: u64,
    pub(crate) thoughts_visible: bool,
    last_timer_seconds: Option<(u64, u64)>,
    failed: bool,
}

fn update(directory: &Path, thread: ThreadId, edit: impl FnOnce(&mut Saved)) -> io::Result<Saved> {
    std::fs::create_dir_all(directory)?;
    let lock = OpenOptions::new().create(true).truncate(false).read(true).write(true)
        .open(directory.join(format!("{thread}.lock")))?;
    lock.lock()?;
    let path = directory.join(format!("{thread}.json"));
    let mut state: Saved = match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(io::Error::other)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => Saved::default(),
        Err(error) => return Err(error),
    };
    edit(&mut state);
    let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
    serde_json::to_writer(&mut temporary, &state).map_err(io::Error::other)?;
    temporary.flush()?;
    temporary.as_file().sync_all()?;
    temporary.persist(&path).map_err(|error| error.error)?;
    Ok(state)
}

impl ConversationView {
    pub(crate) fn sync(&mut self, home: &Path, thread: Option<ThreadId>, now: Instant) -> io::Result<bool> {
        if self.thread == thread { return Ok(false); }
        let saved_previous = self.save(now, None);
        self.thread = thread;
        self.directory = home.join("dodex").join("conversation-ui");
        self.entered = thread.map(|_| now);
        self.checkpoint = self.entered;
        self.total_millis = 0;
        self.last_timer_seconds = None;
        self.thoughts_visible = false;
        self.failed = false;
        if let Some(thread) = thread {
            match update(&self.directory, thread, |_| {}) {
                Ok(saved) => { self.total_millis = saved.total_millis; self.thoughts_visible = saved.thoughts_visible; }
                Err(error) => { self.failed = true; return Err(error); }
            }
        }
        saved_previous?;
        Ok(true)
    }

    pub(crate) fn elapsed(&self, now: Instant) -> Option<(Duration, Duration)> {
        if self.failed { return None; }
        let current = now.saturating_duration_since(self.entered?);
        let pending = now.saturating_duration_since(self.checkpoint?);
        Some((current, Duration::from_millis(self.total_millis).saturating_add(pending)))
    }

    pub(crate) fn tick(&mut self, now: Instant) -> io::Result<()> {
        if !self.failed && self.checkpoint.is_some_and(|at| now.saturating_duration_since(at) >= Duration::from_secs(10)) {
            self.save(now, None)?;
        }
        Ok(())
    }

    pub(crate) fn timers_changed(&mut self, now: Instant) -> bool {
        let seconds = self.elapsed(now).map(|(current, total)| (current.as_secs(), total.as_secs()));
        let changed = self.last_timer_seconds != seconds;
        self.last_timer_seconds = seconds;
        changed
    }

    pub(crate) fn toggle(&mut self, now: Instant) -> io::Result<()> {
        if self.thread.is_none() { return Err(io::Error::other("Open a conversation before using /thoughts.")); }
        if self.failed { return Err(io::Error::other("Conversation UI state could not be loaded; its saved file was preserved.")); }
        self.save(now, Some(!self.thoughts_visible))
    }

    pub(crate) fn save(&mut self, now: Instant, visibility: Option<bool>) -> io::Result<()> {
        let Some(thread) = self.thread else { return Ok(()); };
        if self.failed { return Ok(()); }
        let millis = self.checkpoint.map_or(0, |at| u64::try_from(now.saturating_duration_since(at).as_millis()).unwrap_or(u64::MAX));
        let saved = update(&self.directory, thread, |saved| {
            saved.total_millis = saved.total_millis.saturating_add(millis);
            if let Some(visible) = visibility { saved.thoughts_visible = visible; }
        })?;
        self.total_millis = saved.total_millis;
        self.thoughts_visible = saved.thoughts_visible;
        self.checkpoint = Some(now);
        Ok(())
    }
}

impl Drop for ConversationView {
    fn drop(&mut self) {
        if let Err(error) = self.save(Instant::now(), None) { tracing::warn!(%error, "Could not checkpoint conversation open time"); }
    }
}

pub(crate) fn timer_label(label: &str, elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
    format!("{label} {:02}:{:02}:{:02}", seconds / 3600, seconds / 60 % 60, seconds % 60)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn visits_accumulate_and_visibility_is_conversation_owned() {
        let home = tempfile::tempdir().unwrap();
        let a = ThreadId::new(); let b = ThreadId::new(); let start = Instant::now();
        let mut view = ConversationView::default();
        view.sync(home.path(), Some(a), start).unwrap();
        view.toggle(start + Duration::from_secs(10)).unwrap();
        view.sync(home.path(), Some(b), start + Duration::from_secs(20)).unwrap();
        assert!(!view.thoughts_visible);
        view.sync(home.path(), Some(a), start + Duration::from_secs(50)).unwrap();
        assert!(view.thoughts_visible);
        assert_eq!(view.elapsed(start + Duration::from_secs(60)), Some((Duration::from_secs(10), Duration::from_secs(30))));
        view.save(start + Duration::from_secs(60), None).unwrap();
        view.thread = None;
        let mut reopened = ConversationView::default();
        reopened.sync(home.path(), Some(a), start + Duration::from_secs(100)).unwrap();
        assert_eq!(reopened.elapsed(start + Duration::from_secs(100)).unwrap().1, Duration::from_secs(30));
        assert!(reopened.thoughts_visible);
    }
    #[test]
    fn simultaneous_windows_add_deltas_without_overwriting_visibility() {
        let home = tempfile::tempdir().unwrap(); let id = ThreadId::new(); let now = Instant::now();
        let mut first = ConversationView::default(); let mut second = ConversationView::default();
        first.sync(home.path(), Some(id), now).unwrap(); second.sync(home.path(), Some(id), now).unwrap();
        first.toggle(now + Duration::from_secs(5)).unwrap();
        second.save(now + Duration::from_secs(7), None).unwrap();
        assert!(second.thoughts_visible);
        assert_eq!(second.total_millis, 12_000);
    }
    #[test]
    fn timer_labels_only_refresh_when_displayed_seconds_change() {
        let home = tempfile::tempdir().unwrap(); let now = Instant::now();
        let mut view = ConversationView::default();
        view.sync(home.path(), Some(ThreadId::new()), now).unwrap();
        assert!(view.timers_changed(now));
        assert!(!view.timers_changed(now + Duration::from_millis(500)));
        assert!(view.timers_changed(now + Duration::from_secs(1)));
    }
    #[test]
    fn corrupt_state_is_not_overwritten() {
        let home = tempfile::tempdir().unwrap(); let id = ThreadId::new();
        let directory = home.path().join("dodex/conversation-ui"); std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join(format!("{id}.json")); std::fs::write(&path, b"broken").unwrap();
        let mut view = ConversationView::default(); assert!(view.sync(home.path(), Some(id), Instant::now()).is_err());
        assert!(view.toggle(Instant::now()).is_err()); drop(view);
        assert_eq!(std::fs::read(&path).unwrap(), b"broken");
    }
}

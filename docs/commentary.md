# Commentary in Dodex

Personal passage annotations, ported from the local Pi Commentary extension. Roadmap is a separate feature and is not part of this port.

## Running the source build

From PowerShell:

```powershell
cd D:\repos\dodex
& .\codex-rs\target\debug\codex.exe --no-daemon
```

An unpackaged source build uses `--no-daemon`; it does not include the managed background-server package. Dodex uses version `0.166.0-dodex.1`, retaining the upstream compatibility version and a fork suffix. Do not reset it to `0.0.0`: the model catalogue uses the client version to decide which models to expose.

To select Sol 6.1 with extra-high reasoning for this launch without changing shared configuration:

```powershell
& .\codex-rs\target\debug\codex.exe --no-daemon -m gpt-6.1-sol -c 'model_reasoning_effort="xhigh"'
```

## Annotating

1. Run `/commentary on` or press Alt+M.
2. In the fullscreen transcript, drag over text in one completed assistant response. Releasing opens the comment editor. Selections crossing messages, unfinished responses, and responses without durable source IDs are rejected.
3. Enter saves an observation containing at least three trimmed Unicode graphemes. Shift+Enter inserts a newline. Ctrl+D keeps an unfinished draft. Esc discards the current draft; cancelling an edit retains the saved comment.
4. Repeat, then open `/comments` or press Alt+Shift+C.

Turning Commentary off retains all pending data. Passage colors are visible while enabled, preserve Markdown foreground styling, and use the saved hue across wrapping and appearance changes. Newer annotations take precedence where passages overlap.

The quotation preview shows up to six wrapped lines. Wheel over it or use Alt+PageUp / Alt+PageDown to scroll it independently of the observation.

| Command or shortcut | Action |
| --- | --- |
| `/commentary`, Alt+M | Toggle automatic annotation |
| `/commentary on`, `/commentary off` | Set mode explicitly |
| `/commentary add`, Alt+C | Annotate the current assistant selection |
| `/commentary browse` | Browse saved assistant text on this conversation branch |
| `/comments`, `/commentary review`, Alt+Shift+C | Open pending comments |
| `/commentary send` | Review the outgoing batch |

The browser and source reader provide access to earlier saved messages. Alt+C accepts a selection in the reader. Q opens an editable quotation initialized with the original response; trim it to the desired passage and press Enter. Shift+Enter inserts a newline. Selections and quotations are limited to 50,000 UTF-16 code units.

## Pending comments and sending

Up/Down or the wheel selects a comment. Enter/E edits; D/Delete asks before removal; V opens its source. Mounted sources are reached in the transcript; other saved sources open in the reader. A browses sources, R resumes an unfinished draft, and X asks before discarding that draft. Esc closes the panel.

S opens review. Tab or a click focuses the separate multiline Final consideration field. S sends only while the preview is focused; Ctrl+S or Ctrl+Enter sends from either pane. Typing S in the field edits the field. Esc retains the final consideration and returns from review without sending. The main composer draft is independent.

Submission requires an idle conversation without queued follow-up messages. The frozen batch is saved before submission and sent as literal user text, without skill/tool mention expansion or IDE-context insertion. Pending comments and their final consideration are cleared only after the exact user message is found in persisted history. Failed or unacknowledged sends retain them. Retrying requires confirmation and uses the original text with a read-only final field. Editing an unacknowledged batch requires a separately confirmed marker reset after checking history again.

## Persistence

Commentary checkpoints are embedded in the normal conversation rollout as `dodex_commentary_state` events. They include enabled mode, comments, source identity and display anchors, color hues, unfinished drafts, the final consideration, and any frozen submission. They survive resume and compaction. Forks inherit the checkpoint at their frozen history boundary; later parent changes do not alter that inherited checkpoint.

Typing saves after a 600 ms debounce; editor close, review close, normal exit, thread transitions, and forks flush pending changes. Abrupt termination can lose the last unsaved keystrokes. A failed write retains dirty state in memory and reports the failure. A normal fork is not created if its current conversation's dirty checkpoint could not be saved.

Unsent state is excluded from model context and realtime voice handoffs. It remains part of the session file: backups or exports that include raw rollout data can include unsent quotations and observations. Unsupported or damaged checkpoints are reported and retained rather than silently replaced with empty state.

## Maintaining the port

Feature-specific state and views live in `codex-rs/tui/src/dodex_features/commentary*.rs`. The thread-scoped controller is `codex-rs/tui/src/app/commentary.rs`. Background painting is isolated in `codex-rs/tui/src/transcript_view/commentary.rs`.

The upstream integration points are slash-command dispatch, owned-transcript input, bottom-pane mouse/geometry hooks, source IDs on assistant cells, and the app-server/history checkpoint methods. The storage path spans protocol, rollout policy, thread-store, core thread access, and app-server request processing. This is a small set of personal feature modules, not an extension API framework.

Relevant checks from `codex-rs`:

```powershell
cargo test -p codex-tui --lib commentary
cargo test -p codex-tui --lib keyboard_quotation
cargo test -p codex-tui --lib assistant_selection
cargo test -p codex-tui --lib passage_background_preserves_foreground_and_newer_overlap_wins
cargo test -p codex-thread-store --lib commentary_checkpoint
cargo test -p codex-app-server --test all commentary_checkpoints_survive_fork
cargo build -p codex-cli --bin codex
```

Use an isolated temporary Codex home and a localhost mock Responses provider for terminal-level acceptance checks; avoid sending verification prompts through a real model account.

# Dodex

Current source version: **0.167.2-dodex.1**.

Upstream base: OpenAI Codex CLI stable [`rust-v0.167.2`](https://github.com/openai/codex/releases/tag/rust-v0.167.2) (November 11, 2026), with Dodex customizations applied on top.

A Codex fork with a few personal extensions to the terminal interface, implemented directly in the source code.

Refer to the original README at [openai/codex](https://github.com/openai/codex).

## Compile on Windows x64

Use PowerShell with Git, Python 3.13, Rustup, ripgrep (`rg`), and Visual Studio Build Tools with Desktop development with C++ and a Windows SDK installed. Cargo uses the Rust version pinned in `codex-rs/rust-toolchain.toml`. Windows Developer Mode allows the symbolic links used by the V8 build, including when the Cargo cache and repository are on different drives.

Close any Dodex instance running from the build output before rebuilding that executable. Start in the repository root:

On machines with limited RAM, add `-j 1` to the Cargo build command to run one build job at a time.

```powershell
$env:CODEX_REPO_ROOT = (Get-Location).Path
$v8Settings = python -c 'import sys,json; sys.path.insert(0,"scripts"); from codex_package.v8 import resolve_codex_v8_cargo_env; from codex_package.targets import TARGET_SPECS; print(json.dumps(resolve_codex_v8_cargo_env(TARGET_SPECS["x86_64-pc-windows-msvc"])))' | ConvertFrom-Json
if ($LASTEXITCODE -ne 0) { throw 'V8 setup failed' }
foreach ($setting in $v8Settings.PSObject.Properties) {
    [Environment]::SetEnvironmentVariable($setting.Name, $setting.Value, 'Process')
}

Push-Location codex-rs
cargo build --locked --bin codex --bin codex-code-mode-host --bin codex-exec-server --bin codex-command-runner --bin codex-windows-sandbox-setup
$buildResult = $LASTEXITCODE
Pop-Location
if ($buildResult -ne 0) { throw 'Compilation failed' }

python scripts/build_codex_package.py --target x86_64-pc-windows-msvc --package-dir dist/windows-x64 --entrypoint-bin codex-rs/target/debug/codex.exe --code-mode-host-bin codex-rs/target/debug/codex-code-mode-host.exe --exec-server-bin codex-rs/target/debug/codex-exec-server.exe --codex-command-runner-bin codex-rs/target/debug/codex-command-runner.exe --codex-windows-sandbox-setup-bin codex-rs/target/debug/codex-windows-sandbox-setup.exe --rg-bin (Get-Command rg.exe).Source --strip none
if ($LASTEXITCODE -ne 0) { throw 'Packaging failed' }

.\dist\windows-x64\bin\codex.exe --version
.\dist\windows-x64\bin\codex.exe
```

V8 setup downloads the pinned prebuilt library and bindings and verifies their checksums. Keep the whole package directory together:

```text
dist/windows-x64/
  codex-package.json
  bin/
    codex.exe
    codex-code-mode-host.exe
    codex-exec-server.exe
  codex-resources/
    codex-command-runner.exe
    codex-windows-sandbox-setup.exe
  codex-path/
    rg.exe
```

For another build, choose a new package directory or add `--force` to replace the previous package after closing processes that use it. `--no-daemon` selects an embedded app server when testing a build independently of an existing daemon. Voice and Windows Sandbox service installation require their separate runtime setup.

## Dodex daemon

Launching the packaged CLI normally starts or reuses a shared Dodex background server. Multiple terminals can use it from different projects. Official Codex uses its own daemon; Dodex does not replace or stop it. Login, configuration, and conversation history continue to use the existing `CODEX_HOME`.

Dodex keeps daemon state under `$CODEX_HOME/dodex-server-daemon/`, its control socket under `$CODEX_HOME/dodex-server-control/`, and complete backend packages under `$CODEX_HOME/packages/dodex-server-daemon/`. Startup checks the Dodex extension protocol before restoring Commentary.

Manage it with the packaged executable:

```powershell
.\dist\windows-x64\bin\codex.exe app-server daemon start
.\dist\windows-x64\bin\codex.exe app-server daemon version
.\dist\windows-x64\bin\codex.exe app-server daemon stop
```

After installing another Dodex build, select its backend with that build's executable:

```powershell
.\dist\windows-x64\bin\codex.exe app-server daemon update --from-cli
```

The replacement can restart a running Dodex daemon and interrupt connected terminals; the command asks for confirmation. Official Codex release downloads are disabled for the Dodex daemon, because those packages do not contain Dodex features. A newer frontend does not silently replace a running backend. `--no-daemon` remains available for an embedded server, and a shell wrapper that supplies it must omit it to use the daemon.

---

# Features

All features are currently being tested.

## Commentary

In temporary `/btw` conversations, comments and unfinished drafts stay in memory while switching views. Closing the temporary conversation discards them; they are not written to saved session history.

Select passages from the clanker's messages and leave comments on them. Useful for answering questions, asking follow-ups, or giving feedback on several parts of a response. Once you're done, review your comments and send them together as a single message.

## Outline

Outline is a conversation-owned, editable Markdown reference for implementation. Run `/outline start`, select an assistant message, and name the document. The selected text is copied exactly, without a model call. A bordered excerpt stays at the top of the terminal while keyboard focus remains in the composer. Alt+R or `/outline view` opens the expanded reader; the wheel scrolls the excerpt or reader.

| Command | Action |
|---|---|
| `/outline` | Expand the active document, or choose one to resume when mode is off |
| `/outline start` | Copy an assistant message into a new named document |
| `/outline resume [id]` | Activate one of this conversation's documents |
| `/outline view` | Read the selected document without changing activation |
| `/outline edit` | Edit the selected document; Enter saves, Shift+Enter inserts a newline, Ctrl+G opens the external editor, Ctrl+D preserves a recovery draft, Esc cancels |
| `/outline history` | Select and confirm restoration of a saved revision |
| `/outline path` | Show the editable Markdown file |
| `/outline off` | Hide the excerpt and stop new Outline guidance, references, and maintenance tools |
| `/outline delete` | Confirm logical deletion; remove the document from normal use while preserving historical quotation sources |

Several documents may belong to a conversation; one is active at a time. While active, agreed Outline requirements and decisions guide implementation ahead of older conversational proposals. System/developer and applicable repository instructions and the user's latest explicit instructions still take precedence. Proposals and open questions are not automatically approved requirements, and activation does not put the agent into planning-only mode.

Each ordinary model request receives one current reference. Small documents are supplied in full; large documents use a bounded reference of roughly 7,000 document characters plus metadata and guidance, containing headings and locally selected excerpts. Partial references are explicitly marked. The agent can request omitted lines with `outline_read` and make revision-checked edits with `outline_update`. Guidance asks it to avoid redundant reads and consolidate writes at meaningful checkpoints: agreed decisions, scope changes, verified completion, or blockers. Maintenance frequency is instruction-driven, not a promise of model compliance. Local viewing, editing, scrolling, restoration, and file refreshes make no model calls; references and model tool exchanges still consume input/output tokens.

Accepted content, activation, and immutable revisions are embedded as model-invisible checkpoints in conversation history. Forks inherit the document state at their branch point and then evolve independently. Compaction does not delete the documents. Raw session history contains the document checkpoints; rendered transcript exports do not automatically show them. The Markdown editing projection lives at `$CODEX_HOME/dodex/outlines/<thread-id>/<document-id>/outline.md`; recovery drafts are in that document's `drafts/` directory. Back up the conversation history for accepted revisions and the projection directory for unsaved recovery drafts.

UI and model edits share the backend's revision checks. A stale editor save preserves its draft instead of overwriting newer content. External file saves are imported against the last projected revision; detected conflicts preserve a recovery draft and disable mode with a warning. External editors do not participate in the backend transaction and should use atomic saves. Documents are limited to 1 MiB. Only observed external saves become accepted revisions.

Commentary works in the pinned and expanded views. With Commentary enabled, drag over a passage to comment; the expanded view also supports Alt+C for a selection and Q to choose a quotation. Notes use the existing editor, outbox, review, and batch submission. Each quotation identifies its document and revision, so edits and logical deletion preserve the original source and do not move old highlights onto newer text.

Deactivation and deletion affect future requests. They do not erase earlier discussion, tool results, or a request already in flight. Outline requires a backend containing this feature; when upgrading an existing Dodex daemon, use the complete new package's `app-server daemon update --from-cli` command.

## Conversation timers and status line

The default status line follows the configured Dodex layout, with `Current HH:MM:SS` and `Total HH:MM:SS` immediately after the thread name. Current measures this opening of the conversation; total accumulates open time across visits, starting when tracking is installed. Closed time is excluded. Customize the items with `/statusline` (`session-current` and `session-total`).

## Transcript navigation

Ctrl+mouse wheel scrolls to the nearest user or assistant message in that direction, including assistant commentary. Messages align with the top of the transcript when possible; scrolling stops at the normal bottom boundary. Ctrl+Up/Down scrolls one rendered row. Terminal applications must receive these modified wheel/key events for the bindings to work.

## Thought visibility

Thoughts start hidden. `/thoughts` toggles received reasoning text and its live status headings; the preference is remembered for each conversation. Hidden reasoning remains retained, and a generic Thinking indicator remains visible during reasoning. The command only changes presentation: it does not send a model request or change reasoning effort.

Timer totals and visibility preferences are stored locally under `$CODEX_HOME/dodex/conversation-ui/`. They are separate from model context and transcript exports. Timers checkpoint approximately every ten seconds and on closing a conversation; an abrupt exit can lose recent uncheckpointed time. Concurrent windows add their respective open durations to the same total.

# Notes

Run `bin/codex.exe` from the complete package and preserve this directory structure:

```text
codex-package.json
bin/
  codex.exe
  codex-code-mode-host.exe
  codex-exec-server.exe
codex-resources/
  codex-command-runner.exe
  codex-windows-sandbox-setup.exe
codex-path/
  rg.exe
```

Building or copying only `codex.exe` does not supply all required runtime helpers.

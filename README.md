# Dodex

Current source version: **0.166.0-dodex.1**.

Upstream base: OpenAI's development `main` at [`a09812981d`](https://github.com/openai/codex/commit/a09812981de703f18f38015cd5d500ab373f616c) (November 1, 2026). This is not the `rust-v0.166.0` stable release tag.

A Codex fork with a few personal extensions to the terminal interface, implemented directly in the source code.

Refer to the original README at [openai/codex](https://github.com/openai/codex).

## Compile on Windows x64

Use PowerShell with Git, Python 3.13, Rustup, ripgrep (`rg`), and Visual Studio Build Tools with Desktop development with C++ and a Windows SDK installed. Cargo uses the Rust version pinned in `codex-rs/rust-toolchain.toml`. Windows Developer Mode allows the symbolic links used by the V8 build, including when the Cargo cache and repository are on different drives.

Close any Dodex instance running from the build output before rebuilding that executable. Start in the repository root:

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

---

# Features

All features are currently being tested.

## Commentary

Select passages from the clanker's messages and leave comments on them. Useful for answering questions, asking follow-ups, or giving feedback on several parts of a response. Once you're done, review your comments and send them together as a single message.

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

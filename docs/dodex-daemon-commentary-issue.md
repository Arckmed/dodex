# Dodex daemon incompatibility and reported paste latency

Recorded: 2026-11-02. Status: deferred; no fix implemented during this investigation.

## Problem

After launching the separately packaged typing-delay fix, a second terminal displayed:

```text
Could not restore Commentary: thread/dodex/commentary/get failed:
Invalid request: unknown variant `thread/dodex/commentary/get`, expected one of ...
(code -32600)
```

The user also reported that pasting was very slow. The relationship between paste latency and the server mismatch has not been established.

## Verified evidence

The process inspection during the investigation showed:

| Process | Executable | Arguments |
| --- | --- | --- |
| First fixed Dodex terminal | `D:\codex-build-cache\dodex-typing-fix-package\bin\codex.exe` | `--no-daemon resume` |
| Second fixed Dodex terminal | Same executable | No arguments |
| Official shared daemon | `C:\Users\r0x\.codex\packages\app-server-daemon\releases\0.166.0-x86_64-pc-windows-msvc\bin\codex.exe` | `app-server --listen unix:// ... --managed-daemon` |

The fixed package manifest reports version `0.166.0-dodex.1`.

Source inspection established:

- `codex-rs/tui/src/app/commentary.rs` requests `ClientRequest::ThreadCommentaryGet` with a thread ID to restore saved Commentary. Its error handler displays the complete server error.
- `codex-rs/app-server-protocol/src/protocol/common.rs` defines the custom method `thread/dodex/commentary/get`.
- `codex-rs/tui/src/daemon_startup.rs` excludes shared-daemon startup when `--no-daemon` is supplied.
- `codex-rs/tui/src/startup_orchestration.rs` invokes `codex_app_server_daemon::start_with_features` for eligible automatic daemon startup.
- `codex-rs/app-server-daemon/src/managed_install.rs` resolves the managed daemon package under the Codex home, including `packages/app-server-daemon`.
- Paste buffering and redraw handling are in the local frontend, including `codex-rs/tui/src/chatwidget/interaction.rs` and `codex-rs/tui/src/app/owned_transcript.rs`.

## Diagnosis and limits

The error proves that the receiving server does not recognize the custom Dodex request. The running processes and startup code strongly support the diagnosis that the second Dodex terminal connected to the official shared daemon. The specific live socket connection was not independently mapped to that process.

Building the client into a separate folder does not isolate daemon discovery or the installed daemon package. The first terminal bypassed this issue through `--no-daemon` and used the embedded server implementation.

Commentary is presented by the terminal, but the current restoration mechanism delegates retrieval of its saved state to the server. Consequently, the server needs the custom protocol support; it does not need to render the interface. Moving persistence entirely into the terminal would be a separate architectural change, including coordination of concurrent reads and writes.

No evidence currently establishes that the daemon mismatch causes slow pasting. Both terminals used the same fixed frontend executable. Investigate local input processing and rendering if the symptom persists with a compatible backend.

## User requirement and proposed approaches

The user explicitly wants to retain the daemon feature and parallel terminals for different projects. Disabling the daemon is only a temporary workaround.

Options discussed, with no implementation choice approved:

1. Replace the existing shared official daemon with Dodex. This changes the backend used by other clients and requires accounting for official updates replacing it.
2. Give Dodex a dedicated shared daemon identity and lifecycle. This was the recommended proposal: reuse existing daemon machinery while separating discovery/socket, lifecycle state, installation, and update ownership from official Codex.

For the second option, inspect storage boundaries before deciding whether history, authentication, and settings can remain shared. Do not assume a separate Codex home is required or silently migrate user data.

Also proposed: validate Dodex protocol support before restoring Commentary and report a concise compatibility error if the server is incompatible. Hiding the current error alone would leave Commentary unavailable.

## Resume work

1. Confirm the desired daemon ownership arrangement with the user.
2. Trace daemon discovery, startup, package selection, updater ownership, and connection compatibility checks end to end.
3. Implement only the approved arrangement, preserving currently running instances and unrelated working-tree edits.
4. Verify that two Dodex terminals in different projects share the intended compatible daemon and restore Commentary successfully. If coexistence is chosen, verify official Codex remains independent.
5. Reproduce paste latency with the compatible backend. Distinguish bracketed paste from terminals delivering individual key events; measure before attributing a cause or claiming a fix.

Temporary launch workaround from a project terminal:

```powershell
& 'D:\codex-build-cache\dodex-typing-fix-package\bin\codex.exe' --no-daemon
```

Append `resume` to resume a conversation. No processes were stopped and no application source was changed during the investigation.

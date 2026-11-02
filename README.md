# Dodex

A Codex fork with a few personal extensions to the terminal interface, implemented directly in the source code.

Refer to the original README at [openai/codex](https://github.com/openai/codex).

---

# Features

All features are currently being tested.

## Commentary

Select passages from the clanker's messages and leave comments on them. Useful for answering questions, asking follow-ups, or giving feedback on several parts of a response. Once you're done, review your comments and send them together as a single message.

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

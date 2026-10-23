# ssh-manager

A personal SSH workbench for you and your AI agents.

[简体中文](README.zh-CN.md)

ssh-manager keeps your servers, logins, credentials and project notes in one place, so an agent can find the right machine and run commands on it without you pasting IPs, ports, usernames or passwords into the chat. It drives the system OpenSSH client; nothing needs to be installed on the servers.

- **Your own host catalog.** Import hosts from `~/.ssh/config` or add them by hand. After import, ssh-manager uses its own catalog and never edits your SSH files.
- **Credentials stay local.** Passwords and key passphrases live in the system keychain. Agents use saved logins but never see the secrets.
- **Separate host trust.** ssh-manager keeps its own known_hosts. Imported hosts bring their records from `~/.ssh/known_hosts`, a new server is recorded on its first connection, and connections stop if a server's identity changes.
- **Built for agents.** Every command has JSON output, and a local MCP server exposes the same features. Results report clearly whether a command succeeded, failed, or has an unknown outcome.
- **Context for each host.** Project paths, descriptions and agent notes are saved with each host, so a new session can pick up where the last one left off.

> Status: early development. Verified on a macOS client against Linux servers. On Linux clients, password storage is not available yet (keys and ssh-agent work). Windows is not supported yet, and the desktop app is a first preview. CLI messages are currently in Chinese; the app can switch between Chinese and English.

## Install

Requires Rust 1.89+ and OpenSSH (`ssh`, `sftp`, `ssh-keygen`, `ssh-keyscan`).

```sh
scripts/install.sh      # installs ssh-manager and ssh-manager-credentials to ~/.local/bin
ssh-manager doctor
```

## Quick start

```sh
# Preview hosts from ~/.ssh/config, then import the ones you want
ssh-manager host import
ssh-manager host import --select web --apply

# Or add one by hand
ssh-manager host add web --address example.com --user deploy --key ~/.ssh/id_ed25519

# Optional: pin the server key before the first connection instead of recording it then
ssh-manager trust import web                                # from ~/.ssh/known_hosts
ssh-manager trust accept web --fingerprint 'SHA256:...'     # a fingerprint you verified

# Save a password or key passphrase (prompted, never echoed)
ssh-manager credential set web
ssh-manager credential set web --passphrase

# Use it
ssh-manager check web
ssh-manager connect web            # interactive shell, saved password filled in
ssh-manager exec web -- 'df -h'
ssh-manager upload web ./dist /srv/site --mode contents
ssh-manager download web /var/log/app.log ./app.log
```

Add context so agents know what lives where:

```sh
ssh-manager context describe web 'Personal website'
ssh-manager context project web blog --path /srv/blog --description 'Docker Compose'
ssh-manager exec web --project blog -- 'docker compose ps'
```

The import preview shows which options were imported, which were ignored as irrelevant (such as `UseKeychain`), and which need your attention. `Match` blocks are never executed. Hosts with open issues won't run until you fix them and confirm with `host edit NAME --clear-issues`.

Add `--json` to any command for structured output. See `ssh-manager --help` for everything else, including multiple logins per host, jump hosts, custom agent sockets (1Password, Secretive), `history` and `export`/`restore`.

## Desktop app

A Tauri desktop app shares the same catalog as the CLI and MCP server. It shows your hosts with live reachability (a login-free probe every few minutes while the window is open), connection paths, trust and login status, plus an operation history you can filter by caller and host. You can import from `~/.ssh/config`, edit a host's address, user and login method, save passwords, open a host in your terminal (Terminal, Ghostty or iTerm), and add ssh-manager to Claude Code, Codex or Claude Desktop in one click. Settings cover language, appearance and the default terminal.

```sh
scripts/install.sh                      # the app uses the installed credentials helper
cargo run --release -p ssh-manager-desktop
```

## Use with agents

Register the local MCP server (stdio only, no network port). For Claude Code:

```sh
claude mcp add ssh-manager -- ~/.local/bin/ssh-manager mcp
```

Other clients:

```json
{ "mcpServers": { "ssh-manager": { "command": "/absolute/path/to/ssh-manager", "args": ["mcp"] } } }
```

Tools: `host_list`, `host_show`, `check`, `exec`, `upload`, `download` and `context_note`. Managing credentials and host trust stays in the CLI.

[`skills/ssh-manager/SKILL.md`](skills/ssh-manager/SKILL.md) is a skill that teaches agents the workflow: find the host, read its context, run the command, and interpret the result.

## Good to know

- `exec` runs any shell command with the permissions of that SSH account. For a real read-only boundary, use a restricted server account.
- `result_unknown` means the connection dropped, timed out or was cancelled. The remote command may still be running, so check before retrying anything with side effects.
- MFA, keyboard-interactive and sudo passwords are never filled in automatically.
- Operation history records the target, time and status, but not command text or output.

## Use as a library

`crates/core` (`ssh-manager-core`) is a standalone Rust library that the CLI and MCP server are built on. Embed it with `Store`, `OpenSsh` and your own `CredentialStore`. See `crates/core/examples/check.rs`.

## License

MIT

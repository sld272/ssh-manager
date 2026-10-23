# ssh-manager

给你和 AI agent 用的个人 SSH 工作台。

[English](README.md)

ssh-manager 把服务器、登录方式、凭据和项目说明集中管理。agent 可以自己找到要操作的机器并执行命令，你不用再往对话里贴 IP、端口、账号或密码。它调用系统自带的 OpenSSH，服务器上不需要安装任何东西。

- **自己的主机库**：从 `~/.ssh/config` 导入或手动添加。导入后以 ssh-manager 的主机库为准，不会改动你的 SSH 配置文件。
- **凭据留在本机**：密码和密钥口令存在系统钥匙串里。agent 可以使用已保存的登录方式，但看不到密码本身。
- **独立的主机信任**：ssh-manager 使用自己的 known_hosts，服务器身份变化时会停止连接。
- **为 agent 设计**：所有命令都有 JSON 输出，另有本地 MCP 服务提供同样的能力。结果会明确区分成功、失败和结果未知。
- **主机上下文**：项目路径、用途说明和 agent 记录随主机一起保存，新会话可以接着上次的进度。

> 状态：早期开发中。已在 macOS 客户端连接 Linux 服务器的环境下实测。Linux 客户端暂不支持保存密码（密钥和 ssh-agent 可用）。Windows 和图形界面尚未推出。

## 安装

需要 Rust 1.89+ 和 OpenSSH（`ssh`、`sftp`、`ssh-keygen`、`ssh-keyscan`）。

```sh
scripts/install.sh      # 安装 ssh-manager 和 ssh-manager-credentials 到 ~/.local/bin
ssh-manager doctor
```

## 快速上手

```sh
# 预览 ~/.ssh/config 里的主机，再导入需要的
ssh-manager host import
ssh-manager host import --select web --apply

# 或者手动添加
ssh-manager host add web --address example.com --user deploy --key ~/.ssh/id_ed25519

# 信任服务器：沿用 ~/.ssh/known_hosts，或扫描后核对指纹再确认
ssh-manager trust import web
ssh-manager trust scan web
ssh-manager trust accept web --fingerprint 'SHA256:...'

# 保存密码或密钥口令（交互输入，不回显）
ssh-manager credential set web
ssh-manager credential set web --passphrase

# 开始使用
ssh-manager check web
ssh-manager exec web -- 'df -h'
ssh-manager upload web ./dist /srv/site --mode contents
ssh-manager download web /var/log/app.log ./app.log
```

补充上下文，让 agent 知道东西都在哪：

```sh
ssh-manager context describe web '个人网站'
ssh-manager context project web blog --path /srv/blog --description 'Docker Compose'
ssh-manager exec web --project blog -- 'docker compose ps'
```

导入预览会列出哪些选项已导入、哪些因为无关被忽略（比如 `UseKeychain`）、哪些需要你处理。`Match` 条件块不会被执行。有待处理问题的主机不能直接使用，处理完后用 `host edit NAME --clear-issues` 确认。

任何命令加上 `--json` 都会输出结构化结果。其他功能见 `ssh-manager --help`，包括同一主机配置多个登录方式、跳板机、指定 agent 套接字（1Password、Secretive）、`history` 和 `export`/`restore`。

## 接入 agent

注册本地 MCP 服务（只用 stdio，不监听网络端口）。Claude Code：

```sh
claude mcp add ssh-manager -- ~/.local/bin/ssh-manager mcp
```

其他客户端：

```json
{ "mcpServers": { "ssh-manager": { "command": "/absolute/path/to/ssh-manager", "args": ["mcp"] } } }
```

提供的工具有 `host_list`、`host_show`、`check`、`exec`、`upload`、`download` 和 `context_note`。管理凭据和主机信任仍需通过 CLI。

[`skills/ssh-manager/SKILL.md`](skills/ssh-manager/SKILL.md) 是一份 skill，告诉 agent 如何操作：先找到主机，读取上下文，再执行命令，最后解读结果。

## 注意事项

- `exec` 可以执行任意 Shell 命令，拥有对应 SSH 账号的全部权限。需要真正的只读限制时，请在服务器上使用受限账号。
- `result_unknown` 表示连接中断、超时或被取消，远端命令可能仍在运行。重试有副作用的操作前，请先确认实际状态。
- 不会自动填写 MFA、keyboard-interactive 或 sudo 密码。
- 操作记录只保存目标、时间和状态，不保存命令内容和输出。

## 作为库使用

`crates/core`（`ssh-manager-core`）是一个独立的 Rust 库，CLI 和 MCP 服务都基于它构建。通过 `Store`、`OpenSsh` 和你自己实现的 `CredentialStore` 即可嵌入使用，示例见 `crates/core/examples/check.rs`。

## 许可证

MIT

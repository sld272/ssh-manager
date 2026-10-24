# ssh-manager

给你和 AI agent 用的个人 SSH 工作台。

[English](README.md)

ssh-manager 把服务器、登录方式、凭据和项目说明集中管理。agent 可以自己找到要操作的机器并执行命令，你不用再往对话里贴 IP、端口、账号或密码。它调用系统自带的 OpenSSH，服务器上不需要安装任何东西。

- **自己的主机库**：从 `~/.ssh/config` 导入或手动添加。导入后以 ssh-manager 的主机库为准，不会改动你的 SSH 配置文件。
- **凭据留在本机**：密码和密钥密码存在系统钥匙串里。agent 可以使用已保存的登录方式，但看不到密码本身。
- **独立的主机信任**：ssh-manager 使用自己的 known_hosts。导入的主机沿用 `~/.ssh/known_hosts` 里的记录，新服务器在首次连接时记录身份，之后身份变化会停止连接。
- **为 agent 设计**：所有命令都有 JSON 输出，另有本地 MCP 服务提供同样的能力。结果会明确区分成功、失败和结果未知。
- **主机上下文**：项目路径、用途说明和 agent 记录随主机一起保存，新会话可以接着上次的进度。

> 状态：早期开发中。已在 macOS 客户端连接 Linux 服务器的环境下实测。Linux 客户端暂不支持保存密码（密钥和 ssh-agent 可用）。暂不支持 Windows，桌面端是第一个预览版。

## 安装

需要 Rust 1.89+ 和 OpenSSH（`ssh`、`sftp`、`ssh-keygen`、`ssh-keyscan`）。

```sh
scripts/install.sh      # 安装 ssh-manager 和 ssh-manager-credentials 到 ~/.local/bin；在 macOS 上同时把桌面端装到 ~/Applications
ssh-manager doctor
```

## 快速上手

```sh
# 预览 ~/.ssh/config 里的主机，再导入需要的
ssh-manager host import
ssh-manager host import --select web --apply

# 或者手动添加
ssh-manager host add web --address example.com --user deploy --key ~/.ssh/id_ed25519

# 可选：在首次连接前就固定服务器公钥，而不是首次连接时记录
ssh-manager trust import web                                # 来自 ~/.ssh/known_hosts
ssh-manager trust accept web --fingerprint 'SHA256:...'     # 你核对过的指纹

# 保存密码或密钥密码（交互输入，不回显）
ssh-manager credential set web
ssh-manager credential set web --passphrase

# 开始使用
ssh-manager check web
ssh-manager connect web            # 交互式登录，自动填入已保存的密码
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

## 桌面端

基于 Tauri 的桌面应用，与 CLI、MCP 共用同一个主机库。可以查看主机的实时在线状态（窗口打开时每隔几分钟做一次不登录的探测）、连接路径、信任与登录状态，以及可按调用方和主机筛选的操作记录；也可以从 `~/.ssh/config` 导入、修改主机地址、用户和登录方式、保存密码、在终端中打开主机（终端、Ghostty 或 iTerm）。设置中可以切换语言、外观、默认终端，以及复制接入 agent 的提示词。

`scripts/install.sh` 会和 CLI 一起构建桌面端，安装为 `~/Applications/ssh-manager.app`，保证两者版本一致；设置 `SSH_MANAGER_APP_DIR` 可以换一个安装位置。开发时可以用 `cargo run --release -p ssh-manager-desktop` 直接从源码运行，它会使用已安装的凭据组件。

## 接入 agent

ssh-manager 是一个本地 MCP 服务（只用 stdio，不监听网络端口）。各个 AI 工具添加 MCP 服务的方式不同，所以交给 agent 自己完成：把下面这段提示词发给它即可。桌面端的设置页也能一键复制同样的提示词，并自动填好本机的安装路径。

```text
请把 ssh-manager 添加为你的 MCP 服务器，这样你就能使用我在这台 Mac 上保存的 SSH 主机。

- 名称：ssh-manager
- 传输方式：stdio（本地进程，不开放网络端口）
- 命令：ssh-manager 的绝对路径（通常是 ~/.local/bin/ssh-manager，请写成完整路径）
- 参数：mcp

请用你自己支持的方式添加，作用范围是当前用户而不是某个项目：可以用你自带的 `mcp add` 命令，也可以写入你的 MCP 配置文件。不要改动与 ssh-manager 无关的配置。如果需要重启你才能生效，请告诉我。

添加后先调用 host_list，确认能看到我的主机。使用方法见：https://github.com/sld272/ssh-manager/blob/main/skills/ssh-manager/SKILL.md
```

提供的工具有 `host_list`、`host_show`、`check`、`exec`、`job_start`、`job_status`、`job_cancel`、`upload`、`download`、`history` 和 `context_note`。管理凭据和主机信任仍需通过 CLI。

[`skills/ssh-manager/SKILL.md`](skills/ssh-manager/SKILL.md) 是一份 skill，告诉 agent 如何操作：先找到主机，读取上下文，再执行命令，最后解读结果。

## 注意事项

- `exec` 可以执行任意 Shell 命令，拥有对应 SSH 账号的全部权限。需要真正的只读限制时，请在服务器上使用受限账号。
- `result_unknown` 表示连接中断、超时或被取消，远端命令可能仍在运行。重试有副作用的操作前，请先确认实际状态。耗时较长的命令可以改用后台任务（`job start`）：断开连接后继续运行，之后用 `job status` 查看状态和输出。
- 后台任务的输出保存在服务器的 `~/.local/state/ssh-manager/jobs` 中，只有该账号可读；结束超过一周的任务会在下次启动任务时清理。
- 不会自动填写 MFA、keyboard-interactive 或 sudo 密码。
- 操作记录只保存目标、时间、状态和连接耗时，不保存命令内容、输入和输出。

## 作为库使用

`crates/core`（`ssh-manager-core`）是一个独立的 Rust 库，CLI 和 MCP 服务都基于它构建。通过 `Store`、`OpenSsh` 和你自己实现的 `CredentialStore` 即可嵌入使用，示例见 `crates/core/examples/check.rs`。

## 许可证

MIT

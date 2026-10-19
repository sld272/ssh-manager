# ssh-manager

面向个人开发者和本地 AI agent 的 SSH 工作台。保存主机、登录配置、凭据引用和项目上下文，通过系统 OpenSSH 执行命令与传输文件。远端无需安装常驻程序。

核心是独立 Rust 库，CLI 和本地 stdio MCP 共享同一套能力。其他设备互联项目可以直接引用核心库，或通过 JSON / MCP 调用；服务器身份、凭据和任务状态由 ssh-manager 自己维护。

## 当前能力

- 主机增删改查、稳定 ID、标签、同一主机的多套登录配置与有序跳板。
- OpenSSH 静态配置预览与一次性导入：Host、Include、默认规则、多密钥、IdentitiesOnly、ProxyJump。递归检查后才调用 `ssh -G`；Match 条件直接报告，其他未转换选项保留为待处理问题。
- 密钥、ssh-agent、登录密码和密钥口令。密码与口令进入系统凭据存储，数据库只保存引用。
- 独立 known_hosts：导入已有信任，或扫描并核对指纹后确认；身份变化时停止连接。
- 前台命令执行：工作目录、项目选择、超时、取消、分别有界的 stdout/stderr、远端退出码与结果状态。
- SFTP 单文件上传下载、目录内容合并上传；本地下载成功后才替换目标文件。
- 用户说明、项目路径、带时间的系统探测和 agent 探索记录分别保存。
- 操作记录默认保存目标、时间、状态、调用方与目的，不保存命令正文或完整输出。
- 本地 stdio MCP：主机查询、连接检查、执行、传输及探索记录。

已在 **macOS 客户端 → Ubuntu x86-64 虚拟机**进行实际验证。Linux 客户端使用 Secret Service 后端，尚未进行本机运行验收；Windows 客户端、GUI、可靠后台任务恢复和网络服务尚未实现。

## 构建与安装

需要 Rust 1.89+、系统 OpenSSH（`ssh`、`sftp`、`ssh-keygen`、`ssh-keyscan`）。macOS 的实际验证版本为 OpenSSH 9.9p2。远端执行要求 POSIX `sh`，连接检查使用 `uname` 和 `id`，传输要求 SFTP 子系统。

```sh
cargo build --release --locked
cargo test --workspace --locked
cargo install --path crates/cli --locked
ssh-manager doctor
```

Linux 的密码存储需要可用且已解锁的 Secret Service；不可用时明确报错，不建立明文或自制加密存储。密钥和 ssh-agent 可独立使用。凭据访问通过短进程隔离，最多等待五秒；macOS 无交互执行不会弹出钥匙串对话框。未签名开发构建更换后，系统可能要求重新授权；可通过 `credential set` 重新绑定。

## 开始使用

先预览，确认后保存选中的主机。跳板依赖会一同导入，原配置文件不会被修改。

```sh
ssh-manager host import --select vm --json
ssh-manager host import --select vm --apply
ssh-manager host show vm --json

# 仅导入已有 known_hosts 中匹配此地址与端口的记录
ssh-manager trust import vm

# 交互输入，不回显，不把密码写进 shell 历史
ssh-manager credential set vm
ssh-manager check vm --json
ssh-manager exec vm --json -- 'uname -a'
```

也可以手动添加：

```sh
ssh-manager host add web --address server.example.com --user deploy --key ~/.ssh/deploy_key
ssh-manager trust scan web --json
# 通过服务器控制台等可信来源核对扫描结果，再确认指定指纹
ssh-manager trust accept web --fingerprint 'SHA256:核对后的指纹'
ssh-manager credential set web --passphrase
```

扫描获取的公钥候选不等于可信身份。跳板后的目标目前通过 `trust import` 导入已有信任；`trust scan` 只支持直接连接。已有公钥冲突时，需要核实变化后显式使用 `--replace`。

导入结果含 `issues` 时不会执行；可修改登录配置，在处理完列出的选项与依赖后使用 `host edit NAME --clear-issues` 确认。导入不自动透传任意 SSH 选项，也不覆盖已有同名主机。`host export` 输出 JSON，去掉系统凭据绑定；`host restore FILE` 在无冲突时整体导入，仍需在本机绑定凭据和建立主机信任。

## 项目、登录配置与传输

```sh
ssh-manager context describe web '个人网站'
ssh-manager context project web blog --path /srv/blog --description 'Docker Compose'
ssh-manager context note web '已确认由 compose 管理；需要时重新检查运行状态'
ssh-manager exec web --project blog --json -- 'docker compose ps'
ssh-manager exec web --cwd /srv/blog --timeout 30 --json -- 'git status --short'

ssh-manager connection add web admin --address server.example.com --user admin --key ~/.ssh/admin_key
ssh-manager trust import web --connection admin
ssh-manager check web --connection admin --json
ssh-manager connection default web admin

ssh-manager upload web ./index.html /srv/blog/index.html --json
ssh-manager upload web ./dist /srv/blog --mode contents --json
ssh-manager download web /var/log/example.log ./example.log --json
ssh-manager history --host web --json
```

`file` 模式的目标是完整文件名，不根据目标是否存在追加源文件名。`contents` 模式把源目录的内容合并到目标目录，重复上传不会产生 `dist/dist`，不删除目标中多余的文件。目标父目录需要存在。下载目前支持单个普通文件；本地源树中的符号链接和特殊文件不支持。上传失败或取消后，远端可能留有部分文件；没有自动重试、目录同步或事务发布语义。`--timeout` 分别限制路径检查与传输子步骤的等待。

远端 stdout/stderr 按 UTF-8 返回，非法字节使用替代字符；二进制数据请使用文件传输。默认每个流返回最多 64 KiB，继续读取但不保存被截断的完整输出。可通过 `--output-limit` 调整，最多 16 MiB。需要完整日志时显式下载日志文件。

## 结果与权限

CLI 的 `--json` 可用于所有命令。远程操作结果包含 `schema_version`、操作/主机/登录配置 ID、状态、执行阶段、时间、退出码、输出字节数和截断标记。CLI 退出码：`0` 为成功，`1` 为已返回未成功的操作结果，`2` 为参数、配置或本机前置条件错误。远端退出码保存在 JSON 的 `exit_code`，不直接占用 CLI 的退出码。

| 状态 | 含义 |
| --- | --- |
| `succeeded` | 已确认完成 |
| `command_failed` | 已收到远端命令退出码，命令未成功；包括远端主动退出 255 |
| `connection_failed` / `authentication_failed` | 已识别的网络或认证失败 |
| `host_key_required` / `host_key_changed` | 尚未建立信任或服务器身份发生变化 |
| `needs_interaction` | 已配置的认证流程需要用户操作 |
| `result_unknown` | 无法确认最终结果，例如命令启动后断线、超时或取消 |
| `transfer_failed` | 传输没有完成，目标可能有部分内容 |

只有确认收到远端完成标记才填写命令退出码。完成标记用于区分进程生命周期，不是对恶意服务器的完整性证明。取消会停止本机 SSH 进程组，**不保证远端进程已经停止**。未知结果不可用于自动重试部署、重启等有副作用的命令。

MFA、keyboard-interactive、硬件密钥确认和 sudo 密码不会自动填入。命令无 PTY、stdin 为 EOF；项目不会全局改变包管理器或分页器环境。执行任意 Shell 命令拥有对应 SSH 账号的权限；需要权限隔离时使用受限账号。主机说明、探索记录和远端输出都是参考数据，不自动成为执行指令。

`--reason` 和 `--caller` 会进入本机操作记录，请勿填写秘密。配置、数据库和项目上下文本身也可能包含敏感地址与说明，按本机私有数据处理。

## 接入其他项目

直接引用 `ssh-manager-core`，使用 `Store`、`Host`、`OpenSsh`、`ExecutionOptions` 和 `OperationResult`。凭据通过 `CredentialStore` trait 注入，可复用调用方的授权与安全存储。核心不依赖 CLI 参数解析、MCP、全局异步运行时或 PairDock。

系统凭据和 askpass 的默认适配需要一个配套 CLI 路径；嵌入应用传入安装的 `ssh-manager`，核心不会假设宿主应用能充当认证助手。可运行示例验证这种集成：

```sh
cargo run -p ssh-manager-core --example check -- /absolute/path/to/ssh-manager vm
```

本地 MCP 配置示例：

```json
{
  "mcpServers": {
    "ssh-manager": {
      "command": "/absolute/path/to/ssh-manager",
      "args": ["mcp"]
    }
  }
}
```

支持 MCP `2025-11-25`、结构化工具结果、并发调用和取消通知；仅使用 stdin/stdout，不监听端口。stdout 只输出协议消息。工具没有独立的权限沙箱。绑定凭据、修改主机信任等管理操作保留在 CLI。

## 数据与验证

`doctor --json` 返回本机数据目录；`--data-dir PATH` 可为集成应用或实验使用独立主机库。macOS 默认使用 Application Support，Linux 使用 XDG 数据目录。数据目录权限为 `0700`，数据库、主机信任文件和临时连接配置为 `0600`。每次操作生成独立配置快照，不持续读取原来的 `~/.ssh/config`。

```sh
# 使用已经绑定凭据、已建立信任的测试主机；不会读取凭据值
python3 scripts/verify_vm.py --binary target/release/ssh-manager --host vm
```

实机脚本复制本机主机库到临时目录，用随机的远端 `/tmp` 目录验证命令和文件操作，并临时添加、移除测试公钥。它会进行一次故意失败的登录来核对认证状态，应只用于自己的测试机器。实际输出和报告保存在忽略的 `.local/verification`，不会提交。

Git 中除 README 外不保存设计文档、skill 说明或验证报告；这些本机材料放在 `.local`。

本次 macOS → Ubuntu VM 验证：**10 项核心测试、26 项实机场景通过**。实机覆盖密码、加密密钥口令、ssh-agent、密码跳板、工作目录与上下文、退出码 255、超时与取消、会话中断、大输出截断、特殊文件名的二进制往返校验、重复目录上传、MCP 执行/取消/关闭及主机信任变化。验证为有限时长操作，不代表后台任务恢复或跨平台验收完成。

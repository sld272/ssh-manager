import { h, icon, screen, toast } from './dom.js';
import { language, t } from './i18n.js';

const SKILL = 'https://github.com/sld272/ssh-manager/blob/main/skills/ssh-manager/SKILL.md';

// Each AI tool registers MCP servers its own way, so the prompt states what to register and leaves the how to the tool.
function agentPrompt(command) {
  return language() === 'en'
    ? `Add ssh-manager as one of your MCP servers so you can use the SSH hosts I keep on this Mac.

- Name: ssh-manager
- Transport: stdio (a local process; it opens no network port)
- Command: ${command}
- Arguments: mcp

Add it the way you support, for the current user rather than one project: your own \`mcp add\` command or your MCP config file. Do not change anything unrelated to ssh-manager. Tell me if I need to restart you for it to take effect.

Once it is added, call host_list to confirm you can see my hosts. How to use it: ${SKILL}`
    : `请把 ssh-manager 添加为你的 MCP 服务器，这样你就能使用我在这台 Mac 上保存的 SSH 主机。

- 名称：ssh-manager
- 传输方式：stdio（本地进程，不开放网络端口）
- 命令：${command}
- 参数：mcp

请用你自己支持的方式添加，作用范围是当前用户而不是某个项目：可以用你自带的 \`mcp add\` 命令，也可以写入你的 MCP 配置文件。不要改动与 ssh-manager 无关的配置。如果需要重启你才能生效，请告诉我。

添加后先调用 host_list，确认能看到我的主机。使用方法见：${SKILL}`;
}

function group(title, ...rows) {
  return h('section', { class: 'settings-group' },
    h('h2', { class: 'section-title' }, title),
    h('div', { class: 'card settings-card' }, rows));
}

function row(label, control, hint) {
  return h('div', { class: 'setting-row' },
    h('div', { class: 'grow stack' }, h('span', null, label), hint && h('span', { class: 'small muted' }, hint)),
    control);
}

function segmented(label, options, value, onPick) {
  return h('div', { class: 'segmented', role: 'group', 'aria-label': label },
    options.map(([key, text]) => h('button', {
      type: 'button', 'aria-pressed': String(value === key), onclick: () => onPick(key),
    }, text)));
}

function status(ok, text) {
  return h('span', { class: `state ${ok ? 'ok' : 'warn'}`, style: 'font-size: 12px' }, h('span', { class: `dot ${ok ? 'ok' : 'warn'}` }), text);
}

export function settingsView(state, actions) {
  const view = state.settings;
  const s = view.settings;
  const env = view.environment;
  const change = (patch) => actions.saveSettings({ ...s, ...patch });

  const select = (label, options, value, onPick) => h('select', { class: 'select', 'aria-label': label, onchange: (e) => onPick(e.target.value) },
    options.map(([key, text]) => h('option', { value: key, selected: value === key }, text)));
  const language = select(t('语言'), [['system', t('跟随系统')], ['zh-CN', '简体中文'], ['en', 'English']], s.language, (v) => change({ language: v }));
  // Only installed terminals are listed.
  const terminal = select(t('默认终端'), view.terminals.map((app) => [app.id, app.id === 'terminal' ? t('系统终端') : app.name]), s.terminal, (v) => change({ terminal: v }));

  return screen({
    title: t('设置'),
    body: h('div', { class: 'main-body' }, h('div', { class: 'page settings' },
      group(t('通用'),
        row(t('语言'), language),
        row(t('外观'), segmented(t('外观'), [['system', t('跟随系统')], ['light', t('浅色')], ['dark', t('深色')]], s.theme, (theme) => change({ theme })))),
      group(t('终端'),
        row(t('默认终端'), terminal, t('在主机详情里点“连接”时打开。'))),
      group('Agent',
        row(t('接入 AI 工具'),
          h('button', {
            type: 'button', class: 'btn btn-sm', disabled: !env.command,
            onclick: () => navigator.clipboard.writeText(agentPrompt(env.command)).then(() => toast(t('已复制，发给你的 AI 工具即可'))),
          }, icon('copy', 13), t('复制接入提示词')),
          t(env.command ? '把提示词发给 Claude Code、Codex 等 AI 工具，由它自己完成接入。' : '请先安装命令行工具，AI 工具通过它连接。'))),
      group(t('本机环境'),
        row('OpenSSH', status(!!env.ssh_version, env.ssh_version ?? t('未找到'))),
        row(t('凭据组件'), status(env.credential_helper_available, t(env.credential_helper_available ? '可用' : '未找到'))),
        row(t('命令行工具'), env.command ? h('span', { class: 'mono small muted ellipsis', style: 'max-width: 280px' }, env.command) : status(false, t('未安装'))),
        row(t('数据目录'),
          h('button', { type: 'button', class: 'btn btn-sm', onclick: () => actions.revealDataDir() }, icon('folder', 13), t('在访达中显示')),
          h('span', { class: 'mono small muted ellipsis', title: env.data_dir }, env.data_dir.replace(/^\/Users\/[^/]+/, '~')))),
      group(t('关于'),
        row(t('版本'), h('span', { class: 'small muted' }, `ssh-manager ${env.version}`))))),
  });
}

export function environmentReady(view) {
  const env = view?.environment;
  return !env || (!!env.ssh_version && env.credential_helper_available);
}

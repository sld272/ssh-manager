import { h, icon, screen } from './dom.js';
import { t } from './i18n.js';

// One row per MCP client installed on this Mac; adding writes ssh-manager into that client's own config.
function clientRow(client, state, actions) {
  const busy = state.mcpBusy.has(client.id);
  return h('div', { class: 'setting-row' },
    h('div', { class: 'grow stack' },
      h('span', { style: 'font-weight: 600' }, client.name),
      client.connected && h('span', { class: 'state ok small' }, h('span', { class: 'dot ok' }), t('已接入')),
      client.connected && client.id === 'claude-desktop' && h('span', { class: 'small muted' }, t('重新打开 Claude 后生效'))),
    client.connected
      ? h('button', { type: 'button', class: 'btn btn-sm', disabled: busy, onclick: () => actions.setMcp(client, false) }, t('移除'))
      : h('button', { type: 'button', class: 'btn btn-sm btn-primary', disabled: busy || !state.mcp.command, onclick: () => actions.setMcp(client, true) }, busy ? t('接入中…') : t('接入')));
}

export function agentsView(state, actions) {
  const data = state.mcp;
  return screen({
    title: t('Agent 接入'),
    body: h('div', { class: 'main-body' }, h('div', { class: 'page settings' },
      h('p', { class: 'lead small' }, t('接入后，AI 工具可以用你保存的主机检查连接、执行命令和传输文件，但看不到密码和私钥。它们做过的操作记录在“活动”里。')),
      data?.error && h('div', { class: 'banner err' }, icon('alert', 16), h('div', { class: 'grow' }, data.error)),
      data && !data.error && !data.command && h('div', { class: 'banner' }, icon('alert', 16),
        h('div', { class: 'grow' }, t('没有找到已安装的 ssh-manager 命令行工具。agent 通过它连接，请先安装。'))),
      h('section', { class: 'settings-group' },
        h('h2', { class: 'section-title' }, t('本机的 AI 工具')),
        h('div', { class: 'card settings-card' },
          !data ? h('div', { class: 'setting-row muted' }, t('读取中…'))
            : data.clients?.length ? data.clients.map((c) => clientRow(c, state, actions))
              : h('div', { class: 'setting-row muted' }, t('没有找到支持的 AI 工具。目前支持 Claude Code、Codex 和 Claude Desktop。')))),
      h('p', { class: 'small muted' }, t('agent 执行命令时拥有对应 SSH 账号的全部权限。需要只读时，请在服务器上使用受限账号。')))),
  });
}

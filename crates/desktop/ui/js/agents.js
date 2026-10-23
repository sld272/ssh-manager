import { h, icon, toast } from './dom.js';
import { relative } from './format.js';

function copyBox(text) {
  return h('div', { class: 'code' },
    h('pre', null, text),
    h('button', {
      type: 'button', class: 'icon-btn', 'aria-label': '复制',
      onclick: () => navigator.clipboard.writeText(text).then(() => toast('已复制')),
    }, icon('copy', 16)));
}

function mark(name) {
  const parts = name.split(/[-_\s.]+/).filter(Boolean);
  return (parts.length > 1 ? parts[0][0] + parts[1][0] : name.slice(0, 2)).toUpperCase();
}

export function agentsView(state) {
  const data = state.agents;
  const command = data?.command;
  const json = JSON.stringify({ mcpServers: { 'ssh-manager': { command: command ?? '/path/to/ssh-manager', args: ['mcp'] } } }, null, 2);

  return h('div', { class: 'main' }, h('div', { class: 'page' },
    h('div', null,
      h('h1', { class: 'page-title' }, 'Agent 接入'),
      h('p', { class: 'page-sub' }, '让本机的 AI 工具使用你保存的主机。agent 只能用已保存的登录方式，看不到密码和私钥。')),

    h('div', { class: 'agents' },
      h('section', { class: 'clients', 'aria-label': '用过的客户端' },
        h('div', { class: 'section-title', style: 'padding: 0 4px 4px' }, '用过 ssh-manager 的客户端'),
        !data ? h('p', { class: 'muted' }, '读取中…')
          : data.clients.length
            ? data.clients.map((c) => h('div', { class: 'client' },
              h('span', { class: 'client-mark' }, mark(c.caller)),
              h('span', { class: 'grow stack' },
                h('span', { style: 'font-weight: 600' }, c.caller),
                h('span', { class: 'small', style: 'color: var(--text2)' }, `${c.operations} 次操作 · ${relative(c.last_used)}使用`))))
            : h('div', { class: 'client', style: 'border-style: dashed' },
              h('span', { class: 'small', style: 'color: var(--text2)' }, '还没有 agent 用过 ssh-manager。按右侧步骤接入后，会显示在这里。'))),

      h('section', { class: 'card setup', 'aria-label': '接入方式' },
        h('h2', { style: 'font-size: 20px; font-weight: 700' }, '接入方式'),
        data && !command && h('div', { class: 'banner' }, icon('alert', 18),
          h('div', { class: 'grow' }, '没有找到已安装的 ssh-manager 命令行工具。agent 通过它连接，请先安装。')),
        h('div', { class: 'section' },
          h('h3', { class: 'section-title' }, 'Claude Code'),
          copyBox(`claude mcp add ssh-manager -- ${command ?? '/path/to/ssh-manager'} mcp`)),
        h('div', { class: 'section' },
          h('h3', { class: 'section-title' }, '其他 MCP 客户端'),
          copyBox(json)),
        h('div', { class: 'section' },
          h('h3', { class: 'section-title' }, 'agent 可以做的事'),
          h('ul', { class: 'can-list' },
            ['查看主机、项目和记录', '检查连接、执行命令', '上传和下载文件', '写下自己的发现，不会覆盖你的说明']
              .map((t) => h('li', null, h('span', { class: 'status-text ok' }, icon('check', 16, 2)), t)),
            ['查看密码或私钥', '保存凭据、修改服务器信任']
              .map((t) => h('li', null, h('span', { class: 'status-text err' }, icon('x', 16, 2)), `不能${t}`)))),
        h('p', { class: 'small', style: 'color: var(--text2)' }, '执行命令拥有对应 SSH 账号的全部权限。需要只读时，请在服务器上使用受限账号。')))));
}

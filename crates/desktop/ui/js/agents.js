import { h, icon, screen, toast } from './dom.js';
import { relative } from './format.js';
import { t } from './i18n.js';

function copyBox(text) {
  return h('div', { class: 'code' },
    h('pre', null, text),
    h('button', {
      type: 'button', class: 'icon-btn', 'aria-label': t('复制'), title: t('复制'),
      onclick: () => navigator.clipboard.writeText(text).then(() => toast(t('已复制'))),
    }, icon('copy', 14)));
}

function mark(name) {
  const parts = name.split(/[-_\s.]+/).filter(Boolean);
  return (parts.length > 1 ? parts[0][0] + parts[1][0] : name.slice(0, 2)).toUpperCase();
}

export function agentsView(state) {
  const data = state.agents;
  const command = data?.command;
  const binary = command ?? '/path/to/ssh-manager';
  const json = JSON.stringify({ mcpServers: { 'ssh-manager': { command: binary, args: ['mcp'] } } }, null, 2);

  return screen({
    title: t('Agent 接入'),
    body: h('div', { class: 'main-body' }, h('div', { class: 'page' },
      h('p', { class: 'lead small' }, t('让本机的 AI 工具使用你保存的主机。agent 只能用已保存的登录方式，看不到密码和私钥。')),
      h('div', { class: 'agents' },
        h('section', { class: 'section', 'aria-label': t('用过的客户端') },
          h('h3', { class: 'section-title' }, t('用过的客户端')),
          h('div', { class: 'clients' },
            !data ? h('p', { class: 'muted' }, t('读取中…'))
              : data.clients.length
                ? data.clients.map((c) => h('div', { class: 'client' },
                  h('span', { class: 'client-mark' }, mark(c.caller)),
                  h('span', { class: 'grow stack' },
                    h('span', { class: 'ellipsis', style: 'font-weight: 600' }, c.caller),
                    h('span', { class: 'small muted' }, t('{n} 次操作 · {when}', { n: c.operations, when: relative(c.last_used) })))))
                : h('div', { class: 'client placeholder' }, t('还没有 agent 用过 ssh-manager。接入后会显示在这里。')))),

        h('section', { class: 'card setup', 'aria-label': t('接入方式') },
          data && !command && h('div', { class: 'banner' }, icon('alert', 16),
            h('div', { class: 'grow' }, t('没有找到已安装的 ssh-manager 命令行工具。agent 通过它连接，请先安装。'))),
          h('div', { class: 'section' },
            h('h3', { class: 'section-title' }, 'Claude Code'),
            copyBox(`claude mcp add ssh-manager -- ${binary} mcp`)),
          h('div', { class: 'section' },
            h('h3', { class: 'section-title' }, t('其他 MCP 客户端')),
            copyBox(json)),
          h('div', { class: 'section' },
            h('h3', { class: 'section-title' }, t('agent 可以做的事')),
            h('ul', { class: 'can-list' },
              ['查看主机、项目和记录', '检查连接、执行命令', '上传和下载文件', '写下自己的发现，不会覆盖你的说明']
                .map((text) => h('li', null, h('span', { class: 'state ok' }, icon('check', 14, 2)), t(text))),
              ['不能查看密码或私钥', '不能保存凭据、修改服务器信任']
                .map((text) => h('li', null, h('span', { class: 'state err' }, icon('x', 14, 2)), t(text))))),
          h('p', { class: 'small muted' }, t('执行命令拥有对应 SSH 账号的全部权限。需要只读时，请在服务器上使用受限账号。')))))),
  });
}

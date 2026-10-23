import { h, icon, replace, screen } from './dom.js';
import {
  SOURCES, address, auth, caller, defaultConnection, keyAlgorithm, kind, relative, shortFingerprint, source, status,
} from './format.js';

// The short state shown in the list; the first problem wins because it blocks connecting.
export function hostState(view) {
  const c = defaultConnection(view);
  if (c.issues.length) return { tone: 'warn', label: '配置需补全', attention: true };
  if (view.trusted_keys.length === 0) return { tone: 'warn', label: '需确认身份', attention: true };
  if (c.authentication.kind === 'password' && !c.authentication.credential_ref) return { tone: 'warn', label: '需保存密码', attention: true };
  const last = view.last_check;
  if (!last) return { tone: '', label: '未检查' };
  if (last.status === 'succeeded') return { tone: 'ok', label: '可连接' };
  const s = status(last);
  return { tone: s.tone, label: s.label };
}

function stateBadge(view) {
  const s = hostState(view);
  return h('span', { class: `state ${s.tone}` }, h('span', { class: `dot ${s.tone || 'hollow'}` }), s.label);
}

function jumpNames(view, hosts) {
  const byConnection = new Map(hosts.flatMap((v) => v.connections.map((c) => [c.id, v.name])));
  return defaultConnection(view).jumps.map((id) => byConnection.get(id) ?? '未知跳板');
}

function matches(view, filter) {
  if (filter.source && source(view) !== filter.source) return false;
  if (filter.tag && !view.tags.includes(filter.tag)) return false;
  const q = filter.query.trim().toLowerCase();
  if (!q) return true;
  const c = defaultConnection(view);
  return [view.name, c.address, c.user, view.context.description, ...view.tags, ...view.context.projects.map((p) => `${p.name} ${p.path}`)]
    .some((text) => text.toLowerCase().includes(q));
}

function row(view, hosts, selected, actions) {
  const c = defaultConnection(view);
  const jumps = jumpNames(view, hosts);
  const summary = view.context.description.split('\n')[0];
  return h('button', {
    type: 'button', class: 'host-row', 'aria-pressed': String(selected), onclick: () => actions.select(view.id),
  },
  h('span', { class: 'row' }, h('span', { class: 'host-name grow ellipsis' }, view.name), stateBadge(view)),
  summary && h('span', { class: 'host-sub ellipsis' }, summary),
  h('span', { class: 'host-addr ellipsis' }, address(c), jumps.length ? ` · 经 ${jumps.join(' → ')}` : ''));
}

function info(iconName, title, value, detail, action, tone = '') {
  return h('div', { class: 'info' },
    h('div', { class: 'info-head' }, icon(iconName, 14), title),
    h('div', { class: `info-value ${tone}` }, value),
    detail && h('div', { class: 'info-detail' }, detail),
    action);
}

const link = (label, onclick) => h('button', { type: 'button', class: 'btn-link', onclick }, label);
const home = (path) => path.replace(/^\/Users\/[^/]+|^\/home\/[^/]+/, '~');

function loginInfo(view, actions) {
  const c = defaultConnection(view);
  const a = c.authentication;
  if (a.kind === 'password') {
    return info('key', '登录方式', a.credential_ref ? '密码 · 已保存' : '密码 · 尚未保存', '保存在系统钥匙串中',
      link(a.credential_ref ? '更换密码' : '保存密码', () => actions.credential(view, 'login_password')),
      a.credential_ref ? '' : 'warn');
  }
  if (a.kind === 'key') {
    return info('key', '登录方式', a.passphrase_ref ? '密钥 · 口令已保存' : '密钥',
      [a.paths.map((p) => h('div', { class: 'mono ellipsis' }, home(p))),
        c.identity_agent && h('div', { class: 'ellipsis' }, `代理 ${home(c.identity_agent)}`)],
      link(a.passphrase_ref ? '更换口令' : '保存口令', () => actions.credential(view, 'key_passphrase')));
  }
  return info('key', '登录方式', 'ssh-agent', c.identity_agent ? h('div', { class: 'ellipsis' }, home(c.identity_agent)) : '本机默认的 ssh-agent');
}

function trustInfo(view, actions) {
  const key = view.trusted_keys[0];
  if (!key) return info('shield', '服务器身份', '尚未确认', '确认后才能连接', link('确认身份', () => actions.trust(view)), 'warn');
  return info('shieldCheck', '服务器身份', '已信任',
    h('div', { class: 'mono ellipsis', title: key.fingerprint }, `${keyAlgorithm(key.algorithm)} ${shortFingerprint(key.fingerprint)}`),
    link('重新确认', () => actions.trust(view)), 'ok');
}

function detail(view, state, actions) {
  const c = defaultConnection(view);
  const jumps = jumpNames(view, state.overview.hosts);
  const checking = state.checking.has(view.id);
  const result = state.checkResults[view.id];
  const facts = view.context.probed_facts;
  const last = view.last_operation;
  const notes = [...view.context.agent_notes].reverse();

  return h('div', { class: 'detail' },
    h('div', { class: 'detail-head' },
      h('div', { class: 'grow stack', style: 'gap: 4px' },
        h('h2', { class: 'detail-title selectable' }, view.name),
        view.context.description
          ? h('p', { class: 'note-text', style: 'color: var(--text2)' }, view.context.description)
          : link('添加说明', () => actions.edit(view)),
        h('div', { class: 'row wrap', style: 'gap: 4px; margin-top: 2px' },
          h('span', { class: 'chip' }, SOURCES[source(view)]),
          view.tags.map((t) => h('span', { class: 'chip' }, t)))),
      h('button', { type: 'button', class: 'btn', onclick: () => actions.edit(view) }, '编辑'),
      h('button', { type: 'button', class: 'btn btn-primary', disabled: checking, onclick: () => actions.check(view) },
        icon('refresh', 14, 2), checking ? '检查中…' : '检查连接')),

    c.issues.length > 0 && h('div', { class: 'banner' },
      icon('alert', 16),
      h('div', { class: 'grow' },
        h('strong', null, '导入时有配置未能转换，处理前无法连接'),
        h('ul', null, c.issues.map((issue) => h('li', { class: 'small' }, issue)))),
      h('button', { type: 'button', class: 'btn btn-sm', onclick: () => actions.clearIssues(view) }, '我已处理')),

    result && result.status !== 'succeeded' && h('div', { class: `banner${status(result).tone === 'err' ? ' err' : ''}` },
      icon(status(result).icon, 16),
      h('div', { class: 'grow stack' },
        h('strong', null, status(result).label),
        result.next_step && h('span', { class: 'small' }, result.next_step),
        result.stderr && h('span', { class: 'mono small break' }, result.stderr.trim().split('\n').slice(-3).join('\n'))),
      result.status === 'host_key_required' && h('button', { type: 'button', class: 'btn btn-sm', onclick: () => actions.trust(view) }, '确认身份')),

    h('div', { class: 'section' },
      h('h3', { class: 'section-title' }, '连接路径'),
      h('div', { class: 'chain' },
        h('span', { class: 'hop' }, '本机'),
        jumps.map((name) => [icon('arrow', 14), h('span', { class: 'hop mono' }, name)]),
        icon('arrow', 14),
        h('span', { class: 'hop target mono' }, address(c)))),

    h('div', { class: 'info-grid' },
      loginInfo(view, actions),
      trustInfo(view, actions),
      info('system', '系统信息',
        facts ? `${facts.system} · ${facts.architecture}` : '暂无',
        facts ? `用户 ${facts.user} · 探测于${relative(facts.observed_at)}` : '检查连接后显示'),
      info('activity', '最近操作',
        last ? `${caller(last.caller)} · ${kind(last.kind)}` : '暂无',
        last ? `${relative(last.started_at)} · ${status(last).label}` : null)),

    h('div', { class: 'section' },
      h('div', { class: 'section-head' },
        h('h3', { class: 'section-title grow' }, '项目'),
        link('添加项目', () => actions.project(view))),
      view.context.projects.length
        ? h('div', { class: 'list-box' }, view.context.projects.map((p) => h('div', { class: 'list-item' },
          icon('folder', 14),
          h('span', { class: 'grow stack' },
            h('span', { class: 'row', style: 'gap: 8px' }, h('strong', null, p.name), h('span', { class: 'mono ellipsis', style: 'color: var(--text2)' }, p.path)),
            p.description && h('span', { class: 'small', style: 'color: var(--text2)' }, p.description)),
          h('button', { type: 'button', class: 'icon-btn', 'aria-label': `删除项目 ${p.name}`, title: '删除项目', onclick: () => actions.removeProject(view, p.name) }, icon('x', 13)))))
        : h('p', { class: 'small muted' }, '记下项目路径后，agent 可以直接在对应目录工作。')),

    h('div', { class: 'section' },
      h('h3', { class: 'section-title' }, 'Agent 记录'),
      notes.length
        ? notes.map((n) => h('div', { class: 'note' },
          h('span', { class: 'note-who' }, 'Agent'),
          h('div', { class: 'stack', style: 'gap: 2px; min-width: 0' },
            h('div', { class: 'note-text' }, n.text),
            h('div', { class: 'small muted' }, `${relative(n.recorded_at)} · 仅供参考，可能已变化`))))
        : h('p', { class: 'small muted' }, 'agent 在这台主机上的发现会显示在这里。')),

    h('div', { class: 'row', style: 'justify-content: flex-end; padding-top: 4px' },
      h('button', { type: 'button', class: 'btn btn-sm btn-danger', onclick: () => actions.remove(view) }, '删除主机')));
}

export function hostsView(state, actions) {
  const hosts = state.overview.hosts;
  const list = h('nav', { class: 'host-list', 'aria-label': '主机列表' });
  const attention = hosts.filter((v) => hostState(v).attention).length;

  function fill() {
    const shown = hosts.filter((v) => matches(v, state.filter));
    replace(list, shown.length
      ? shown.map((v) => row(v, hosts, v.id === state.selected, actions))
      : h('p', { class: 'list-empty' }, '没有符合条件的主机。'));
  }
  fill();
  const selected = hosts.find((v) => v.id === state.selected);

  return screen({
    title: '主机',
    sub: `${hosts.length} 台${attention ? ` · ${attention} 台需要处理` : ''}`,
    tools: [
      h('label', { class: 'search' }, icon('search', 14), h('input', {
        type: 'search', placeholder: '搜索', value: state.filter.query, 'aria-label': '搜索主机、地址、项目',
        oninput: (e) => { state.filter.query = e.target.value; fill(); },
      })),
      h('button', { type: 'button', class: 'btn btn-primary', onclick: () => actions.add() }, icon('plus', 14, 2), '添加'),
    ],
    body: h('div', { class: 'split' },
      list,
      h('section', { class: 'host-detail', 'aria-label': '主机详情' }, selected && detail(selected, state, actions))),
  });
}

export function emptyHostsView(actions) {
  return screen({
    title: '主机',
    body: h('div', { class: 'main-body' }, h('div', { class: 'empty', style: 'margin-top: 48px' },
      h('div', { class: 'empty-icon' }, icon('server', 20)),
      h('div', { class: 'empty-title' }, '还没有主机'),
      h('p', null, '从已有的 SSH 配置导入，或者手动添加一台。'),
      h('div', { class: 'row' },
        h('button', { type: 'button', class: 'btn', onclick: () => actions.add() }, '手动添加'),
        h('button', { type: 'button', class: 'btn btn-primary', onclick: () => actions.navigate('import') }, icon('import', 14), '从 SSH 配置导入')))),
  });
}

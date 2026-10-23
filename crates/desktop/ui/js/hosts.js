import { h, icon, replace } from './dom.js';
import {
  SOURCES, address, auth, caller, defaultConnection, keyAlgorithm, kind, relative, shortFingerprint, source, status,
} from './format.js';

// The one-line state shown in the list; the first problem wins because it blocks connecting.
export function hostState(view) {
  const c = defaultConnection(view);
  if (c.issues.length) return { tone: 'warn', icon: 'alert', label: '配置需补全', pill: true };
  if (view.trusted_keys.length === 0) return { tone: 'warn', icon: 'shield', label: '需确认身份', pill: true };
  if (c.authentication.kind === 'password' && !c.authentication.credential_ref) return { tone: 'warn', icon: 'key', label: '需保存密码', pill: true };
  const last = view.last_check;
  if (!last) return { tone: '', label: '尚未检查' };
  if (last.status === 'succeeded') return { tone: 'ok', icon: 'check', label: `可连接 · ${relative(last.started_at)}` };
  const s = status(last);
  return { tone: s.tone, icon: s.icon, label: `${s.label} · ${relative(last.started_at)}` };
}

function stateBadge(view) {
  const s = hostState(view);
  const content = [s.icon && icon(s.icon, 13, 2), s.label];
  return s.pill ? h('span', { class: `pill ${s.tone}` }, content) : h('span', { class: `status-text ${s.tone}` }, content);
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
  h('div', { class: 'row' },
    h('span', { class: 'host-name' }, view.name),
    h('span', { class: 'small muted ellipsis grow' }, summary),
    stateBadge(view)),
  h('div', { class: 'mono ellipsis', style: 'color: var(--body2)' }, address(c), jumps.length ? h('span', { class: 'muted' }, ` · 经 ${jumps.join(' → ')}`) : null),
  h('div', { class: 'row wrap', style: 'gap: 6px' },
    h('span', { class: 'chip' }, SOURCES[source(view)]),
    h('span', { class: 'chip' }, auth(c)),
    view.tags.map((t) => h('span', { class: 'chip' }, t))));
}

function infoCard(iconName, title, value, detail, action, tone = '') {
  return h('div', { class: 'info' },
    h('div', { class: 'info-head' }, icon(iconName, 15), title),
    h('div', { class: `info-value ${tone}` }, value),
    detail && h('div', { class: 'small', style: 'color: var(--text2)' }, detail),
    action);
}

function loginCard(view, actions) {
  const c = defaultConnection(view);
  const a = c.authentication;
  if (a.kind === 'password') {
    return infoCard('key', '登录方式', a.credential_ref ? '密码 · 已保存' : '密码 · 尚未保存', '保存在系统钥匙串中',
      h('button', { type: 'button', class: 'btn-link small', style: 'align-self: flex-start; padding-left: 0', onclick: () => actions.credential(view, 'login_password') }, a.credential_ref ? '更换密码' : '保存密码'),
      a.credential_ref ? '' : 'warn');
  }
  if (a.kind === 'key') {
    return infoCard('key', '登录方式', a.passphrase_ref ? '密钥 · 口令已保存' : '密钥',
      h('div', { class: 'stack' }, a.paths.map((p) => h('span', { class: 'mono small break' }, p.replace(/^\/Users\/[^/]+|^\/home\/[^/]+/, '~'))),
        c.identity_agent && h('span', { class: 'small break' }, `代理：${c.identity_agent}`)),
      h('button', { type: 'button', class: 'btn-link small', style: 'align-self: flex-start; padding-left: 0', onclick: () => actions.credential(view, 'key_passphrase') }, a.passphrase_ref ? '更换口令' : '保存口令'));
  }
  return infoCard('key', '登录方式', 'ssh-agent', c.identity_agent ? h('span', { class: 'break' }, c.identity_agent) : '使用本机默认的 ssh-agent');
}

function trustCard(view, actions) {
  const key = view.trusted_keys[0];
  const button = (label) => h('button', { type: 'button', class: 'btn-link small', style: 'align-self: flex-start; padding-left: 0', onclick: () => actions.trust(view) }, label);
  if (!key) return infoCard('shield', '服务器身份', '尚未确认', '确认后才能连接', button('确认身份'), 'warn');
  return infoCard('shieldCheck', '服务器身份', '已信任',
    h('span', { class: 'mono' }, `${keyAlgorithm(key.algorithm)} ${shortFingerprint(key.fingerprint)}`), button('重新确认'), 'ok');
}

function detail(view, state, actions) {
  const c = defaultConnection(view);
  const jumps = jumpNames(view, state.overview.hosts);
  const checking = state.checking.has(view.id);
  const result = state.checkResults[view.id];
  const facts = view.context.probed_facts;
  const last = view.last_operation;
  const notes = [...view.context.agent_notes].reverse();

  return h('div', { class: 'card detail-card' },
    h('div', { class: 'row wrap', style: 'align-items: flex-start; gap: 16px' },
      h('div', { class: 'stack grow', style: 'gap: 4px; flex: 1 1 240px' },
        h('h2', { class: 'detail-title' }, view.name),
        view.context.description
          ? h('p', { class: 'note-text', style: 'color: var(--text2)' }, view.context.description)
          : h('button', { type: 'button', class: 'btn-link', style: 'align-self: flex-start; padding-left: 0', onclick: () => actions.edit(view) }, '添加说明')),
      h('div', { class: 'row' },
        h('button', { type: 'button', class: 'btn', onclick: () => actions.edit(view) }, '编辑'),
        h('button', { type: 'button', class: 'btn btn-primary', disabled: checking, onclick: () => actions.check(view) },
          icon('refresh', 16, 2), checking ? '检查中…' : '检查连接'))),

    c.issues.length > 0 && h('div', { class: 'banner' },
      icon('alert', 18),
      h('div', { class: 'grow' },
        h('strong', null, '导入时有配置未能转换，处理前无法连接'),
        h('ul', null, c.issues.map((issue) => h('li', { class: 'small' }, issue)))),
      h('button', { type: 'button', class: 'btn btn-sm', onclick: () => actions.clearIssues(view) }, '我已处理')),

    result && result.status !== 'succeeded' && h('div', { class: `banner ${status(result).tone === 'err' ? 'err' : ''}` },
      icon(status(result).icon, 18),
      h('div', { class: 'grow stack' },
        h('strong', null, status(result).label),
        result.next_step && h('span', { class: 'small' }, result.next_step),
        result.stderr && h('span', { class: 'mono small break' }, result.stderr.trim().split('\n').slice(-3).join('\n'))),
      result.status === 'host_key_required' && h('button', { type: 'button', class: 'btn btn-sm', onclick: () => actions.trust(view) }, '确认身份')),

    h('div', { class: 'section' },
      h('h3', { class: 'section-title' }, '连接路径'),
      h('div', { class: 'chain' },
        h('span', { class: 'hop' }, '本机'),
        jumps.map((name) => [icon('arrow', 16), h('span', { class: 'hop mono' }, name)]),
        icon('arrow', 16),
        h('span', { class: 'hop target mono' }, address(c)))),

    h('div', { class: 'info-grid' },
      loginCard(view, actions),
      trustCard(view, actions),
      infoCard('system', '系统信息',
        facts ? `${facts.system} · ${facts.architecture}` : '暂无',
        facts ? `用户 ${facts.user} · 探测于 ${relative(facts.observed_at)}` : '检查连接后显示'),
      infoCard('activity', '最近操作',
        last ? `${caller(last.caller)} ${kind(last.kind)}` : '暂无',
        last ? `${relative(last.started_at)} · ${status(last).label}` : null)),

    h('div', { class: 'section' },
      h('div', { class: 'row' },
        h('h3', { class: 'section-title grow' }, '项目'),
        h('button', { type: 'button', class: 'btn-link small', onclick: () => actions.project(view) }, '添加项目')),
      view.context.projects.length
        ? h('div', { class: 'list-box' }, view.context.projects.map((p) => h('div', { class: 'list-item' },
          icon('folder', 16),
          h('span', { style: 'font-weight: 600; min-width: 64px' }, p.name),
          h('span', { class: 'mono' }, p.path),
          h('span', { class: 'grow small', style: 'text-align: right; color: var(--text2)' }, p.description),
          h('button', { type: 'button', class: 'icon-btn', 'aria-label': `删除项目 ${p.name}`, onclick: () => actions.removeProject(view, p.name) }, icon('x', 14)))))
        : h('p', { class: 'small muted' }, '记下项目路径后，agent 可以直接在对应目录工作。')),

    h('div', { class: 'section' },
      h('h3', { class: 'section-title' }, 'Agent 记录'),
      notes.length
        ? notes.map((n) => h('div', { class: 'note agent' },
          h('span', { class: 'note-who' }, 'Agent'),
          h('div', { class: 'stack', style: 'gap: 2px; min-width: 0' },
            h('div', { class: 'note-text' }, n.text),
            h('div', { class: 'small muted' }, `${relative(n.recorded_at)} · 仅供参考，可能已变化`))))
        : h('p', { class: 'small muted' }, 'agent 在这台主机上的发现会显示在这里。')),

    h('div', { class: 'row', style: 'justify-content: flex-end' },
      h('button', { type: 'button', class: 'btn btn-sm btn-danger', onclick: () => actions.remove(view) }, icon('trash', 14), '删除主机')));
}

export function hostsView(state, actions) {
  const hosts = state.overview.hosts;
  const list = h('section', { class: 'host-list', 'aria-label': '主机列表' });
  const detailBox = h('section', { class: 'host-detail', 'aria-label': '主机详情' });

  function fill() {
    const shown = hosts.filter((v) => matches(v, state.filter));
    const attention = hosts.filter((v) => hostState(v).pill).length;
    replace(list,
      h('div', { class: 'host-list-head' },
        h('h1', { style: 'font-size: 20px; font-weight: 700' }, '主机'),
        h('span', { class: 'small muted' }, `${hosts.length} 台${attention ? ` · ${attention} 台需要处理` : ''}`)),
      shown.length
        ? shown.map((v) => row(v, hosts, v.id === state.selected, actions))
        : h('p', { class: 'muted', style: 'padding: 12px' }, '没有符合条件的主机。'));
  }
  fill();
  const selected = hosts.find((v) => v.id === state.selected);
  if (selected) detailBox.append(detail(selected, state, actions));

  const search = h('input', {
    type: 'search', placeholder: '搜索主机、地址、项目', value: state.filter.query, 'aria-label': '搜索主机',
    oninput: (e) => { state.filter.query = e.target.value; fill(); },
  });

  return h('div', { class: 'main' },
    h('header', { class: 'topbar' },
      h('label', { class: 'search' }, icon('search', 16), search),
      h('button', { type: 'button', class: 'btn', onclick: () => actions.navigate('import') }, icon('import', 16), '导入'),
      h('button', { type: 'button', class: 'btn btn-primary', onclick: () => actions.add() }, icon('plus', 16, 2), '添加主机')),
    h('div', { class: 'split' }, list, detailBox));
}

export function emptyHostsView(actions) {
  return h('div', { class: 'main' },
    h('div', { class: 'empty', style: 'margin: auto' },
      h('div', { class: 'dialog-icon' }, icon('server', 20)),
      h('div', { class: 'empty-title' }, '还没有主机'),
      h('p', null, '从已有的 SSH 配置导入，或者手动添加一台。'),
      h('div', { class: 'row' },
        h('button', { type: 'button', class: 'btn', onclick: () => actions.add() }, '手动添加'),
        h('button', { type: 'button', class: 'btn btn-primary', onclick: () => actions.navigate('import') }, icon('import', 16), '从 SSH 配置导入'))));
}

import { h, icon, replace, screen, showMenu } from './dom.js';
import {
  address, agentName, caller, defaultConnection, keyAlgorithm, kind, relative, shortFingerprint, status,
} from './format.js';
import { t } from './i18n.js';

// The short state shown in the list. Problems that block connecting come first, then whether the server
// answers right now (from the background probe), then how the last real login went.
export function hostState(view, reach) {
  const c = defaultConnection(view);
  if (c.issues.length) return { tone: 'warn', label: t('配置需补全'), attention: true };
  if (view.last_operation?.status === 'host_key_changed') return { tone: 'err', label: t('服务器身份已变化'), attention: true };
  if (c.authentication.kind === 'password' && !c.authentication.credential_ref) return { tone: 'warn', label: t('需保存密码'), attention: true };
  if (reach && !reach.reachable) return { tone: 'err', label: t(reach.via_jump ? '跳板无法连接' : '无法连接') };
  const last = view.last_check;
  if (last && last.status !== 'succeeded' && last.status !== 'connection_failed') {
    const s = status(last);
    return { tone: s.tone, label: s.label };
  }
  if (reach && !reach.via_jump) return { tone: 'ok', label: t('在线') };
  if (last?.status === 'succeeded') return { tone: 'ok', label: t('可连接') };
  return { tone: '', label: t(reach ? '未检查' : '检测中…') };
}

function stateBadge(view, reach) {
  const s = hostState(view, reach);
  return h('span', { class: `state ${s.tone}` }, h('span', { class: `dot ${s.tone || 'hollow'}` }), s.label);
}

function jumpNames(view, hosts) {
  const byConnection = new Map(hosts.flatMap((v) => v.connections.map((c) => [c.id, v.name])));
  return defaultConnection(view).jumps.map((id) => byConnection.get(id) ?? t('未知跳板'));
}

function matches(view, filter) {
  if (filter.tag && !view.tags.includes(filter.tag)) return false;
  const q = filter.query.trim().toLowerCase();
  if (!q) return true;
  const c = defaultConnection(view);
  return [view.name, c.address, c.user, view.context.description, ...view.tags].some((text) => text.toLowerCase().includes(q));
}

// The same actions from the ⋯ button and from a right click on the host.
function hostMenu(view, state, actions, { withConnect }) {
  return [
    withConnect && { label: t('连接'), run: () => actions.terminal(view) },
    withConnect && { label: t('检查'), run: () => actions.check(view) },
    withConnect && '-',
    { label: t('编辑…'), run: () => actions.edit(view) },
    { label: t('复制连接命令'), run: () => actions.copyCommand(view) },
    '-',
    { label: t('删除主机…'), danger: true, run: () => actions.remove(view) },
  ];
}

function row(view, state, actions) {
  const hosts = state.overview.hosts;
  const c = defaultConnection(view);
  const jumps = jumpNames(view, hosts);
  const summary = view.context.description.split('\n')[0];
  return h('button', {
    type: 'button', class: 'host-row', 'aria-pressed': String(view.id === state.selected),
    onclick: () => actions.select(view.id),
    oncontextmenu: (e) => {
      e.preventDefault();
      actions.select(view.id);
      showMenu(hostMenu(view, state, actions, { withConnect: true }), [e.clientX, e.clientY]);
    },
  },
  h('span', { class: 'row' }, h('span', { class: 'host-name grow ellipsis' }, view.name), stateBadge(view, state.reach[view.id])),
  summary && h('span', { class: 'host-sub ellipsis' }, summary),
  h('span', { class: 'host-addr ellipsis' }, address(c), jumps.length ? t(' · 经 {hops}', { hops: jumps.join(' → ') }) : ''));
}

function info(iconName, title, value, detail, action, tone = '') {
  return h('div', { class: 'info' },
    h('div', { class: 'info-head' }, icon(iconName, 14), title),
    h('div', { class: `info-value ${tone}` }, value),
    detail && h('div', { class: 'info-detail' }, detail),
    action);
}

// "刚刚探测" reads naturally; other times read "探测于 3 分钟前".
function probedAt(iso) {
  const when = relative(iso);
  return when === t('刚刚') ? t('刚刚探测') : t('探测于 {when}', { when });
}
const link = (label, onclick) => h('button', { type: 'button', class: 'btn-link', onclick }, label);
const home = (path) => path.replace(/^\/Users\/[^/]+|^\/home\/[^/]+/, '~');

function loginInfo(view, actions) {
  const c = defaultConnection(view);
  const a = c.authentication;
  if (a.kind === 'password') {
    return info('key', t('登录方式'), t(a.credential_ref ? '密码 · 已保存' : '密码 · 尚未保存'), t('保存在系统钥匙串中'),
      link(t(a.credential_ref ? '更换密码' : '保存密码'), () => actions.credential(view, 'login_password')),
      a.credential_ref ? '' : 'warn');
  }
  if (a.kind === 'agent' || c.identity_agent) {
    return info('key', t('登录方式'), t('密钥 · {agent}', { agent: agentName(c.identity_agent) }),
      t('使用 {agent} 中的密钥', { agent: agentName(c.identity_agent) }));
  }
  // Only a key with a passphrase needs one saved; a plain key just logs in.
  return info('key', t('登录方式'), t(a.passphrase_ref ? '密钥 · 已保存密钥密码' : '密钥'),
    [a.paths.map((p) => h('div', { class: 'mono ellipsis' }, home(p))),
      view.key_passphrase && !a.passphrase_ref && h('div', null, t('这个密钥设有密码，保存后连接时不用再输入'))],
    (view.key_passphrase || a.passphrase_ref) && link(t(a.passphrase_ref ? '更换密钥密码' : '保存密钥密码'), () => actions.credential(view, 'key_passphrase')));
}

// The server's key is recorded on the first connection; a later change stops connecting until the person accepts it.
function trustInfo(view, actions) {
  const key = view.trusted_keys[0];
  if (view.last_operation?.status === 'host_key_changed') {
    return info('shield', t('服务器身份'), t('已变化'), t('连接已停止'), link(t('信任新身份'), () => actions.trust(view)), 'err');
  }
  if (!key) return info('shield', t('服务器身份'), t('未记录'), t('首次连接时自动记录'));
  return info('shieldCheck', t('服务器身份'), t('已记录'),
    h('div', { class: 'mono ellipsis', title: key.fingerprint }, `${keyAlgorithm(key.algorithm)} ${shortFingerprint(key.fingerprint)}`));
}

function reachLine(reach) {
  if (!reach) return h('span', { class: 'small muted' }, t('检测中…'));
  const when = relative(reach.at);
  const text = reach.reachable
    ? t(reach.via_jump ? '跳板在线 · {when}' : '在线 · {when}', { when })
    : t(reach.via_jump ? '跳板无法连接 · {when}' : '无法连接 · {when}', { when });
  return h('span', { class: `state small ${reach.reachable ? 'ok' : 'err'}` }, h('span', { class: `dot ${reach.reachable ? 'ok' : 'err'}` }), text);
}

function detail(view, state, actions) {
  const c = defaultConnection(view);
  const jumps = jumpNames(view, state.overview.hosts);
  const checking = state.checking.has(view.id);
  const opening = state.opening.has(view.id);
  const result = state.checkResults[view.id];
  const facts = view.context.probed_facts;
  const last = view.last_operation;
  const more = h('button', {
    type: 'button', class: 'btn icon-only', 'aria-label': t('更多操作'), title: t('更多操作'),
    onclick: (e) => showMenu(hostMenu(view, state, actions, { withConnect: false }), e.currentTarget),
  }, icon('more', 16, 2.4));

  return h('div', { class: 'detail' },
    h('div', { class: 'detail-head' },
      h('div', { class: 'grow stack', style: 'gap: 4px' },
        h('h2', { class: 'detail-title selectable' }, view.name),
        view.context.description
          ? h('p', { class: 'note-text', style: 'color: var(--text2)' }, view.context.description)
          : link(t('添加说明'), () => actions.edit(view)),
        view.tags.length > 0 && h('div', { class: 'row wrap', style: 'gap: 4px; margin-top: 2px' },
          view.tags.map((tag) => h('span', { class: 'chip' }, tag)))),
      h('button', { type: 'button', class: 'btn', disabled: checking, onclick: () => actions.check(view) },
        icon('refresh', 14, 2), checking ? t('检查中…') : t('检查')),
      h('button', { type: 'button', class: 'btn btn-primary', disabled: opening, title: t('在终端中连接'), onclick: () => actions.terminal(view) },
        icon('terminal', 14, 2), opening ? t('打开中…') : t('连接')),
      more),

    c.issues.length > 0 && h('div', { class: 'banner' },
      icon('alert', 16),
      h('div', { class: 'grow' },
        h('strong', null, t('导入时有配置未能转换，处理前无法连接')),
        h('ul', null, c.issues.map((issue) => h('li', { class: 'small' }, issue)))),
      h('button', { type: 'button', class: 'btn btn-sm', onclick: () => actions.clearIssues(view) }, t('我已处理'))),

    result && result.status !== 'succeeded' && h('div', { class: `banner${status(result).tone === 'err' ? ' err' : ''}` },
      icon(status(result).icon, 16),
      h('div', { class: 'grow stack' },
        h('strong', null, status(result).label),
        (result.error ?? status(result).hint) && h('span', { class: 'small' }, result.error ?? status(result).hint),
        result.stderr && h('span', { class: 'mono small break' }, result.stderr.trim().split('\n').slice(-3).join('\n'))),
      result.status === 'host_key_changed' && h('button', { type: 'button', class: 'btn btn-sm', onclick: () => actions.trust(view) }, t('信任新身份'))),

    h('div', { class: 'section' },
      h('div', { class: 'section-head' },
        h('h3', { class: 'section-title grow' }, t('连接路径')),
        reachLine(state.reach[view.id])),
      h('div', { class: 'chain' },
        h('span', { class: 'hop' }, t('本机')),
        jumps.map((name) => [icon('arrow', 14), h('span', { class: 'hop mono' }, name)]),
        icon('arrow', 14),
        h('span', { class: 'hop target mono' }, address(c)))),

    h('div', { class: 'info-grid' },
      loginInfo(view, actions),
      trustInfo(view, actions),
      info('system', t('系统信息'),
        facts ? `${facts.system} · ${facts.architecture}` : t('暂无'),
        facts ? t('用户 {user} · {when}', { user: facts.user, when: probedAt(facts.observed_at) }) : t('检查连接后显示')),
      info('activity', t('最近操作'),
        last ? `${caller(last.caller)} · ${kind(last.kind)}` : t('暂无'),
        last ? `${relative(last.started_at)} · ${status(last).label}` : null)));
}

export function hostsView(state, actions) {
  const hosts = state.overview.hosts;
  const list = h('nav', { class: 'host-list', 'aria-label': t('主机列表') });
  const attention = hosts.filter((v) => hostState(v, state.reach[v.id]).attention).length;

  function fill() {
    const shown = hosts.filter((v) => matches(v, state.filter));
    replace(list, shown.length
      ? shown.map((v) => row(v, state, actions))
      : h('p', { class: 'list-empty' }, t('没有符合条件的主机。')));
  }
  fill();
  const selected = hosts.find((v) => v.id === state.selected);

  return screen({
    title: t('主机'),
    sub: attention ? t('{n} 台 · {m} 台需要处理', { n: hosts.length, m: attention }) : t('{n} 台', { n: hosts.length }),
    tools: [
      h('label', { class: 'search' }, icon('search', 14), h('input', {
        type: 'search', placeholder: t('搜索'), value: state.filter.query, 'aria-label': t('搜索主机、地址、标签'),
        oninput: (e) => { state.filter.query = e.target.value; fill(); },
      })),
      h('button', { type: 'button', class: 'btn btn-primary', onclick: () => actions.add() }, icon('plus', 14, 2), t('添加')),
    ],
    body: h('div', { class: 'split' },
      list,
      h('section', { class: 'host-detail', 'aria-label': t('主机详情') }, selected && detail(selected, state, actions))),
  });
}

export function emptyHostsView(actions) {
  return screen({
    title: t('主机'),
    body: h('div', { class: 'main-body' }, h('div', { class: 'empty', style: 'margin-top: 48px' },
      h('div', { class: 'empty-icon' }, icon('server', 20)),
      h('div', { class: 'empty-title' }, t('还没有主机')),
      h('p', null, t('从已有的 SSH 配置导入，或者手动添加一台。')),
      h('div', { class: 'row' },
        h('button', { type: 'button', class: 'btn', onclick: () => actions.add() }, t('手动添加')),
        h('button', { type: 'button', class: 'btn btn-primary', onclick: () => actions.navigate('import') }, icon('import', 14), t('从 SSH 配置导入'))))),
  });
}

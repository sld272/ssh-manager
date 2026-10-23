import { activityNav, activityView } from './activity.js';
import { agentsView } from './agents.js';
import { api, message } from './api.js';
import {
  addHostDialog, confirmDialog, credentialDialog, editHostDialog, projectDialog, trustDialog,
} from './dialogs.js';
import { h, icon, replace, toast } from './dom.js';
import { SOURCES, source } from './format.js';
import { emptyHostsView, hostsView } from './hosts.js';
import { importView } from './import.js';

const state = {
  view: 'hosts',
  overview: null,
  loadError: null,
  selected: null,
  filter: { source: null, tag: null, query: '' },
  checking: new Set(),
  checkResults: {},
  running: new Map(),
  history: [],
  activity: { filter: 'all', caller: null },
  import: { path: '', preview: null, selected: new Set(), error: null, applying: false },
  agents: null,
};

async function refresh() {
  try {
    state.overview = await api.overview();
    state.loadError = null;
    if (!state.overview.hosts.some((v) => v.id === state.selected)) state.selected = state.overview.hosts[0]?.id ?? null;
  } catch (e) {
    state.loadError = message(e);
  }
  render();
}

async function loadHistory() {
  try {
    state.history = await api.history(300);
  } catch (e) {
    toast(message(e));
  }
}

const actions = {
  render,
  async navigate(view) {
    state.view = view;
    render();
    if (view === 'activity') { await loadHistory(); render(); }
    if (view === 'agents') { state.agents = await api.agents().catch(() => null); render(); }
    if (view === 'import' && !state.import.preview) actions.loadImport();
    if (view === 'hosts') refresh();
  },
  select(id) { state.selected = id; render(); },
  openHost(id) { state.selected = id; actions.navigate('hosts'); },
  async check(view) {
    state.checking.add(view.id);
    state.running.set(view.id, { kind: 'check', hostName: view.name, startedAt: new Date().toISOString() });
    delete state.checkResults[view.id];
    render();
    try {
      const result = await api.check(view.id);
      state.checkResults[view.id] = result;
      if (result.status === 'succeeded') toast(`${view.name} 可以连接`);
    } catch (e) {
      state.checkResults[view.id] = { status: 'needs_interaction', next_step: message(e), stderr: '' };
    } finally {
      state.checking.delete(view.id);
      state.running.delete(view.id);
    }
    await refresh();
  },
  trust: (view) => trustDialog(view, refresh),
  credential: (view, purpose) => credentialDialog(view, purpose, refresh),
  edit: (view) => editHostDialog(view, refresh),
  project: (view) => projectDialog(view, refresh),
  add: () => addHostDialog((host) => { state.selected = host.id; state.view = 'hosts'; refresh(); }),
  async clearIssues(view) {
    const ok = await confirmDialog({ title: '确认已处理这些配置？', body: '确认后可以连接。如果配置仍不完整，连接可能失败。', confirmLabel: '确认' });
    if (!ok) return;
    await api.hostUpdate(view.id, { clearIssues: true }).catch((e) => toast(message(e)));
    refresh();
  },
  async removeProject(view, name) {
    await api.projectRemove(view.id, name).catch((e) => toast(message(e)));
    refresh();
  },
  async remove(view) {
    const ok = await confirmDialog({
      title: `删除 ${view.name}？`,
      body: '会同时删除为它保存的密码和口令。操作记录会保留。',
      confirmLabel: '删除',
    });
    if (!ok) return;
    try {
      await api.hostRemove(view.id);
      toast(`已删除 ${view.name}`);
    } catch (e) {
      toast(message(e));
    }
    refresh();
  },
  async loadImport() {
    const imp = state.import;
    imp.error = null;
    imp.preview = null;
    render();
    try {
      imp.preview = await api.importPreview(imp.path || null);
      const existing = new Set(imp.preview.existing);
      imp.selected = new Set(imp.preview.plan.hosts.map((x) => x.name).filter((name) => !existing.has(name)));
    } catch (e) {
      imp.error = message(e);
    }
    render();
  },
  async applyImport() {
    const imp = state.import;
    imp.applying = true;
    render();
    try {
      const count = await api.importApply(imp.path || null, [...imp.selected]);
      toast(`已导入 ${count} 台主机`);
      imp.preview = null;
      state.view = 'hosts';
      await refresh();
    } catch (e) {
      imp.error = message(e);
    } finally {
      imp.applying = false;
      render();
    }
  },
};

function navItem(view, label, iconName, extra) {
  return h('button', {
    type: 'button', class: 'nav-item', 'aria-current': state.view === view ? 'page' : null,
    onclick: () => actions.navigate(view),
  }, icon(iconName, 16), h('span', { class: 'grow' }, label), extra);
}

function hostFilters() {
  const hosts = state.overview?.hosts ?? [];
  if (!hosts.length) return null;
  const counts = {};
  for (const v of hosts) counts[source(v)] = (counts[source(v)] ?? 0) + 1;
  const tags = [...new Set(hosts.flatMap((v) => v.tags))].sort();
  return [
    h('div', { class: 'nav-group' },
      h('div', { class: 'nav-label' }, '来源'),
      Object.entries(counts).map(([key, count]) => h('button', {
        type: 'button', class: 'nav-item sub', 'aria-pressed': String(state.filter.source === key),
        onclick: () => { state.filter.source = state.filter.source === key ? null : key; render(); },
      }, h('span', { class: 'grow' }, SOURCES[key]), h('span', { class: 'nav-count' }, count)))),
    tags.length > 0 && h('div', { class: 'stack', style: 'gap: 8px' },
      h('div', { class: 'nav-label', style: 'padding-bottom: 0' }, '标签'),
      h('div', { class: 'tag-list' }, tags.map((tag) => h('button', {
        type: 'button', class: 'tag', 'aria-pressed': String(state.filter.tag === tag),
        onclick: () => { state.filter.tag = state.filter.tag === tag ? null : tag; render(); },
      }, tag)))),
  ];
}

function nav() {
  const o = state.overview;
  const running = state.running.size;
  const ready = o && o.ssh_version && o.credential_helper_available;
  return h('nav', { class: 'nav', 'aria-label': '主导航' },
    h('div', { class: 'brand' }, h('span', { class: 'brand-mark' }, icon('terminal', 16, 2)), 'ssh-manager'),
    h('div', { class: 'nav-group' },
      navItem('hosts', '主机', 'server', o && h('span', { class: 'nav-count' }, o.hosts.length)),
      navItem('activity', '活动', 'activity', running > 0 && h('span', { class: 'nav-badge' }, `${running} 进行中`)),
      navItem('agents', 'Agent 接入', 'agent'),
      navItem('import', '导入', 'import')),
    state.view === 'hosts' && hostFilters(),
    state.view === 'activity' && activityNav(state, actions),
    o && h('div', { class: 'nav-foot' },
      h('span', { class: `dot ${ready ? 'ok' : 'warn'}` }),
      h('span', null, [
        o.credential_helper_available ? '凭据存储可用' : '未找到凭据组件',
        o.ssh_version ?? '未找到 OpenSSH',
      ].join(' · '))));
}

function content() {
  if (state.loadError) {
    return h('div', { class: 'main' }, h('div', { class: 'empty', style: 'margin: auto' },
      h('div', { class: 'empty-title' }, '无法读取主机库'),
      h('p', null, state.loadError),
      h('button', { type: 'button', class: 'btn', onclick: refresh }, '重试')));
  }
  if (!state.overview) return h('div', { class: 'main' });
  switch (state.view) {
    case 'activity': return activityView(state, actions);
    case 'agents': return agentsView(state, actions);
    case 'import': return importView(state, actions);
    default: return state.overview.hosts.length ? hostsView(state, actions) : emptyHostsView(actions);
  }
}

function render() {
  const app = document.getElementById('app');
  const scroll = document.scrollingElement.scrollTop;
  replace(app, h('div', { class: 'app' }, nav(), content()));
  document.scrollingElement.scrollTop = scroll;
}

// Agents and the CLI change the same catalog; pick up their work while the window is idle.
function idle() {
  return !document.querySelector('dialog[open]') && !(document.activeElement instanceof HTMLInputElement);
}
setInterval(async () => {
  if (!idle() || document.hidden) return;
  if (state.view === 'activity') { await loadHistory(); render(); }
  if (state.view === 'hosts') refresh();
}, 10000);
window.addEventListener('focus', () => { if (idle()) refresh(); });

refresh();

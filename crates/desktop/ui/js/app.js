import { activityView } from './activity.js';
import { agentsView } from './agents.js';
import { api, message } from './api.js';
import {
  addHostDialog, confirmDialog, credentialDialog, editHostDialog,
} from './dialogs.js';
import { h, icon, replace, toast } from './dom.js';
import { emptyHostsView, hostsView } from './hosts.js';
import { setLanguage, t } from './i18n.js';
import { importView } from './import.js';
import { environmentReady, settingsView } from './settings.js';

const state = {
  view: 'hosts',
  overview: null,
  settings: null,
  loadError: null,
  selected: null,
  filter: { tag: null, query: '' },
  checking: new Set(),
  opening: new Set(),
  checkResults: {},
  running: new Map(),
  history: [],
  activity: { filter: 'all', caller: null, host: null },
  import: { path: '', preview: null, selected: new Set(), error: null, applying: false },
  mcp: null,
  mcpBusy: new Set(),
  // Background reachability per host: { reachable, via_jump, at, failures }.
  reach: {},
};

// The last theme is cached so the window does not flash light before settings load.
function cachedTheme(theme) {
  try {
    if (theme) localStorage.setItem('theme', theme);
    return localStorage.getItem('theme') ?? 'system';
  } catch {
    return theme ?? 'system';
  }
}
function applyTheme(theme) {
  if (theme === 'system') delete document.documentElement.dataset.theme;
  else document.documentElement.dataset.theme = theme;
  cachedTheme(theme);
  api.setTheme(theme === 'system' ? null : theme).catch(() => {});
}
function applySettings(settings) {
  setLanguage(settings.language);
  applyTheme(settings.theme);
}

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

async function loadSettings() {
  try {
    state.settings = await api.settingsGet();
    applySettings(state.settings.settings);
  } catch (e) {
    toast(message(e));
  }
}

// Every host is probed when the window opens or comes back, then every 5 minutes while it is in front.
// A host that keeps failing is probed less often, up to every 40 minutes, until it answers again.
const PROBE_EVERY = 5 * 60 * 1000;
let probing = false;
async function probe(force = false) {
  const hosts = state.overview?.hosts ?? [];
  if (probing || document.hidden || !hosts.length) return;
  const now = Date.now();
  const due = hosts.filter((v) => {
    const r = state.reach[v.id];
    return force || !r || now - new Date(r.at).getTime() >= PROBE_EVERY * 2 ** Math.min(r.failures, 3);
  }).map((v) => v.id);
  if (!due.length) return;
  probing = true;
  try {
    const at = new Date().toISOString();
    for (const r of await api.probe(due)) {
      const before = state.reach[r.host_id];
      state.reach[r.host_id] = { ...r, at, failures: r.reachable ? 0 : (before?.failures ?? 0) + 1 };
    }
  } catch {
    // The probe only reads the catalog; a failure here leaves the last known states in place.
  } finally {
    probing = false;
  }
  if (idle()) render();
}
setInterval(() => probe(), 60 * 1000);
document.addEventListener('visibilitychange', () => probe(true));
window.addEventListener('online', () => probe(true));

async function loadMcp() {
  state.mcp = await api.mcpClients().catch((e) => ({ error: message(e) }));
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
    if (view === 'agents') loadMcp();
    if (view === 'settings') { await loadSettings(); render(); }
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
      if (result.status === 'succeeded') toast(t('{name} 可以连接', { name: view.name }));
    } catch (e) {
      state.checkResults[view.id] = { status: 'needs_interaction', error: message(e), stderr: '' };
    } finally {
      state.checking.delete(view.id);
      state.running.delete(view.id);
    }
    await refresh();
  },
  async terminal(view) {
    state.opening.add(view.id);
    render();
    try {
      await api.openTerminal(view.id);
    } catch (e) {
      toast(message(e));
    } finally {
      state.opening.delete(view.id);
      render();
    }
  },
  async trust(view) {
    const ok = await confirmDialog({
      title: t('信任 {name} 的新身份？', { name: view.name }),
      body: t('只在你知道服务器重装过系统、换过机器或地址时这样做。之后会记录它现在的密钥。'),
      confirmLabel: t('信任新身份'),
    });
    if (!ok) return;
    try {
      await api.trustReset(view.id);
    } catch (e) {
      toast(message(e));
      return;
    }
    await actions.check(view);
  },
  credential: (view, purpose) => credentialDialog(view, purpose, refresh),
  edit: (view) => editHostDialog(view, async () => { delete state.reach[view.id]; await refresh(); probe(); }),
  add: () => addHostDialog(async (host) => { state.selected = host.id; state.view = 'hosts'; await refresh(); probe(); }),
  copyCommand: (view) => navigator.clipboard.writeText(`ssh-manager connect ${view.name}`)
    .then(() => toast(t('已复制连接命令'))),
  async setMcp(client, connected) {
    state.mcpBusy.add(client.id);
    render();
    try {
      await api.mcpSet(client.id, connected);
      toast(t(connected ? '已接入 {name}' : '已从 {name} 移除', { name: client.name }));
    } catch (e) {
      toast(message(e));
    } finally {
      state.mcpBusy.delete(client.id);
    }
    loadMcp();
  },
  async clearIssues(view) {
    const ok = await confirmDialog({ title: t('确认已处理这些配置？'), body: t('确认后可以连接。如果配置仍不完整，连接可能失败。'), confirmLabel: t('确认') });
    if (!ok) return;
    await api.hostClearIssues(view.id).catch((e) => toast(message(e)));
    refresh();
  },
  async remove(view) {
    const ok = await confirmDialog({
      title: t('删除 {name}？', { name: view.name }),
      body: t('会同时删除为它保存的密码和密钥密码。操作记录会保留。'),
      confirmLabel: t('删除'),
    });
    if (!ok) return;
    try {
      await api.hostRemove(view.id);
      toast(t('已删除 {name}', { name: view.name }));
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
      toast(t('已导入 {n} 台主机', { n: count }));
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
  async saveSettings(next) {
    state.settings.settings = next;
    applySettings(next);
    render();
    try {
      await api.settingsSet(next);
    } catch (e) {
      toast(message(e));
      await loadSettings();
      render();
    }
  },
  revealDataDir: () => api.revealDataDir().catch((e) => toast(message(e))),
};

function navItem(view, label, iconName, extra) {
  return h('button', {
    type: 'button', class: 'nav-item', title: label, 'aria-current': state.view === view ? 'page' : null,
    onclick: () => actions.navigate(view),
  }, icon(iconName, 15), h('span', { class: 'grow label ellipsis' }, label), extra);
}

function hostFilters() {
  const tags = [...new Set((state.overview?.hosts ?? []).flatMap((v) => v.tags))].sort();
  if (!tags.length) return null;
  return h('div', { class: 'stack nav-filters', style: 'gap: 6px' },
    h('div', { class: 'nav-label', style: 'padding-bottom: 0' }, t('标签')),
    h('div', { class: 'tag-list' }, tags.map((tag) => h('button', {
      type: 'button', class: 'tag', 'aria-pressed': String(state.filter.tag === tag),
      onclick: () => { state.filter.tag = state.filter.tag === tag ? null : tag; render(); },
    }, tag))));
}

// In a narrow window the sidebar floats over the content: hovering the toolbar button opens it,
// leaving both closes it, and clicking the button toggles it for keyboard and trackpad users.
let closeTimer;
let openedAt = 0;
function setSidebar(open) {
  clearTimeout(closeTimer);
  const app = document.querySelector('.app');
  if (open && !app?.classList.contains('nav-open')) openedAt = Date.now();
  app?.classList.toggle('nav-open', open);
}
const closeSoon = () => { clearTimeout(closeTimer); closeTimer = setTimeout(() => setSidebar(false), 250); };
window.addEventListener('sidebar', (e) => {
  const open = !!document.querySelector('.app.nav-open');
  // Moving onto the button already opened it; the click that follows must not close it again.
  if (e.detail === 'toggle') setSidebar(!open || Date.now() - openedAt < 800);
  if (e.detail === 'hover') setSidebar(true);
  if (e.detail === 'leave') closeSoon();
});
document.addEventListener('keydown', (e) => { if (e.key === 'Escape') setSidebar(false); });
document.addEventListener('pointerdown', (e) => {
  if (document.querySelector('.app.nav-open') && !e.target.closest('.nav, .sidebar-toggle')) setSidebar(false);
});

function nav() {
  const o = state.overview;
  const running = state.running.size;
  const healthy = o ? !!o.ssh_version && o.credential_helper_available : true;
  return h('nav', {
    class: 'nav', 'aria-label': t('主导航'),
    onmouseenter: () => clearTimeout(closeTimer), onmouseleave: closeSoon,
  },
  // The window's traffic lights sit here; the strip also drags the window.
  h('div', { class: 'nav-top', 'data-tauri-drag-region': true }),
  h('div', { class: 'nav-group' },
    navItem('hosts', t('主机'), 'server', o && h('span', { class: 'nav-count' }, o.hosts.length)),
    navItem('activity', t('活动'), 'activity', running > 0 && h('span', { class: 'nav-badge' }, running)),
    navItem('agents', t('Agent 接入'), 'agent'),
    navItem('import', t('导入'), 'import')),
  state.view === 'hosts' && hostFilters(),
  h('div', { class: 'nav-group nav-bottom' },
    !(healthy && environmentReady(state.settings)) && h('button', {
      type: 'button', class: 'nav-item warn', title: t('环境需要处理'), onclick: () => actions.navigate('settings'),
    }, icon('alert', 15), h('span', { class: 'grow label ellipsis' }, t('环境需要处理'))),
    navItem('settings', t('设置'), 'settings')));
}

function content() {
  if (state.loadError) {
    return h('div', { class: 'main' }, h('div', { class: 'empty', style: 'margin: auto' },
      h('div', { class: 'empty-title' }, t('无法读取主机库')),
      h('p', null, state.loadError),
      h('button', { type: 'button', class: 'btn', onclick: refresh }, t('重试'))));
  }
  if (!state.overview) return h('div', { class: 'main' });
  switch (state.view) {
    case 'activity': return activityView(state, actions);
    case 'agents': return agentsView(state, actions);
    case 'import': return importView(state, actions);
    case 'settings': return state.settings ? settingsView(state, actions) : h('div', { class: 'main' });
    default: return state.overview.hosts.length ? hostsView(state, actions) : emptyHostsView(actions);
  }
}

// Each pane scrolls on its own; a refresh must not jump any of them back to the top.
const PANES = ['.nav', '.host-list', '.host-detail', '.main-body'];
function render() {
  const app = document.getElementById('app');
  const sameView = app.dataset.view === state.view;
  const scroll = PANES.map((selector) => app.querySelector(selector)?.scrollTop ?? 0);
  replace(app, h('div', { class: 'app' }, nav(), content()));
  app.dataset.view = state.view;
  PANES.forEach((selector, i) => {
    const pane = app.querySelector(selector);
    if (pane && (sameView || selector === '.nav')) pane.scrollTop = scroll[i];
  });
}

// Agents and the CLI change the same catalog; pick up their work while the window is idle.
function idle() {
  // A redraw replaces the focused field, which would also close an open dropdown and drop the choice.
  return !document.querySelector('dialog[open], .app.nav-open, .menu')
    && !(document.activeElement instanceof HTMLInputElement || document.activeElement instanceof HTMLSelectElement);
}
setInterval(async () => {
  if (!idle() || document.hidden) return;
  if (state.view === 'activity') { await loadHistory(); render(); }
  if (state.view === 'hosts') refresh();
}, 10000);
window.addEventListener('focus', () => { if (idle()) refresh(); probe(); });

applyTheme(cachedTheme());
await loadSettings();
await refresh();
probe(true);

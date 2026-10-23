import { activityNav, activityView } from './activity.js';
import { agentsView } from './agents.js';
import { api, message } from './api.js';
import {
  addHostDialog, confirmDialog, credentialDialog, editHostDialog, projectDialog, trustDialog,
} from './dialogs.js';
import { h, icon, replace, toast } from './dom.js';
import { source, sourceLabel } from './format.js';
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
  filter: { source: null, tag: null, query: '' },
  checking: new Set(),
  opening: new Set(),
  checkResults: {},
  running: new Map(),
  history: [],
  activity: { filter: 'all', caller: null },
  import: { path: '', preview: null, selected: new Set(), error: null, applying: false },
  agents: null,
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
  trust: (view) => trustDialog(view, refresh),
  credential: (view, purpose) => credentialDialog(view, purpose, refresh),
  edit: (view) => editHostDialog(view, refresh),
  project: (view) => projectDialog(view, refresh),
  add: () => addHostDialog((host) => { state.selected = host.id; state.view = 'hosts'; refresh(); }),
  async clearIssues(view) {
    const ok = await confirmDialog({ title: t('确认已处理这些配置？'), body: t('确认后可以连接。如果配置仍不完整，连接可能失败。'), confirmLabel: t('确认') });
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
      title: t('删除 {name}？', { name: view.name }),
      body: t('会同时删除为它保存的密码和口令。操作记录会保留。'),
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
  const hosts = state.overview?.hosts ?? [];
  if (!hosts.length) return null;
  const counts = {};
  for (const v of hosts) counts[source(v)] = (counts[source(v)] ?? 0) + 1;
  const tags = [...new Set(hosts.flatMap((v) => v.tags))].sort();
  return [
    h('div', { class: 'nav-group nav-filters' },
      h('div', { class: 'nav-label' }, t('来源')),
      Object.entries(counts).map(([key, count]) => h('button', {
        type: 'button', class: 'nav-item', 'aria-pressed': String(state.filter.source === key),
        onclick: () => { state.filter.source = state.filter.source === key ? null : key; render(); },
      }, h('span', { class: 'grow' }, sourceLabel(key)), h('span', { class: 'nav-count' }, count)))),
    tags.length > 0 && h('div', { class: 'stack nav-filters', style: 'gap: 6px' },
      h('div', { class: 'nav-label', style: 'padding-bottom: 0' }, t('标签')),
      h('div', { class: 'tag-list' }, tags.map((tag) => h('button', {
        type: 'button', class: 'tag', 'aria-pressed': String(state.filter.tag === tag),
        onclick: () => { state.filter.tag = state.filter.tag === tag ? null : tag; render(); },
      }, tag)))),
  ];
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
  state.view === 'activity' && activityNav(state, actions),
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
  return !document.querySelector('dialog[open], .app.nav-open') && !(document.activeElement instanceof HTMLInputElement);
}
setInterval(async () => {
  if (!idle() || document.hidden) return;
  if (state.view === 'activity') { await loadHistory(); render(); }
  if (state.view === 'hosts') refresh();
}, 10000);
window.addEventListener('focus', () => { if (idle()) refresh(); });

applyTheme(cachedTheme());
await loadSettings();
await refresh();

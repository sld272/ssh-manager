import { errorMessage } from './i18n.js';

// Every call returns the core library's JSON; failures reject with { code, message }.
const invoke = (command, args) => window.__TAURI__.core.invoke(command, args);

export const api = {
  overview: () => invoke('overview'),
  history: (limit = 200) => invoke('history', { limit }),
  check: (host) => invoke('check', { host }),
  trustScan: (host) => invoke('trust_scan', { host }),
  trustAccept: (host, fingerprint, replace) => invoke('trust_accept', { host, fingerprint, replace }),
  trustImport: (host, replace) => invoke('trust_import', { host, replace }),
  credentialSet: (host, purpose, secret) => invoke('credential_set', { host, purpose, secret }),
  hostAdd: (input) => invoke('host_add', { input }),
  hostUpdate: (host, changes) => invoke('host_update', { host, description: null, tags: null, clearIssues: false, ...changes }),
  hostRemove: (host) => invoke('host_remove', { host }),
  projectSave: (host, project) => invoke('project_save', { host, ...project }),
  projectRemove: (host, name) => invoke('project_remove', { host, name }),
  importPreview: (path) => invoke('import_preview', { path }),
  importApply: (path, names) => invoke('import_apply', { path, names }),
  agents: () => invoke('agents'),
  settingsGet: () => invoke('settings_get'),
  settingsSet: (settings) => invoke('settings_set', { settings }),
  revealDataDir: () => invoke('reveal_data_dir'),
  openTerminal: (host) => invoke('open_terminal', { host }),
  setTheme: (theme) => window.__TAURI__.window.getCurrentWindow().setTheme(theme),
};

export const message = errorMessage;

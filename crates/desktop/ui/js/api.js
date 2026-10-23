import { errorMessage } from './i18n.js';

// Every call returns the core library's JSON; failures reject with { code, message }.
const invoke = (command, args) => window.__TAURI__.core.invoke(command, args);

export const api = {
  overview: () => invoke('overview'),
  history: (limit = 200) => invoke('history', { limit }),
  check: (host) => invoke('check', { host }),
  trustReset: (host) => invoke('trust_reset', { host }),
  credentialSet: (host, purpose, secret) => invoke('credential_set', { host, purpose, secret }),
  hostAdd: (input) => invoke('host_add', { input }),
  hostUpdate: (host, input) => invoke('host_update', { host, input }),
  hostClearIssues: (host) => invoke('host_clear_issues', { host }),
  hostRemove: (host) => invoke('host_remove', { host }),
  importPreview: (path) => invoke('import_preview', { path }),
  importApply: (path, names) => invoke('import_apply', { path, names }),
  settingsGet: () => invoke('settings_get'),
  settingsSet: (settings) => invoke('settings_set', { settings }),
  revealDataDir: () => invoke('reveal_data_dir'),
  openTerminal: (host) => invoke('open_terminal', { host }),
  pickKeyFile: () => invoke('pick_key_file'),
  keyAgents: () => invoke('key_agents'),
  probe: (hosts) => invoke('probe', { hosts }),
  mcpClients: () => invoke('mcp_clients'),
  mcpSet: (client, connected) => invoke('mcp_set', { client, connected }),
  setTheme: (theme) => window.__TAURI__.window.getCurrentWindow().setTheme(theme),
};

export const message = errorMessage;

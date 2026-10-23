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
};

export function message(error) {
  return error?.message ?? String(error);
}

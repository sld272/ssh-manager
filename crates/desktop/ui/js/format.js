import { h, icon } from './dom.js';

export function relative(iso) {
  const then = new Date(iso);
  const seconds = Math.max(0, (Date.now() - then.getTime()) / 1000);
  if (seconds < 60) return '刚刚';
  if (seconds < 3600) return `${Math.floor(seconds / 60)} 分钟前`;
  const now = new Date();
  const time = then.toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit', hour12: false });
  if (then.toDateString() === now.toDateString()) return `今天 ${time}`;
  const yesterday = new Date(now);
  yesterday.setDate(now.getDate() - 1);
  if (then.toDateString() === yesterday.toDateString()) return `昨天 ${time}`;
  return `${then.getMonth() + 1} 月 ${then.getDate()} 日`;
}

// "14:18" today, otherwise the same wording as relative().
export function clock(iso) {
  const then = new Date(iso);
  if (then.toDateString() !== new Date().toDateString()) return relative(iso);
  return then.toLocaleTimeString('zh-CN', { hour: '2-digit', minute: '2-digit', hour12: false });
}

export function duration(ms) {
  if (ms < 100) return '<0.1 秒';
  if (ms < 1000) return `${(ms / 1000).toFixed(1)} 秒`;
  if (ms < 60000) return `${Math.round(ms / 100) / 10} 秒`;
  return `${Math.floor(ms / 60000)} 分 ${Math.round((ms % 60000) / 1000)} 秒`;
}

const STATUS = {
  succeeded: { label: '成功', tone: 'ok', icon: 'check' },
  command_failed: { label: '命令未成功', tone: 'err', icon: 'x' },
  connection_failed: { label: '连接失败', tone: 'err', icon: 'offline' },
  authentication_failed: { label: '认证失败', tone: 'err', icon: 'key' },
  host_key_required: { label: '需确认身份', tone: 'warn', icon: 'shield' },
  host_key_changed: { label: '服务器身份已变化', tone: 'err', icon: 'shield' },
  needs_interaction: { label: '需要你操作', tone: 'warn', icon: 'alert' },
  result_unknown: { label: '结果未知', tone: 'warn', icon: 'question' },
  transfer_failed: { label: '传输未完成', tone: 'err', icon: 'x' },
};

export function status(record) {
  return STATUS[record.status] ?? { label: record.status, tone: '', icon: 'question' };
}

export function statusPill(record) {
  const s = status(record);
  let label = s.label;
  if (record.status === 'command_failed' && record.exit_code !== null) label += ` · 退出码 ${record.exit_code}`;
  if (record.status === 'result_unknown' && record.termination === 'timeout') label += ' · 超时';
  if (record.status === 'result_unknown' && record.termination === 'cancelled') label += ' · 已取消';
  return h('span', { class: `pill ${s.tone}` }, icon(s.icon, 12, 2.2), label);
}

const KINDS = { exec: '执行命令', check: '检查连接', upload: '上传文件', download: '下载文件' };
export const kind = (k) => KINDS[k] ?? k;

const CALLERS = { cli: '你（命令行）', desktop: '你（桌面端）', library: '集成应用', mcp: 'MCP 客户端' };
export const caller = (c) => CALLERS[c] ?? c;
export const isAgent = (c) => !(c in CALLERS) || c === 'mcp';

export function address(connection) {
  const port = connection.port === 22 ? '' : `:${connection.port}`;
  return `${connection.user}@${connection.address}${port}`;
}

export function auth(connection) {
  const a = connection.authentication;
  if (a.kind === 'password') return a.credential_ref ? '密码已保存' : '密码';
  if (a.kind === 'key') return connection.identity_agent ? '密钥 · 指定代理' : '密钥';
  return connection.identity_agent ? '指定的 ssh-agent' : 'ssh-agent';
}

const ALGORITHMS = { 'ssh-ed25519': 'ED25519', 'ssh-rsa': 'RSA', 'ecdsa-sha2-nistp256': 'ECDSA', 'ecdsa-sha2-nistp384': 'ECDSA', 'ecdsa-sha2-nistp521': 'ECDSA' };
export const keyAlgorithm = (a) => ALGORITHMS[a] ?? a;

// "SHA256:pQ3vX8mN…nV7k" keeps both ends visible for a quick visual comparison.
export function shortFingerprint(fp) {
  return fp.length > 24 ? `${fp.slice(0, 15)}…${fp.slice(-4)}` : fp;
}

export function defaultConnection(host) {
  return host.connections.find((c) => c.id === host.default_connection);
}

export function source(host) {
  return host.source ? 'ssh_config' : 'manual';
}
export const SOURCES = { ssh_config: 'SSH 配置', manual: '手动添加' };

import { h, icon } from './dom.js';
import { language, t } from './i18n.js';

const timeOf = (d) => d.toLocaleTimeString(language() === 'en' ? 'en-GB' : 'zh-CN', { hour: '2-digit', minute: '2-digit', hour12: false });

export function relative(iso) {
  const then = new Date(iso);
  const seconds = Math.max(0, (Date.now() - then.getTime()) / 1000);
  if (seconds < 60) return t('刚刚');
  if (seconds < 3600) return t('{n} 分钟前', { n: Math.floor(seconds / 60) });
  const now = new Date();
  if (then.toDateString() === now.toDateString()) return t('今天 {time}', { time: timeOf(then) });
  const yesterday = new Date(now);
  yesterday.setDate(now.getDate() - 1);
  if (then.toDateString() === yesterday.toDateString()) return t('昨天 {time}', { time: timeOf(then) });
  return language() === 'en'
    ? then.toLocaleDateString('en', { month: 'short', day: 'numeric' })
    : `${then.getMonth() + 1} 月 ${then.getDate()} 日`;
}

// "14:18" today, otherwise the same wording as relative().
export function clock(iso) {
  const then = new Date(iso);
  return then.toDateString() === new Date().toDateString() ? timeOf(then) : relative(iso);
}

export function duration(ms) {
  if (ms < 100) return t('<0.1 秒');
  if (ms < 60000) return t('{n} 秒', { n: ms < 1000 ? (ms / 1000).toFixed(1) : Math.round(ms / 100) / 10 });
  return t('{m} 分 {s} 秒', { m: Math.floor(ms / 60000), s: Math.round((ms % 60000) / 1000) });
}

const STATUS = {
  succeeded: { label: '成功', tone: 'ok', icon: 'check' },
  command_failed: { label: '命令未成功', tone: 'err', icon: 'x' },
  connection_failed: { label: '连接失败', tone: 'err', icon: 'offline', hint: '请检查地址、端口、网络和跳板连接' },
  authentication_failed: { label: '认证失败', tone: 'err', icon: 'key', hint: '请检查登录账号及绑定的凭据' },
  // Recorded before servers were trusted on their first connection.
  host_key_required: { label: '需确认身份', tone: 'warn', icon: 'shield' },
  host_key_changed: { label: '服务器身份已变化', tone: 'err', icon: 'shield', hint: '服务器的密钥和上次连接时不同。如果服务器没有重装或更换，可能有人在冒充它。' },
  needs_interaction: { label: '需要你操作', tone: 'warn', icon: 'alert', hint: '认证需要用户操作，请检查密钥密码或硬件密钥' },
  result_unknown: { label: '结果未知', tone: 'warn', icon: 'question', hint: '未能确认远端执行结果，请检查服务器上的实际状态' },
  transfer_failed: { label: '传输未完成', tone: 'err', icon: 'x', hint: '传输没有完成，目标可能有部分内容' },
};

export function status(record) {
  const s = STATUS[record.status] ?? { label: record.status, tone: '', icon: 'question' };
  return { ...s, label: t(s.label), hint: s.hint && t(s.hint) };
}

export function statusPill(record) {
  const s = status(record);
  let label = s.label;
  if (record.status === 'command_failed' && record.exit_code !== null) label += t(' · 退出码 {code}', { code: record.exit_code });
  if (record.status === 'result_unknown' && record.termination === 'timeout') label += t(' · 超时');
  if (record.status === 'result_unknown' && record.termination === 'cancelled') label += t(' · 已取消');
  return h('span', { class: `pill ${s.tone}` }, icon(s.icon, 12, 2.2), label);
}

const KINDS = { exec: '执行命令', check: '检查连接', upload: '上传文件', download: '下载文件' };
export const kind = (k) => (KINDS[k] ? t(KINDS[k]) : k);

const CALLERS = { cli: '你（命令行）', desktop: '你（桌面端）', library: '集成应用', mcp: 'MCP 客户端' };
export const caller = (c) => (CALLERS[c] ? t(CALLERS[c]) : c);

export function address(connection) {
  const port = connection.port === 22 ? '' : `:${connection.port}`;
  return `${connection.user}@${connection.address}${port}`;
}

// The system ssh-agent has no socket of its own here; 1Password and Secretive are known by their sockets.
export function agentName(socket) {
  if (!socket) return t('系统 ssh-agent');
  if (socket.includes('com.1password')) return '1Password';
  if (socket.includes('Secretive')) return 'Secretive';
  return socket;
}

export function auth(connection) {
  const a = connection.authentication;
  if (a.kind === 'password') return t(a.credential_ref ? '密码已保存' : '密码');
  if (a.kind === 'key' && !connection.identity_agent) return t('密钥');
  return t('密钥 · {agent}', { agent: agentName(connection.identity_agent) });
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

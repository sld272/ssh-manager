import { h, icon, screen } from './dom.js';
import { t } from './i18n.js';

function group(title, ...rows) {
  return h('section', { class: 'settings-group' },
    h('h2', { class: 'section-title' }, title),
    h('div', { class: 'card settings-card' }, rows));
}

function row(label, control, hint) {
  return h('div', { class: 'setting-row' },
    h('div', { class: 'grow stack' }, h('span', null, label), hint && h('span', { class: 'small muted' }, hint)),
    control);
}

function segmented(label, options, value, onPick) {
  return h('div', { class: 'segmented', role: 'group', 'aria-label': label },
    options.map(([key, text]) => h('button', {
      type: 'button', 'aria-pressed': String(value === key), onclick: () => onPick(key),
    }, text)));
}

function status(ok, text) {
  return h('span', { class: `state ${ok ? 'ok' : 'warn'}`, style: 'font-size: 12px' }, h('span', { class: `dot ${ok ? 'ok' : 'warn'}` }), text);
}

export function settingsView(state, actions) {
  const view = state.settings;
  const s = view.settings;
  const env = view.environment;
  const change = (patch) => actions.saveSettings({ ...s, ...patch });

  const select = (label, options, value, onPick) => h('select', { class: 'select', 'aria-label': label, onchange: (e) => onPick(e.target.value) },
    options.map(([key, text]) => h('option', { value: key, selected: value === key }, text)));
  const language = select(t('语言'), [['system', t('跟随系统')], ['zh-CN', '简体中文'], ['en', 'English']], s.language, (v) => change({ language: v }));
  // Only installed terminals are listed.
  const terminal = select(t('默认终端'), view.terminals.map((app) => [app.id, app.id === 'terminal' ? t('系统终端') : app.name]), s.terminal, (v) => change({ terminal: v }));

  return screen({
    title: t('设置'),
    body: h('div', { class: 'main-body' }, h('div', { class: 'page settings' },
      group(t('通用'),
        row(t('语言'), language),
        row(t('外观'), segmented(t('外观'), [['system', t('跟随系统')], ['light', t('浅色')], ['dark', t('深色')]], s.theme, (theme) => change({ theme })))),
      group(t('终端'),
        row(t('默认终端'), terminal, t('在主机详情里点“连接”时打开。'))),
      group(t('本机环境'),
        row('OpenSSH', status(!!env.ssh_version, env.ssh_version ?? t('未找到'))),
        row(t('凭据组件'), status(env.credential_helper_available, t(env.credential_helper_available ? '可用' : '未找到'))),
        row(t('命令行工具'), env.command ? h('span', { class: 'mono small muted ellipsis', style: 'max-width: 280px' }, env.command) : status(false, t('未安装'))),
        row(t('数据目录'),
          h('button', { type: 'button', class: 'btn btn-sm', onclick: () => actions.revealDataDir() }, icon('folder', 13), t('在访达中显示')),
          h('span', { class: 'mono small muted ellipsis', title: env.data_dir }, env.data_dir.replace(/^\/Users\/[^/]+/, '~')))),
      group(t('关于'),
        row(t('版本'), h('span', { class: 'small muted' }, `ssh-manager ${env.version}`))))),
  });
}

export function environmentReady(view) {
  const env = view?.environment;
  return !env || (!!env.ssh_version && env.credential_helper_available);
}

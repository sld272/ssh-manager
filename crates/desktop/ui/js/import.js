import { h, icon, screen } from './dom.js';
import { address, auth } from './format.js';

// Names that importing `selected` brings in, including the jump hosts they depend on.
function closure(plan, selected) {
  const byConnection = new Map(plan.hosts.map((host) => [host.default_connection, host]));
  const result = new Set();
  const visit = (name) => {
    if (result.has(name)) return;
    result.add(name);
    const host = plan.hosts.find((x) => x.name === name);
    for (const id of host?.connections[0].jumps ?? []) {
      const jump = byConnection.get(id);
      if (jump) visit(jump.name);
    }
  };
  selected.forEach(visit);
  return result;
}

function dependents(plan, host) {
  return plan.hosts.filter((x) => x.connections[0].jumps.includes(host.default_connection)).map((x) => x.name);
}

function sourceCard(title, iconName, line, checked, disabled) {
  return h('button', { type: 'button', class: 'source', role: 'radio', 'aria-checked': String(checked), disabled },
    h('span', { class: 'source-title' }, icon(iconName, 15), title),
    h('span', { class: 'small', style: 'color: var(--text2)' }, line));
}

export function importView(state, actions) {
  const imp = state.import;
  const plan = imp.preview?.plan;
  const existing = new Set(imp.preview?.existing ?? []);
  const importable = plan ? plan.hosts.filter((x) => !existing.has(x.name)) : [];
  const total = plan ? closure(plan, imp.selected) : new Set();
  const conflicts = [...total].filter((name) => existing.has(name));
  const allChecked = importable.length > 0 && importable.every((x) => imp.selected.has(x.name));

  const pathInput = h('input', { class: 'input mono', value: imp.path, placeholder: '~/.ssh/config', 'aria-label': '配置文件路径' });

  const rows = plan?.hosts.map((host) => {
    const c = host.connections[0];
    const exists = existing.has(host.name);
    const deps = dependents(plan, host);
    const pill = exists
      ? h('span', { class: 'pill' }, '已在主机库中')
      : c.issues.length ? h('span', { class: 'pill warn' }, `需处理 ${c.issues.length} 项`) : h('span', { class: 'pill ok' }, '可导入');
    const body = h('span', { class: 'grow stack', style: 'gap: 1px' },
      h('span', { class: 'row', style: 'gap: 6px' },
        h('strong', null, host.name),
        deps.length > 0 && h('span', { class: 'small muted' }, `${deps.join('、')} 的跳板`)),
      h('span', { class: 'mono ellipsis', style: 'color: var(--text2)' }, `${address(c)} · ${auth(c)}`),
      c.issues.map((issue) => h('span', { class: 'small', style: 'color: var(--warn)' }, issue)),
      !exists && c.authentication.kind === 'password' && h('span', { class: 'small muted' }, '导入后在本机保存一次密码即可使用'));
    if (exists) {
      return h('div', { class: 'preview-row disabled' },
        h('input', { type: 'checkbox', class: 'check', disabled: true, 'aria-label': `${host.name} 已在主机库中` }), body, pill);
    }
    return h('label', { class: 'preview-row' },
      h('input', {
        type: 'checkbox', class: 'check', checked: imp.selected.has(host.name),
        onchange: (e) => { e.target.checked ? imp.selected.add(host.name) : imp.selected.delete(host.name); actions.render(); },
      }), body, pill);
  });

  const note = conflicts.length
    ? `跳板 ${conflicts.join('、')} 已在主机库中，暂时无法一起导入。请先取消依赖它的主机。`
    : total.size > imp.selected.size
      ? `会一同导入 ${total.size - imp.selected.size} 台跳板机。`
      : '导入后，为使用密码的主机保存密码，并确认服务器身份。';

  return screen({
    title: '导入主机',
    body: h('div', { class: 'main-body' }, h('div', { class: 'page' },
      h('p', { class: 'lead small' }, '从已有配置里挑选主机。原文件不会被修改，导入后以 ssh-manager 中的配置为准。'),
      h('div', { class: 'sources', role: 'radiogroup', 'aria-label': '导入来源' },
        sourceCard('SSH 配置', 'file', plan ? `找到 ${plan.hosts.length} 台主机` : '读取中…', true, false),
        sourceCard('Tailscale', 'network', '即将支持', false, true),
        sourceCard('备份文件', 'folder', '即将支持', false, true)),
      h('form', {
        class: 'import-path', onsubmit: (e) => { e.preventDefault(); imp.path = pathInput.value.trim(); actions.loadImport(); },
      }, pathInput, h('button', { type: 'submit', class: 'btn' }, icon('refresh', 14), '重新读取')),

      imp.error && h('div', { class: 'banner err' }, icon('alert', 16), h('div', { class: 'grow' }, imp.error)),
      plan?.warnings.length > 0 && h('div', { class: 'banner' }, icon('alert', 16),
        h('div', { class: 'grow' }, h('strong', null, '部分内容未能读取'), h('ul', null, plan.warnings.map((w) => h('li', { class: 'small' }, w))))),

      plan && h('div', { class: 'import-body' },
        h('section', { class: 'card', 'aria-label': '预览' },
          h('div', { class: 'preview-head' },
            h('label', { class: 'row', style: 'font-weight: 600' },
              h('input', {
                type: 'checkbox', class: 'check', checked: allChecked, disabled: importable.length === 0,
                onchange: (e) => { importable.forEach((x) => (e.target.checked ? imp.selected.add(x.name) : imp.selected.delete(x.name))); actions.render(); },
              }), '全选'),
            h('span', { class: 'small muted', style: 'margin-left: auto' }, `已选 ${imp.selected.size} / ${importable.length}`)),
          plan.hosts.length ? rows : h('p', { class: 'list-empty' }, '这个文件里没有可导入的主机。')),
        (plan.ignored.length > 0 || plan.skipped.length > 0) && h('aside', { class: 'aside', 'aria-label': '未导入的配置' },
          plan.ignored.length > 0 && h('details', { class: 'card' },
            h('summary', null, `已忽略 ${plan.ignored.length} 个选项`),
            h('p', { class: 'small muted', style: 'margin-top: 6px' }, '不影响 ssh-manager 的连接，不需要处理。'),
            plan.ignored.map((line) => h('div', { class: 'aside-item break' }, line))),
          plan.skipped.length > 0 && h('details', { class: 'card', open: true },
            h('summary', null, `已跳过 ${plan.skipped.length} 个 Match 条件块`),
            h('p', { class: 'small muted', style: 'margin-top: 6px' }, '条件块不会被执行，其中的设置也不会导入。'),
            plan.skipped.map((line) => h('div', { class: 'aside-item break mono' }, line))))))),
    footer: plan && h('div', { class: 'footer-bar' },
      h('p', { class: 'grow small', style: `color: var(${conflicts.length ? '--warn' : '--text2'})` }, note),
      h('button', { type: 'button', class: 'btn', onclick: () => actions.navigate('hosts') }, '取消'),
      h('button', {
        type: 'button', class: 'btn btn-primary', disabled: total.size === 0 || conflicts.length > 0 || imp.applying,
        onclick: () => actions.applyImport(),
      }, imp.applying ? '导入中…' : `导入 ${total.size} 台`)),
  });
}

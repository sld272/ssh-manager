import { h, icon } from './dom.js';
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

function sourceCard(title, iconName, line1, line2, checked, disabled) {
  return h('button', { type: 'button', class: 'source', role: 'radio', 'aria-checked': String(checked), disabled },
    h('span', { class: 'source-title' }, icon(iconName, 18), title),
    h('span', { class: 'small', style: 'color: var(--text2)' }, line1),
    h('span', { class: 'small', style: 'color: var(--body2)' }, line2));
}

export function importView(state, actions) {
  const imp = state.import;
  const preview = imp.preview;
  const plan = preview?.plan;
  const existing = new Set(preview?.existing ?? []);
  const importable = plan ? plan.hosts.filter((x) => !existing.has(x.name)) : [];
  const total = plan ? closure(plan, imp.selected) : new Set();
  const conflicts = [...total].filter((name) => existing.has(name));

  const pathInput = h('input', { class: 'input mono', value: imp.path, placeholder: '~/.ssh/config', 'aria-label': '配置文件路径' });

  const rows = plan?.hosts.map((host) => {
    const c = host.connections[0];
    const exists = existing.has(host.name);
    const deps = dependents(plan, host);
    const pill = exists
      ? h('span', { class: 'pill' }, '已在主机库中')
      : c.issues.length ? h('span', { class: 'pill warn' }, `需处理 ${c.issues.length} 项`) : h('span', { class: 'pill ok' }, '可导入');
    const body = h('span', { class: 'grow stack', style: 'gap: 2px' },
      h('span', { style: 'font-weight: 600' }, host.name,
        deps.length > 0 && h('span', { class: 'small muted', style: 'font-weight: 400' }, `  ${deps.join('、')} 的跳板`)),
      h('span', { class: 'mono', style: 'color: var(--body2)' }, `${address(c)} · ${auth(c)}`),
      c.issues.map((issue) => h('span', { class: 'small', style: 'color: var(--warn)' }, issue)),
      !exists && c.authentication.kind === 'password' && h('span', { class: 'small', style: 'color: var(--text2)' }, '导入后在本机保存一次密码即可使用'));
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

  const allChecked = importable.length > 0 && importable.every((x) => imp.selected.has(x.name));

  return h('div', { class: 'main' }, h('div', { class: 'page' },
    h('div', null,
      h('h1', { class: 'page-title' }, '导入主机'),
      h('p', { class: 'page-sub' }, '从已有配置里挑选主机。原文件不会被修改，导入后以 ssh-manager 中的配置为准。')),

    h('div', { class: 'sources', role: 'radiogroup', 'aria-label': '导入来源' },
      sourceCard('SSH 配置', 'file', imp.path || '~/.ssh/config', plan ? `找到 ${plan.hosts.length} 台主机` : '读取中…', true, false),
      sourceCard('Tailscale', 'network', '你的 tailnet', '即将支持', false, true),
      sourceCard('备份文件', 'folder', 'ssh-manager 导出的 JSON', '即将支持', false, true)),

    h('form', {
      class: 'row', onsubmit: (e) => { e.preventDefault(); imp.path = pathInput.value.trim(); actions.loadImport(); },
    }, h('label', { class: 'grow' }, pathInput), h('button', { type: 'submit', class: 'btn' }, icon('refresh', 16), '重新读取')),

    imp.error && h('div', { class: 'banner err' }, icon('alert', 18), h('div', { class: 'grow' }, imp.error)),
    plan?.warnings.length > 0 && h('div', { class: 'banner' }, icon('alert', 18),
      h('div', { class: 'grow' }, h('strong', null, '部分内容未能读取'), h('ul', null, plan.warnings.map((w) => h('li', { class: 'small' }, w))))),

    plan && h('div', { class: 'import-body' },
      h('section', { class: 'card preview', 'aria-label': '预览' },
        h('div', { class: 'preview-head' },
          h('label', { class: 'row', style: 'font-weight: 600' },
            h('input', {
              type: 'checkbox', class: 'check', checked: allChecked, disabled: importable.length === 0,
              onchange: (e) => { importable.forEach((x) => (e.target.checked ? imp.selected.add(x.name) : imp.selected.delete(x.name))); actions.render(); },
            }), '全选可导入的主机'),
          h('span', { class: 'small', style: 'margin-left: auto; color: var(--text2)' }, `已选 ${imp.selected.size} 台`)),
        plan.hosts.length ? rows : h('p', { class: 'muted', style: 'padding: 18px' }, '这个文件里没有可导入的主机。')),
      h('aside', { class: 'aside', 'aria-label': '未导入的配置' },
        plan.ignored.length > 0 && h('details', { class: 'card', open: true },
          h('summary', null, `已忽略 ${plan.ignored.length} 个选项`),
          h('p', { class: 'small', style: 'margin: 8px 0 0; color: var(--text2)' }, '这些选项不影响 ssh-manager 的连接，不需要处理。'),
          plan.ignored.map((line) => h('div', { class: 'aside-item small break' }, line))),
        plan.skipped.length > 0 && h('details', { class: 'card', open: true },
          h('summary', null, `已跳过 ${plan.skipped.length} 个 Match 条件块`),
          h('p', { class: 'small', style: 'margin: 8px 0 0; color: var(--text2)' }, '条件块不会被执行，其中的设置也不会导入。'),
          plan.skipped.map((line) => h('div', { class: 'aside-item small break mono' }, line))))),

    plan && h('div', { class: 'card footer-bar' },
      h('p', { class: 'grow small', style: 'flex: 1 1 320px; color: var(--text2)' },
        conflicts.length
          ? `跳板 ${conflicts.join('、')} 已在主机库中，暂时无法一起导入。请先取消依赖它的主机。`
          : total.size > imp.selected.size
            ? `会一同导入 ${total.size - imp.selected.size} 台跳板。导入后，记得为使用密码的主机保存密码，并确认每台服务器的身份。`
            : '导入后，记得为使用密码的主机保存密码，并确认每台服务器的身份。'),
      h('button', { type: 'button', class: 'btn', onclick: () => actions.navigate('hosts') }, '取消'),
      h('button', {
        type: 'button', class: 'btn btn-primary', disabled: total.size === 0 || conflicts.length > 0 || imp.applying,
        onclick: () => actions.applyImport(),
      }, imp.applying ? '导入中…' : `导入 ${total.size} 台主机`))));
}

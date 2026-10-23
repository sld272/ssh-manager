import { h, icon, screen } from './dom.js';
import { caller, clock, duration, kind, statusPill } from './format.js';
import { t } from './i18n.js';

const FILTERS = [
  ['all', '全部', () => true],
  ['failed', '未成功', (r) => r.status !== 'succeeded' && r.status !== 'result_unknown'],
  ['unknown', '结果未知', (r) => r.status === 'result_unknown'],
];

export function activityView(state, actions) {
  const [, , test] = FILTERS.find(([key]) => key === state.activity.filter);
  const records = state.history.filter((r) => test(r) && (!state.activity.caller || r.caller === state.activity.caller));
  const dayAgo = Date.now() - 86400000;
  const unknown = state.history.find((r) => r.status === 'result_unknown' && r.kind !== 'check' && new Date(r.started_at).getTime() > dayAgo);

  return screen({
    title: t('活动'),
    tools: h('div', { class: 'segmented', role: 'group', 'aria-label': t('筛选') },
      FILTERS.map(([key, label]) => h('button', {
        type: 'button', 'aria-pressed': String(state.activity.filter === key),
        onclick: () => { state.activity.filter = key; actions.render(); },
      }, t(label)))),
    body: h('div', { class: 'main-body' }, h('div', { class: 'page' },
      h('p', { class: 'lead small' }, t('你和 agent 在各台主机上做过的操作。只记录目标、时间、调用方和目的，不保存命令内容和输出。')),

      [...state.running.values()].map((op) => h('div', { class: 'running' },
        h('span', { class: 'pulse' }),
        h('span', { class: 'grow' }, t('正在{kind}：{host}', { kind: kind(op.kind), host: op.hostName })),
        h('span', { class: 'small' }, t('{time} 开始', { time: clock(op.startedAt) })))),

      unknown && h('div', { class: 'banner' },
        icon('question', 16),
        h('div', { class: 'grow' },
          t('{host} 上的“{what}”结果未知，远端可能已经执行。不要直接重试，先检查实际状态。', { host: unknown.host_name, what: unknown.reason || kind(unknown.kind) })),
        h('button', { type: 'button', class: 'btn btn-sm', onclick: () => actions.openHost(unknown.host_id) }, t('查看主机'))),

      records.length
        ? h('div', { class: 'card table-box' }, h('table', null,
          h('thead', null, h('tr', null, ['时间', '调用方', '主机', '操作', '结果', '耗时'].map((label) => h('th', { scope: 'col' }, t(label))))),
          h('tbody', null, records.map((r) => h('tr', null,
            h('td', { class: 'num' }, clock(r.started_at)),
            h('td', { class: 'ellipsis', style: 'max-width: 140px' }, caller(r.caller)),
            h('td', { class: 'mono' }, r.host_name),
            h('td', null, kind(r.kind), r.reason && h('div', { class: 'small', style: 'color: var(--text2)' }, r.reason)),
            h('td', null, statusPill(r)),
            h('td', { class: 'num' }, duration(r.duration_ms)))))))
        : h('div', { class: 'card empty' },
          h('div', { class: 'empty-title' }, t(state.history.length ? '没有符合条件的记录' : '还没有操作记录')),
          h('p', null, t('检查连接、执行命令或传输文件后，记录会出现在这里。'))))),
  });
}

export function activityNav(state, actions) {
  const callers = new Map();
  for (const r of state.history) callers.set(r.caller, (callers.get(r.caller) ?? 0) + 1);
  if (!callers.size) return null;
  return h('div', { class: 'nav-group nav-filters' },
    h('div', { class: 'nav-label' }, t('调用方')),
    [...callers].map(([name, count]) => h('button', {
      type: 'button', class: 'nav-item', 'aria-pressed': String(state.activity.caller === name),
      onclick: () => { state.activity.caller = state.activity.caller === name ? null : name; actions.render(); },
    }, h('span', { class: 'grow ellipsis' }, caller(name)), h('span', { class: 'nav-count' }, count))));
}

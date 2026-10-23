import { h, icon } from './dom.js';
import { caller, clock, duration, kind, statusPill } from './format.js';

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

  return h('div', { class: 'main' }, h('div', { class: 'page' },
    h('div', { class: 'page-head' },
      h('div', { class: 'grow', style: 'flex: 1 1 280px' },
        h('h1', { class: 'page-title' }, '活动'),
        h('p', { class: 'page-sub' }, '你和 agent 在各台主机上做过的操作。记录只保存目标、时间、调用方和目的，不保存命令内容和输出。')),
      h('div', { class: 'segmented', role: 'group', 'aria-label': '筛选' },
        FILTERS.map(([key, label]) => h('button', {
          type: 'button', 'aria-pressed': String(state.activity.filter === key),
          onclick: () => { state.activity.filter = key; actions.render(); },
        }, label)))),

    [...state.running.values()].map((op) => h('section', { class: 'running', 'aria-label': '进行中' },
      h('div', { class: 'running-icon' }, icon('terminal', 20)),
      h('div', { class: 'grow stack', style: 'flex: 1 1 320px' },
        h('strong', null, `正在${kind(op.kind)}：${op.hostName}`),
        h('span', { class: 'small', style: 'color: var(--text2)' }, `由你在桌面端发起 · ${clock(op.startedAt)} 开始`)),
      h('span', { class: 'row small', style: 'color: var(--accent-text); font-weight: 500' }, h('span', { class: 'pulse' }), '进行中'))),

    unknown && h('div', { class: 'banner' },
      icon('question', 18),
      h('div', { class: 'grow', style: 'flex: 1 1 300px' },
        `${unknown.host_name} 上的“${unknown.reason || kind(unknown.kind)}”结果未知，远端可能已经执行。不要直接重试，先检查实际状态。`),
      h('button', { type: 'button', class: 'btn btn-sm', onclick: () => actions.openHost(unknown.host_id) }, `查看 ${unknown.host_name}`)),

    records.length
      ? h('div', { class: 'card table-box' }, h('table', null,
        h('thead', null, h('tr', null, ['时间', '调用方', '主机', '操作', '目的', '结果', '耗时'].map((t) => h('th', { scope: 'col' }, t)))),
        h('tbody', null, records.map((r) => h('tr', null,
          h('td', { class: 'num' }, clock(r.started_at)),
          h('td', null, caller(r.caller)),
          h('td', { class: 'mono' }, r.host_name),
          h('td', null, kind(r.kind)),
          h('td', { style: 'color: var(--body2)' }, r.reason || h('span', { class: 'muted' }, '—')),
          h('td', null, statusPill(r)),
          h('td', { class: 'num' }, duration(r.duration_ms)))))))
      : h('div', { class: 'card empty' },
        h('div', { class: 'empty-title' }, state.history.length ? '没有符合条件的记录' : '还没有操作记录'),
        h('p', null, '检查连接、执行命令或传输文件后，记录会出现在这里。'))));
}

export function activityNav(state, actions) {
  const callers = new Map();
  for (const r of state.history) callers.set(r.caller, (callers.get(r.caller) ?? 0) + 1);
  if (!callers.size) return null;
  return h('div', { class: 'nav-group' },
    h('div', { class: 'nav-label' }, '调用方'),
    [...callers].map(([name, count]) => h('button', {
      type: 'button', class: 'nav-item sub', 'aria-pressed': String(state.activity.caller === name),
      onclick: () => { state.activity.caller = state.activity.caller === name ? null : name; actions.render(); },
    }, h('span', { class: 'grow ellipsis' }, caller(name)), h('span', { class: 'nav-count' }, count))));
}

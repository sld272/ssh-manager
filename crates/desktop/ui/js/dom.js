import { t } from './i18n.js';

// Builds DOM nodes. Strings always become text nodes, so host notes or remote output never turn into markup.
export function h(tag, props, ...children) {
  const el = document.createElement(tag);
  for (const [key, value] of Object.entries(props ?? {})) {
    if (value === undefined || value === null || value === false) continue;
    if (key === 'class') el.className = value;
    else if (key.startsWith('on')) el.addEventListener(key.slice(2).toLowerCase(), value);
    else if (key === 'value') el.value = value;
    else if (key === 'checked') el.checked = true;
    else el.setAttribute(key, value === true ? '' : String(value));
  }
  append(el, children);
  return el;
}

export function append(el, children) {
  for (const child of children.flat(Infinity)) {
    if (child === null || child === undefined || child === false) continue;
    el.append(child instanceof Node ? child : document.createTextNode(String(child)));
  }
  return el;
}

export function replace(el, ...children) {
  el.replaceChildren();
  return append(el, children);
}

const PATHS = {
  terminal: '<path d="M4 6l6 6-6 6M12 18h8"/>',
  server: '<rect x="3" y="4" width="18" height="7" rx="2"/><rect x="3" y="13" width="18" height="7" rx="2"/><path d="M7 7.5h.01M7 16.5h.01"/>',
  activity: '<path d="M3 12h4l3-8 4 16 3-8h4"/>',
  import: '<path d="M12 3v12M7 10l5 5 5-5M4 21h16"/>',
  search: '<circle cx="11" cy="11" r="7"/><path d="M20 20l-3.5-3.5"/>',
  plus: '<path d="M12 5v14M5 12h14"/>',
  check: '<path d="M5 12l5 5 9-10"/>',
  x: '<path d="M6 6l12 12M18 6L6 18"/>',
  shield: '<path d="M12 3l8 3v6c0 5-3.5 8-8 9-4.5-1-8-4-8-9V6z"/>',
  shieldCheck: '<path d="M12 3l8 3v6c0 5-3.5 8-8 9-4.5-1-8-4-8-9V6z"/><path d="M9 12l2 2 4-4"/>',
  key: '<circle cx="8" cy="15" r="4"/><path d="M11 12l9-9M17 6l3 3"/>',
  alert: '<path d="M12 3l10 18H2z"/><path d="M12 10v4M12 17h.01"/>',
  question: '<circle cx="12" cy="12" r="9"/><path d="M9.5 9.5a2.5 2.5 0 1 1 3.5 2.3c-.7.3-1 .9-1 1.7M12 17h.01"/>',
  offline: '<path d="M2 8.5a15 15 0 0 1 20 0M5 12a10 10 0 0 1 14 0M8.5 15.5a5 5 0 0 1 7 0M3 3l18 18"/>',
  arrow: '<path d="M5 12h14M13 6l6 6-6 6"/>',
  folder: '<path d="M3 6a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/>',
  file: '<path d="M6 3h8l4 4v14H6z"/><path d="M14 3v4h4M9 12h6M9 16h6"/>',
  refresh: '<path d="M20 11a8 8 0 1 0-2.3 5.7M20 4v7h-7"/>',
  system: '<rect x="3" y="4" width="18" height="16" rx="2"/><path d="M7 9l3 3-3 3M13 15h4"/>',
  network: '<circle cx="5" cy="12" r="2"/><circle cx="12" cy="12" r="2"/><circle cx="19" cy="12" r="2"/><circle cx="12" cy="5" r="2"/><circle cx="12" cy="19" r="2"/>',
  copy: '<rect x="8" y="8" width="12" height="12" rx="2"/><path d="M16 8V6a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v8a2 2 0 0 0 2 2h2"/>',
  more: '<circle cx="5" cy="12" r="1"/><circle cx="12" cy="12" r="1"/><circle cx="19" cy="12" r="1"/>',
  trash: '<path d="M4 7h16M10 11v6M14 11v6M6 7l1 13h10l1-13M9 7V4h6v3"/>',
  sidebar: '<rect x="3" y="4" width="18" height="16" rx="2"/><path d="M9 4v16"/>',
  settings: '<circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z"/>',
};

// Icon markup comes only from the constants above, never from data.
export function icon(name, size = 16, stroke = 1.8) {
  const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  svg.setAttribute('width', size);
  svg.setAttribute('height', size);
  svg.setAttribute('viewBox', '0 0 24 24');
  svg.setAttribute('fill', 'none');
  svg.setAttribute('stroke', 'currentColor');
  svg.setAttribute('stroke-width', stroke);
  svg.setAttribute('stroke-linecap', 'round');
  svg.setAttribute('stroke-linejoin', 'round');
  svg.setAttribute('aria-hidden', 'true');
  svg.innerHTML = PATHS[name];
  return svg;
}

// The sidebar toggle only shows in a narrow window; app.js owns the sidebar and listens for these.
const sidebar = (action) => () => window.dispatchEvent(new CustomEvent('sidebar', { detail: action }));

// A view: a toolbar that also drags the window (the title bar is hidden), then the view's own panes.
export function screen({ title, sub, tools, body, footer }) {
  const drag = { 'data-tauri-drag-region': true };
  return h('div', { class: 'main' },
    h('header', { class: 'toolbar', ...drag },
      h('button', {
        type: 'button', class: 'icon-btn sidebar-toggle', 'aria-label': t('显示边栏'), title: t('显示边栏'),
        onclick: sidebar('toggle'), onmouseenter: sidebar('hover'), onmouseleave: sidebar('leave'),
      }, icon('sidebar', 16)),
      h('h1', { class: 'toolbar-title', ...drag }, title),
      sub && h('span', { class: 'toolbar-sub', ...drag }, sub),
      h('div', { class: 'spacer', ...drag }),
      tools),
    body,
    footer);
}

export function toast(message) {
  const box = document.getElementById('toast');
  const item = h('div', null, message);
  box.replaceChildren(item);
  setTimeout(() => item.remove(), 3200);
}

// One popup menu at a time, opened under a button or at the pointer for a right click. It closes on a
// click elsewhere, Escape, scrolling or leaving the window.
let openMenu = null;
let closedBy = null;
export function closeMenu() {
  openMenu?.remove();
  openMenu = null;
}
export function showMenu(items, at) {
  // The press on the button that opened the menu already closed it; that click must not reopen it.
  if (closedBy && closedBy === at) {
    closedBy = null;
    return;
  }
  closeMenu();
  const el = h('div', { class: 'menu', role: 'menu' }, items.filter(Boolean).map((item) => (item === '-'
    ? h('div', { class: 'menu-sep', role: 'separator' })
    : h('button', {
      type: 'button', role: 'menuitem', class: `menu-item${item.danger ? ' danger' : ''}`,
      onclick: () => { closeMenu(); item.run(); },
    }, item.label))));
  el.anchor = at instanceof Element ? at : null;
  document.body.append(el);
  const box = el.getBoundingClientRect();
  const [x, y] = el.anchor
    ? [el.anchor.getBoundingClientRect().right - box.width, el.anchor.getBoundingClientRect().bottom + 4]
    : at;
  el.style.left = `${Math.max(8, Math.min(x, window.innerWidth - box.width - 8))}px`;
  el.style.top = `${Math.max(8, Math.min(y, window.innerHeight - box.height - 8))}px`;
  openMenu = el;
}
document.addEventListener('pointerdown', (e) => {
  if (!openMenu || openMenu.contains(e.target)) return;
  closedBy = openMenu.anchor?.contains(e.target) ? openMenu.anchor : null;
  closeMenu();
}, true);
document.addEventListener('keydown', (e) => {
  if (!openMenu) return;
  if (e.key === 'Escape') closeMenu();
  if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
    e.preventDefault();
    const buttons = [...openMenu.querySelectorAll('.menu-item')];
    const i = buttons.indexOf(document.activeElement);
    buttons[(i + (e.key === 'ArrowDown' ? 1 : buttons.length - 1)) % buttons.length]?.focus();
  }
});
window.addEventListener('blur', closeMenu);
// Elsewhere WebKit's own menu only offers Reload. Text fields and selected text keep it for copy and paste;
// places with their own menu, such as host rows, open it and suppress this one.
document.addEventListener('contextmenu', (e) => {
  const editable = e.target.closest('input, textarea');
  if (!editable && document.getSelection()?.isCollapsed !== false) e.preventDefault();
});
document.addEventListener('scroll', closeMenu, true);

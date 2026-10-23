import { api, message } from './api.js';
import { h, icon, replace, toast } from './dom.js';
import { defaultConnection } from './format.js';
import { t } from './i18n.js';

function open(content) {
  const el = h('dialog', null, content);
  el.addEventListener('close', () => el.remove());
  document.body.append(el);
  el.showModal();
  return el;
}

function head(title, subtitle, tone = '', iconName = 'shield') {
  return h('div', { class: 'dialog-head' },
    h('div', { class: `dialog-icon ${tone}` }, icon(iconName, 17)),
    h('div', { class: 'grow stack', style: 'gap: 4px' },
      h('h2', { class: 'dialog-title' }, title),
      subtitle && h('p', { class: 'muted' }, subtitle)),
  );
}

// A form dialog: `fields` render inside; `submit` resolves to close, or throws to show its message.
function formDialog({ title, subtitle, tone, iconName, fields, submitLabel, submit, danger }) {
  const error = h('p', { class: 'error-text', hidden: true });
  const button = h('button', { type: 'submit', class: danger ? 'btn btn-danger' : 'btn btn-primary' }, submitLabel);
  let el;
  const form = h('form', {
    class: 'dialog',
    onsubmit: async (event) => {
      event.preventDefault();
      button.disabled = true;
      error.hidden = true;
      try {
        await submit();
        el.close();
      } catch (e) {
        error.textContent = message(e);
        error.hidden = false;
      } finally {
        button.disabled = false;
      }
    },
  },
  head(title, subtitle, tone, iconName),
  fields,
  error,
  h('div', { class: 'dialog-actions' },
    h('button', { type: 'button', class: 'btn', onclick: () => el.close() }, t('取消')),
    button));
  el = open(form);
  return el;
}

function field(label, input) {
  return h('label', { class: 'field' }, h('span', null, label), input);
}

export function confirmDialog({ title, body, confirmLabel }) {
  return new Promise((resolve) => {
    let confirmed = false;
    const el = formDialog({
      title, subtitle: body, tone: 'warn', iconName: 'alert', danger: true,
      fields: null, submitLabel: confirmLabel,
      submit: async () => { confirmed = true; },
    });
    el.addEventListener('close', () => resolve(confirmed));
  });
}

export function credentialDialog(view, purpose, onDone) {
  const passphrase = purpose === 'key_passphrase';
  const input = h('input', { class: 'input', type: 'password', autocomplete: 'off', required: true, autofocus: true });
  formDialog({
    title: t(passphrase ? '保存 {name} 的密钥密码' : '保存 {name} 的登录密码', { name: view.name }),
    subtitle: t(passphrase ? '就是解锁这个密钥文件时输入的密码。保存在系统钥匙串中，agent 能用它连接，但看不到它。' : '保存在系统钥匙串中。agent 可以用它登录，但看不到它。'),
    iconName: 'key',
    fields: field(t(passphrase ? '密钥密码' : '登录密码'), input),
    submitLabel: t('保存'),
    submit: async () => {
      try {
        await api.credentialSet(view.id, purpose, input.value);
      } finally {
        input.value = '';
      }
      toast(t(passphrase ? '密钥密码已保存' : '密码已保存'));
      onDone();
    },
  });
}

// Tags show as chips: Enter adds the typed tag, the × on a chip removes it.
function tagEditor(initial) {
  const tags = [...initial];
  const input = h('input', { class: 'tag-input', placeholder: t('输入后按回车添加'), 'aria-label': t('添加标签') });
  const box = h('div', { class: 'tag-editor', onclick: (e) => { if (e.target === box) input.focus(); } });
  const add = () => {
    const tag = input.value.trim();
    if (tag && !tags.includes(tag)) tags.push(tag);
    input.value = '';
    draw();
  };
  // Redrawing moves the input, which drops its focus.
  const redraw = () => { draw(); input.focus(); };
  function draw() {
    replace(box,
      tags.map((tag) => h('span', { class: 'chip tag-chip' }, tag, h('button', {
        type: 'button', class: 'chip-remove', 'aria-label': t('删除标签 {tag}', { tag }),
        onclick: () => { tags.splice(tags.indexOf(tag), 1); redraw(); },
      }, icon('x', 10, 2.2)))),
      input);
  }
  input.addEventListener('keydown', (e) => {
    // Enter while a Chinese input method is composing only confirms the characters.
    if (e.key === 'Enter' && !e.isComposing) { e.preventDefault(); add(); input.focus(); }
    if (e.key === 'Backspace' && !input.value && tags.length) { tags.pop(); redraw(); }
  });
  draw();
  // A tag typed but not yet confirmed with Enter still counts when the form is saved.
  return { el: box, values: () => { add(); return tags; } };
}

// Key files are chosen in Finder; imported hosts can list several.
function keyFiles(initial) {
  const paths = [...initial];
  const list = h('div', { class: 'stack', style: 'gap: 6px' });
  function draw() {
    replace(list,
      paths.map((path) => h('div', { class: 'key-file' },
        icon('key', 13),
        h('span', { class: 'mono grow ellipsis', title: path }, path),
        h('button', {
          type: 'button', class: 'icon-btn', 'aria-label': t('移除 {path}', { path }), title: t('移除'),
          onclick: () => { paths.splice(paths.indexOf(path), 1); draw(); },
        }, icon('x', 12)))),
      h('button', {
        type: 'button', class: 'btn btn-sm', style: 'align-self: flex-start',
        onclick: async () => {
          const path = await api.pickKeyFile().catch((e) => { toast(message(e)); return null; });
          if (path && !paths.includes(path)) { paths.push(path); draw(); }
        },
      }, icon('folder', 13), t(paths.length ? '添加密钥文件…' : '选择密钥文件…')));
  }
  draw();
  return { el: list, values: () => paths };
}

// The add and edit forms share their fields; `view` is the host being edited.
function hostDialog(view, { title, subtitle, submitLabel, save, done }) {
  const c = view && defaultConnection(view);
  const a = c?.authentication;
  let auth = a?.kind ?? 'key';
  const name = h('input', { class: 'input', required: true, autofocus: !view, placeholder: 'blog', value: view?.name });
  const addr = h('input', { class: 'input', required: true, placeholder: t('example.com 或 IP 地址'), value: c?.address });
  const user = h('input', { class: 'input', required: true, placeholder: 'deploy', value: c?.user });
  const port = h('input', { class: 'input', type: 'number', min: 1, max: 65535, required: true, value: c?.port ?? 22 });
  const keys = keyFiles(a?.kind === 'key' ? a.paths : []);
  const keyField = h('div', { class: 'field' }, h('span', null, t('密钥文件')), keys.el);
  const saved = a?.kind === 'password' && !!a.credential_ref;
  const password = h('input', {
    class: 'input', type: 'password', autocomplete: 'off',
    placeholder: t(saved ? '已保存，留空则不修改' : '可以留空，连接时再输入'),
  });
  const passwordField = field(t('登录密码'), password);
  const description = h('input', { class: 'input', placeholder: t('这台机器是做什么的'), value: view?.context.description });
  const tags = tagEditor(view?.tags ?? []);
  const choices = [['key', t('密钥')], ['agent', 'ssh-agent'], ['password', t('密码')]];
  const seg = h('div', { class: 'segmented full', role: 'group', 'aria-label': t('登录方式') });
  const drawSeg = () => {
    replace(seg, choices.map(([v, label]) => h('button', {
      type: 'button', 'aria-pressed': String(auth === v),
      onclick: () => { auth = v; drawSeg(); },
    }, label)));
    keyField.hidden = auth !== 'key';
    passwordField.hidden = auth !== 'password';
  };
  drawSeg();
  formDialog({
    title, subtitle, submitLabel,
    iconName: 'server',
    fields: h('div', { class: 'stack', style: 'gap: 14px' },
      h('div', { class: 'field-row' }, field(t('名称'), name), field(t('登录用户'), user)),
      h('div', { class: 'field-row port' }, field(t('地址'), addr), field(t('端口'), port)),
      h('div', { class: 'field' }, h('span', null, t('登录方式')), seg),
      keyField,
      passwordField,
      field(t('说明'), description),
      h('div', { class: 'field' }, h('span', null, t('标签')), tags.el)),
    submit: async () => {
      const host = await save({
        name: name.value, address: addr.value, user: user.value, port: Number(port.value), auth,
        keyPaths: auth === 'key' ? keys.values() : [],
        description: description.value, tags: tags.values(),
      });
      if (auth === 'password' && password.value) {
        try {
          await api.credentialSet(host, 'login_password', password.value);
        } finally {
          password.value = '';
        }
      }
      done();
    },
  });
}

export function addHostDialog(onDone) {
  let added;
  hostDialog(null, {
    title: t('添加主机'),
    submitLabel: t('添加'),
    // Saving the password again after a failed attempt must not add the host twice.
    save: async (input) => {
      added ??= await api.hostAdd(input);
      return added.id;
    },
    done: () => {
      toast(t('已添加 {name}', { name: added.name }));
      onDone(added);
    },
  });
}

export function editHostDialog(view, onDone) {
  hostDialog(view, {
    title: t('编辑 {name}', { name: view.name }),
    subtitle: t('更换地址或端口后，下次连接会重新记录服务器身份。'),
    submitLabel: t('保存'),
    save: async (input) => {
      await api.hostUpdate(view.id, input);
      return view.id;
    },
    done: onDone,
  });
}

export function projectDialog(view, onDone) {
  const name = h('input', { class: 'input', required: true, autofocus: true, placeholder: 'blog' });
  const path = h('input', { class: 'input', required: true, placeholder: '/srv/blog' });
  const description = h('input', { class: 'input', placeholder: t('例如：Docker Compose 部署') });
  formDialog({
    title: t('添加项目'),
    subtitle: t('记下 {name} 上的项目目录，agent 可以直接在里面工作。', { name: view.name }),
    iconName: 'folder',
    fields: h('div', { class: 'stack', style: 'gap: 14px' },
      h('div', { class: 'field-row' }, field(t('名称'), name), field(t('远端路径'), path)),
      field(t('说明'), description)),
    submitLabel: t('添加'),
    submit: async () => {
      await api.projectSave(view.id, { name: name.value, path: path.value, description: description.value });
      onDone();
    },
  });
}

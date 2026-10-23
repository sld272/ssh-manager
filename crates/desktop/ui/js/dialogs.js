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

const splitTags = (text) => text.split(/[,，]/).map((tag) => tag.trim()).filter(Boolean);

// The add and edit forms share their fields; `view` is the host being edited.
function hostDialog(view, { title, subtitle, submitLabel, save }) {
  const c = view && defaultConnection(view);
  const a = c?.authentication;
  let auth = a?.kind ?? 'key';
  const name = h('input', { class: 'input', required: true, autofocus: !view, placeholder: 'blog', value: view?.name });
  const addr = h('input', { class: 'input', required: true, placeholder: t('example.com 或 IP 地址'), value: c?.address });
  const user = h('input', { class: 'input', required: true, placeholder: 'deploy', value: c?.user });
  const port = h('input', { class: 'input', type: 'number', min: 1, max: 65535, required: true, value: c?.port ?? 22 });
  // Imported hosts can list several key files; they get one line each.
  const paths = a?.kind === 'key' ? a.paths : ['~/.ssh/id_ed25519'];
  const keyPath = paths.length > 1
    ? h('textarea', { class: 'textarea mono', rows: paths.length, spellcheck: 'false' })
    : h('input', { class: 'input mono', placeholder: '~/.ssh/id_ed25519', spellcheck: 'false' });
  keyPath.value = paths.join('\n');
  const keyField = field(t('密钥文件'), keyPath);
  const description = h('input', { class: 'input', placeholder: t('这台机器是做什么的'), value: view?.context.description });
  const tags = h('input', { class: 'input', placeholder: t('用逗号分隔，例如 personal, lab'), value: view?.tags.join(', ') });
  const choices = [['key', t('密钥')], ['agent', 'ssh-agent'], ['password', t('密码')]];
  const seg = h('div', { class: 'segmented full', role: 'group', 'aria-label': t('登录方式') });
  const drawSeg = () => {
    replace(seg, choices.map(([v, label]) => h('button', {
      type: 'button', 'aria-pressed': String(auth === v),
      onclick: () => { auth = v; drawSeg(); },
    }, label)));
    keyField.hidden = auth !== 'key';
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
      field(t('说明'), description),
      field(t('标签'), tags)),
    submit: () => save({
      name: name.value, address: addr.value, user: user.value, port: Number(port.value), auth,
      keyPaths: auth === 'key' ? keyPath.value.split('\n').map((p) => p.trim()).filter(Boolean) : [],
      description: description.value, tags: splitTags(tags.value),
    }),
  });
}

export function addHostDialog(onDone) {
  hostDialog(null, {
    title: t('添加主机'),
    subtitle: t('使用密码登录时，添加后再保存一次密码。'),
    submitLabel: t('添加'),
    save: async (input) => {
      const host = await api.hostAdd(input);
      toast(t('已添加 {name}', { name: host.name }));
      onDone(host);
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
      onDone();
    },
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

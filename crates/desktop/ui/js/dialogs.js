import { api, message } from './api.js';
import { h, icon, replace, toast } from './dom.js';
import { address, defaultConnection, keyAlgorithm } from './format.js';
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
    title: t(passphrase ? '保存 {name} 的密钥口令' : '保存 {name} 的登录密码', { name: view.name }),
    subtitle: t('保存在系统钥匙串中。agent 可以用它登录，但看不到它。'),
    iconName: 'key',
    fields: field(t(passphrase ? '密钥口令' : '登录密码'), input),
    submitLabel: t('保存'),
    submit: async () => {
      try {
        await api.credentialSet(view.id, purpose, input.value);
      } finally {
        input.value = '';
      }
      toast(t(passphrase ? '口令已保存' : '密码已保存'));
      onDone();
    },
  });
}

export function addHostDialog(onDone) {
  let auth = 'key';
  const name = h('input', { class: 'input', required: true, autofocus: true, placeholder: 'blog' });
  const addr = h('input', { class: 'input', required: true, placeholder: t('example.com 或 IP 地址') });
  const user = h('input', { class: 'input', required: true, placeholder: 'deploy' });
  const port = h('input', { class: 'input', type: 'number', min: 1, max: 65535, value: '22', required: true });
  const keyPath = h('input', { class: 'input', placeholder: '~/.ssh/id_ed25519', value: '~/.ssh/id_ed25519' });
  const keyField = field(t('密钥文件'), keyPath);
  const description = h('input', { class: 'input', placeholder: t('这台机器是做什么的') });
  const tags = h('input', { class: 'input', placeholder: t('用逗号分隔，例如 personal, lab') });
  const choices = [['key', t('密钥')], ['agent', 'ssh-agent'], ['password', t('密码')]];
  const seg = h('div', { class: 'segmented full', role: 'group', 'aria-label': t('登录方式') });
  const drawSeg = () => {
    replace(seg, choices.map(([value, label]) => h('button', {
      type: 'button', 'aria-pressed': String(auth === value),
      onclick: () => { auth = value; drawSeg(); },
    }, label)));
    keyField.hidden = auth !== 'key';
  };
  drawSeg();
  formDialog({
    title: t('添加主机'),
    subtitle: t('添加后还需要确认服务器身份；使用密码时再保存一次密码。'),
    iconName: 'server',
    fields: h('div', { class: 'stack', style: 'gap: 14px' },
      h('div', { class: 'field-row' }, field(t('名称'), name), field(t('登录用户'), user)),
      h('div', { class: 'field-row port' }, field(t('地址'), addr), field(t('端口'), port)),
      h('div', { class: 'field' }, h('span', null, t('登录方式')), seg),
      keyField,
      field(t('说明'), description),
      field(t('标签'), tags)),
    submitLabel: t('添加'),
    submit: async () => {
      const host = await api.hostAdd({
        name: name.value, address: addr.value, user: user.value, port: Number(port.value),
        auth, keyPath: auth === 'key' ? keyPath.value : null,
        description: description.value,
        tags: tags.value.split(/[,，]/).map((tag) => tag.trim()).filter(Boolean),
      });
      toast(t('已添加 {name}', { name: host.name }));
      onDone(host);
    },
  });
}

export function editHostDialog(view, onDone) {
  const description = h('textarea', { class: 'textarea' });
  description.value = view.context.description;
  const tags = h('input', { class: 'input', value: view.tags.join(', '), placeholder: t('用逗号分隔') });
  formDialog({
    title: t('编辑 {name}', { name: view.name }),
    iconName: 'server',
    fields: h('div', { class: 'stack', style: 'gap: 14px' }, field(t('说明'), description), field(t('标签'), tags)),
    submitLabel: t('保存'),
    submit: async () => {
      await api.hostUpdate(view.id, {
        description: description.value,
        tags: tags.value.split(/[,，]/).map((tag) => tag.trim()).filter(Boolean),
      });
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

export function trustDialog(view, onDone) {
  const c = defaultConnection(view);
  const replaceExisting = view.trusted_keys.length > 0;
  const behindJump = c.jumps.length > 0;
  let mode = behindJump ? 'import' : 'scan';
  let keys = null;
  let loading = false;
  let error = null;

  const seg = h('div', { class: 'segmented full', role: 'group', 'aria-label': t('来源') });
  const keysBox = h('div', { class: 'stack', style: 'gap: 8px' });
  const fingerprint = h('input', { class: 'input mono', placeholder: 'SHA256:…', autocomplete: 'off', spellcheck: 'false', oninput: () => draw() });
  const fingerprintField = h('label', { class: 'field' }, h('span', null, t('你从可信来源获得的指纹')), fingerprint);
  const matchLine = h('p');
  const errorLine = h('p', { class: 'error-text', hidden: true });
  const note = h('p', { class: 'small muted' });
  const submit = h('button', { type: 'button', class: 'btn btn-primary' });
  let el;

  const normalized = () => {
    const v = fingerprint.value.trim();
    return v && !v.startsWith('SHA256:') ? `SHA256:${v}` : v;
  };
  const match = () => keys?.find((k) => k.fingerprint === normalized());

  function scan() {
    loading = true;
    error = null;
    draw();
    api.trustScan(view.id)
      .then((k) => { keys = k; })
      .catch((e) => { error = message(e); })
      .finally(() => { loading = false; draw(); });
  }

  function draw() {
    replace(seg,
      h('button', { type: 'button', 'aria-pressed': String(mode === 'scan'), disabled: behindJump, onclick: () => { mode = 'scan'; if (!keys) scan(); draw(); } }, t('扫描服务器')),
      h('button', { type: 'button', 'aria-pressed': String(mode === 'import'), onclick: () => { mode = 'import'; error = null; draw(); } }, t('从 known_hosts 导入')));
    fingerprintField.hidden = mode !== 'scan';
    matchLine.hidden = mode !== 'scan';
    if (mode === 'scan') {
      const found = match();
      replace(keysBox,
        h('div', { class: 'section-title' }, t('服务器提供的公钥')),
        loading ? h('p', { class: 'muted' }, t('正在读取服务器公钥…'))
          : (keys ?? []).map((k) => h('div', { class: `key-option${found === k ? ' selected' : ''}` },
            h('span', { class: 'key-alg' }, keyAlgorithm(k.algorithm)),
            h('span', { class: 'mono grow break small' }, k.fingerprint))));
      const typed = normalized();
      matchLine.className = found ? 'ok-text' : 'error-text';
      replace(matchLine, typed && keys ? (found ? [icon('check', 14, 2.2), t('与服务器公钥一致')] : t('与服务器提供的公钥都不一致，请勿信任')) : '');
      note.textContent = t('扫描结果本身不能证明服务器身份。以后公钥发生变化时，连接会停止，等你核实。');
      submit.textContent = t('信任此公钥');
      submit.disabled = !found || loading;
    } else {
      replace(keysBox, h('p', null, t('从 {file} 导入 {address} 已有的信任记录。', { file: '~/.ssh/known_hosts', address: `${c.address}${c.port === 22 ? '' : `:${c.port}`}` })));
      note.textContent = t(behindJump ? '经跳板连接的主机无法直接扫描，请从 known_hosts 导入。' : '适合你之前用 ssh 连接过这台服务器的情况。');
      submit.textContent = t('导入');
      submit.disabled = false;
    }
    errorLine.hidden = !error;
    errorLine.textContent = error ?? '';
  }

  submit.addEventListener('click', async () => {
    submit.disabled = true;
    error = null;
    try {
      if (mode === 'scan') {
        await api.trustAccept(view.id, match().fingerprint, replaceExisting);
      } else {
        await api.trustImport(view.id, replaceExisting);
      }
      toast(t('已信任 {name} 的服务器身份', { name: view.name }));
      el.close();
      onDone();
    } catch (e) {
      error = message(e);
      draw();
    }
  });

  el = open(h('div', { class: 'dialog' },
    head(t('确认 {name} 的服务器身份', { name: view.name }), t('连接 {address}。请在云服务商控制台等可信来源核对指纹，一致后再信任。', { address: address(c) }), 'warn', 'shield'),
    replaceExisting && h('div', { class: 'banner' }, t('已经保存过这台服务器的公钥。只有在确认服务器确实更换了密钥时才更新。')),
    seg, keysBox, fingerprintField, matchLine, note, errorLine,
    h('div', { class: 'dialog-actions' },
      h('button', { type: 'button', class: 'btn', onclick: () => el.close() }, t('取消')),
      submit)));
  draw();
  if (mode === 'scan') scan();
}

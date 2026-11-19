'use strict';

const $ = (id) => document.getElementById(id);
const all = (selector) => [...document.querySelectorAll(selector)];
const rows = [
  { id: 'review', kind: 'text', group: 'work', title: '发布前检查清单', stamp: '14:36',
    from: 'notepad.exe', pinned: true, summary: '检查下载地址与安装包校验值',
    content: '检查下载地址与安装包校验值。\n用全新配置启动，确认首次引导与设置恢复。\n整理浅色和深色截图，核对预览区文案。' },
  { id: 'cover', kind: 'image', group: 'assets', title: '海湾配色参考.png', stamp: '14:21',
    from: 'photos.exe', summary: '1,600 × 1,000 · PNG', content: '1,600 × 1,000 · PNG' },
  { id: 'files', kind: 'file', group: 'work', title: '发布记录.pdf', stamp: '14:04',
    from: 'explorer.exe', summary: '1 个文件 · PDF', content: '发布记录.pdf\n附件 / 开发记录 / 发布记录.pdf' },
  { id: 'link', kind: 'link', group: 'assets', title: 'Fluent 2 Design System', stamp: '13:57',
    from: 'msedge.exe', summary: 'fluent2.microsoft.design', content: 'https://fluent2.microsoft.design/' },
  { id: 'private', kind: 'text', group: 'none', title: '••••••••••••', stamp: '13:42',
    from: 'notepad.exe', summary: '敏感内容 · 无计时', content: '原文已隐藏。', sensitive: true },
  { id: 'missing', kind: 'file', group: 'none', title: '旧版交付.zip', stamp: '13:16',
    from: 'explorer.exe', summary: '源文件失效', content: '原文件已移动或删除。', missing: true },
];
const kindLabel = { text: '文本', image: '图片', file: '文件', link: '链接' };
const kindSymbol = { text: '≡', image: '▧', file: '▤', link: '↗' };
const notes = {
  history: '单列、双列和紧凑布局共用筛选与选中状态。',
  settings: '展开明暗或常规布局，查看选项、焦点与设置草稿。',
  quick: '用箭头或滚轮切换示例。聚焦滚轮后可按空格或右键取消。',
  floating: '选择左侧记录，查看只读浮窗中的文字、图片、敏感和失效状态。',
};
const categories = { general: '通用', appearance: '外观', history: '历史记录', privacy: '隐私', keys: '快捷键', storage: '数据存储' };
const appearanceSections = [
  ['主题', [
    ['skin', '皮肤', '主面板、快速面板和浮窗共用。', [['fluent', 'Fluent']]],
    ['theme', '明暗', '与布局分别设置。', [['light', '浅色'], ['dark', '深色']]],
  ]],
  ['布局', [
    ['layout', '常规布局', '退出紧凑模式后恢复此布局。', [['list', '单列列表'], ['cards', '双列卡片']]],
    ['compact', '紧凑模式', '隐藏分组侧栏和内嵌预览。', 'checkbox'],
  ]],
];
const settingIcons = {
  skin: 'M4 5h16v14H4z M4 9h16 M8 13h4 M8 16h7',
  theme: 'M19 15a7 7 0 0 1-10-9 7.5 7.5 0 1 0 10 9Z M17 5v5 M14.5 7.5h5',
  layout: 'M3 5h18v14H3z M9 5v14 M12 10h6 M12 14h6',
  compact: 'M7 4h10v16H7z M10 8h4 M10 12h4 M10 16h4',
  startup: 'M12 4v7 M8 6a7 7 0 1 0 8 0',
  limit: 'M5 6h14 M5 12h14 M5 18h14',
  sensitive: 'M5 10h14v10H5z M8 10V8a4 4 0 0 1 8 0v2 M12 14v3',
  count: 'M4 7h16v10H4z M6 4h12 M6 20h12',
  path: 'M3 7h7l2 2h9v10H3z',
};
const otherFields = {
  general: ['startup', '开机时启动', '登录 Windows 后在托盘运行。', 'checkbox'],
  history: ['limit', '最大条数', '控制保留的历史记录数量。', 'number'],
  privacy: ['sensitive', '检测敏感内容', '预览默认隐藏敏感原文。', 'checkbox'],
  keys: ['count', '快速面板可见条数', '控制每次呼出时显示的条目数量。', 'number'],
  storage: ['path', '数据目录', '当前目录用于保存历史与设置。', 'text'],
};
let saved = { skin: 'fluent', theme: 'light', layout: 'list', compact: false, startup: false,
  limit: 1000, sensitive: true, count: 5, path: '%LOCALAPPDATA%\\ClipRaft' };
let draft = { ...saved };
let activeView = 'history';
let activeGroup = 'all';
let activeKind = 'all';
let activeCategory = 'appearance';
let selectedId = rows[0].id;
let regularLayout = 'list';
let quickIndex = 1;
let quickOpen = true;

function selectedRow() {
  return rows.find((row) => row.id === selectedId);
}

function filteredRows() {
  const query = $('query').value.trim().toLowerCase();
  return rows.filter((row) =>
    (activeGroup === 'all' || row.group === activeGroup) &&
    (activeKind === 'all' || row.kind === activeKind) &&
    [row.title, row.summary, row.content].some((value) => value.toLowerCase().includes(query)),
  );
}

function recordMarkup(row, selection) {
  return `<button class="record" type="button" data-record="${row.id}" aria-pressed="${row.id === selection}">
    <span class="symbol" aria-hidden="true">${kindSymbol[row.kind]}</span>
    <span class="record-copy"><strong>${row.title}</strong>
      <small>${row.pinned ? '置顶 · ' : ''}${row.summary}</small></span>
    <time>${row.stamp}</time>
  </button>`;
}

function detailMarkup(row, showTitle = true) {
  const title = showTitle ? `<h3>${row.title}</h3>` : '';
  const labels = { private: '敏感内容', missing: '源文件失效' };
  const label = labels[row.id];
  const picture = row.kind === 'image' ? '<div class="sample-picture" role="img" aria-label="合成海湾配色参考"></div>' : '';
  return `${title}${label ? `<div><span class="status-chip">${label}</span></div>` : ''}
    ${picture}<p>${row.content}</p>`;
}

function renderHistory() {
  const list = filteredRows();
  if (!list.some((row) => row.id === selectedId)) selectedId = list[0]?.id;
  $('total').textContent = `${list.length} 条`;
  $('records').innerHTML = list.map((row) => recordMarkup(row, selectedId)).join('') || '<p class="empty-state">未找到匹配的记录</p>';
  const selected = selectedRow();
  $('detail-content').innerHTML = selected ? detailMarkup(selected) : '<p>请选择记录</p>';
  $('source').textContent = selected ? `来源：${selected.from}` : '';
  $('paste').disabled = !selected || selected.missing;
  $('menu-trigger').disabled = !selected;
  const openLabel = { file: '打开文件', link: '打开链接' }[selected?.kind];
  $('open-record').textContent = openLabel ?? '';
  $('open-record').hidden = !openLabel;
  $('open-record').disabled = !!selected?.missing;
  toggleMenu(false);
}

function renderFloating() {
  const selected = selectedRow() || rows[0];
  $('floating-records').innerHTML = rows.map((row) => recordMarkup(row, selected.id)).join('');
  $('float-title').textContent = selected.title;
  $('float-content').innerHTML = detailMarkup(selected, false);
  $('float-source').textContent = `来源：${selected.from}`;
  positionBeak();
}

function positionBeak() {
  if (activeView !== 'floating') return;
  const record = $('floating-records').querySelector('[aria-pressed="true"]');
  if (!record) return;
  const frame = document.querySelector('.floating-card');
  const source = record.getBoundingClientRect();
  const target = frame.getBoundingClientRect();
  const padding = 14;
  const offset = source.top + source.height / 2 - target.top;
  frame.style.setProperty('--beak-offset', `${Math.min(target.height - padding, Math.max(padding, offset))}px`);
}

function renderWheel() {
  $('quick-stage').hidden = !quickOpen;
  $('reopen').hidden = quickOpen;
  $('previous').hidden = !quickOpen;
  $('next').hidden = !quickOpen;
  const length = rows.length;
  $('wheel-count').textContent = `${quickIndex + 1} / ${length} 条`;
  $('wheel').innerHTML = Array.from({ length: Math.min(5, length) }, (_, index) => {
    const offset = index - Math.min(2, Math.floor(length / 2));
    const row = rows[(quickIndex + offset + length) % length];
    const distance = Math.abs(offset);
    const transform = `translateY(${offset * 76}px) translateZ(${-distance * 40}px) rotateX(${-offset * 18}deg) scale(${1 - distance * 0.06})`;
    return `<div class="wheel-card${offset === 0 ? ' selected' : ''}" style="transform:${transform};z-index:${20 - distance}">
      <strong>${row.title}</strong><small>${kindLabel[row.kind]} · ${row.summary}</small></div>`;
  }).join('');
}

function moveWheel(direction) {
  quickIndex = (quickIndex + direction + rows.length) % rows.length;
  renderWheel();
}

function updateLayout() {
  const layout = $('layout').value;
  const showingHistory = activeView === 'history';
  if (layout !== 'compact') regularLayout = layout;
  $('main-window').dataset.layout = showingHistory ? layout : 'list';
  $('compact-settings').hidden = layout !== 'compact';
  $('compact-toggle').hidden = !showingHistory;
  $('compact-toggle').setAttribute('aria-pressed', layout === 'compact');
  $('layout').disabled = !showingHistory;
}

function showView(view) {
  if (view === 'settings' && activeView !== 'settings') {
    draft = { ...saved };
    selectCategory('appearance');
  }
  activeView = view;
  $('main-window').hidden = view !== 'history' && view !== 'settings';
  $('history').hidden = view !== 'history';
  $('settings').hidden = view !== 'settings';
  $('quick').hidden = view !== 'quick';
  $('floating').hidden = view !== 'floating';
  all('[data-view]').forEach((button) => button.setAttribute('aria-pressed', button.dataset.view === view));
  $('scene-note').textContent = notes[view];
  updateLayout();
  const draw = { history: renderHistory, quick: renderWheel, floating: renderFloating };
  draw[view]?.();
}

function fieldMarkup([name, title, description, type]) {
  const attributes = `id="setting-${name}" name="${name}" aria-describedby="description-${name}"`;
  const control = Array.isArray(type)
    ? `<select class="fluent-select" ${attributes}>${type.map(([value, label]) => `<option value="${value}" ${draft[name] === value ? 'selected' : ''}>${label}</option>`).join('')}</select>`
    : `<input ${attributes} type="${type}" class="${type === 'checkbox' ? 'switch' : 'field-input'}"${draft[name] === true ? ' checked' : ''}${type === 'text' ? ' readonly' : ''}>`;
  return `<div class="settings-field">
    <svg class="settings-icon" viewBox="0 0 24 24" focusable="false" aria-hidden="true"><path d="${settingIcons[name]}" /></svg>
    <div class="settings-field-copy">
      <label for="setting-${name}">${title}</label>
      <small id="description-${name}">${description}</small>
    </div>
    ${control}
  </div>`;
}

function settingsMarkup(category) {
  if (category !== 'appearance') return fieldMarkup(otherFields[category]);
  return appearanceSections.map(([title, fields]) => `<fieldset class="settings-section">
    <legend>${title}</legend>${fields.map(fieldMarkup).join('')}
  </fieldset>`).join('');
}

function selectCategory(category) {
  activeCategory = category;
  all('[data-setting]').forEach((button) => button.setAttribute('aria-pressed', button.dataset.setting === category));
  $('settings-title').textContent = categories[category];
  $('settings-fields').innerHTML = settingsMarkup(category);
  $('settings-fields').querySelectorAll('.field-input').forEach((input) => { input.value = draft[input.name]; });
  updateSaveState();
}

function updateSaveState() {
  const changed = Object.keys(saved).some((name) => saved[name] !== draft[name]);
  $('settings-save').disabled = !changed;
  $('settings-status').textContent = changed ? '有未保存的更改' : '';
}

function setGroup(group) {
  activeGroup = group;
  $('compact-group').value = group;
  all('[data-group]').forEach((button) => button.setAttribute('aria-pressed', button.dataset.group === group));
  renderHistory();
}

function toggleMenu(open) {
  $('record-menu').hidden = !open;
  $('menu-trigger').setAttribute('aria-expanded', open);
}

const menuActions = {
  pin: () => {
    const row = selectedRow();
    row.pinned = !row.pinned;
    renderHistory();
  },
  copy: () => { $('status').textContent = '复制内容（演示）'; },
  delete: () => {
    $('delete-summary').textContent = selectedRow().title;
    $('delete-dialog').returnValue = '';
    $('delete-dialog').showModal();
  },
};

all('[data-view]').forEach((button) => button.addEventListener('click', () => showView(button.dataset.view)));
all('[data-setting]').forEach((button) => button.addEventListener('click', () => selectCategory(button.dataset.setting)));
all('[data-group]').forEach((button) => button.addEventListener('click', () => setGroup(button.dataset.group)));
all('[data-kind]').forEach((button) => button.addEventListener('click', () => {
  activeKind = button.dataset.kind;
  all('[data-kind]').forEach((type) => type.setAttribute('aria-pressed', type.dataset.kind === activeKind));
  renderHistory();
}));

$('tone').addEventListener('change', (event) => { document.documentElement.dataset.tone = event.target.value; });
$('layout').addEventListener('change', updateLayout);
$('material').addEventListener('change', (event) => { $('canvas').dataset.material = event.target.value; });
$('motion').addEventListener('change', (event) => { $('canvas').dataset.motion = event.target.value; });
$('query').addEventListener('input', renderHistory);
$('compact-group').addEventListener('change', (event) => setGroup(event.target.value));
$('compact-toggle').addEventListener('click', () => {
  $('layout').value = $('layout').value === 'compact' ? regularLayout : 'compact';
  updateLayout();
});
$('records').addEventListener('click', (event) => selectRecord(event, renderHistory));
$('floating-records').addEventListener('click', (event) => selectRecord(event, renderFloating));
function selectRecord(event, render) {
  const record = event.target.closest('[data-record]');
  if (!record) return;
  selectedId = record.dataset.record;
  render();
  event.currentTarget.querySelector(`[data-record="${selectedId}"]`)?.focus();
}
$('paste').addEventListener('click', () => { $('status').textContent = '粘贴此条记录（演示）'; });
$('open-record').addEventListener('click', () => {
  $('status').textContent = `${$('open-record').textContent}（演示）`;
});
$('records').addEventListener('keydown', (event) => {
  const visible = filteredRows();
  const index = visible.findIndex((row) => row.id === selectedId);
  const directions = { ArrowDown: 1, ArrowUp: -1 };
  if (event.key === 'Enter') {
    event.preventDefault();
    $('paste').click();
  }
  const direction = directions[event.key];
  if (!direction || index < 0) return;
  event.preventDefault();
  selectedId = visible[(index + direction + visible.length) % visible.length].id;
  renderHistory();
  $('records').querySelector(`[data-record="${selectedId}"]`)?.focus();
});
document.addEventListener('keydown', (event) => {
  if (activeView === 'history' && event.ctrlKey && event.key.toLowerCase() === 'f') {
    event.preventDefault();
    $('query').focus();
  }
});
window.addEventListener('resize', positionBeak);
$('menu-trigger').addEventListener('click', () => toggleMenu($('record-menu').hidden));
$('record-menu').addEventListener('click', (event) => {
  const action = event.target.closest('[data-action]')?.dataset.action;
  if (!action) return;
  toggleMenu(false);
  $('menu-trigger').focus();
  menuActions[action]();
});
$('delete-dialog').addEventListener('close', () => {
  if ($('delete-dialog').returnValue === 'delete') $('status').textContent = '删除记录（演示）';
});
for (const id of ['open-settings', 'compact-settings']) $(id).addEventListener('click', () => showView('settings'));
for (const id of ['back', 'settings-cancel']) $(id).addEventListener('click', () => showView('history'));
$('settings-fields').addEventListener('change', (event) => {
  const field = event.target;
  const fieldValues = { checkbox: field.checked, number: Number(field.value) };
  draft[field.name] = fieldValues[field.type] ?? field.value;
  updateSaveState();
});
$('settings-save').addEventListener('click', () => {
  saved = { ...draft };
  document.documentElement.dataset.tone = saved.theme;
  $('tone').value = saved.theme;
  $('layout').value = saved.compact ? 'compact' : saved.layout;
  regularLayout = saved.layout;
  selectCategory(activeCategory);
  $('settings-status').textContent = '已保存（演示）';
});
$('previous').addEventListener('click', () => moveWheel(-1));
$('next').addEventListener('click', () => moveWheel(1));
$('reopen').addEventListener('click', () => {
  quickOpen = true;
  quickIndex = 1;
  renderWheel();
  $('quick-stage').focus();
});
function closeWheel(event) {
  event.preventDefault();
  quickOpen = false;
  renderWheel();
  $('reopen').focus();
}
$('quick-stage').addEventListener('contextmenu', closeWheel);
$('quick-stage').addEventListener('keydown', (event) => {
  if (event.key === ' ') return closeWheel(event);
  const directions = { ArrowUp: -1, ArrowDown: 1 };
  const direction = directions[event.key];
  if (!direction) return;
  event.preventDefault();
  moveWheel(direction);
});
$('quick-stage').addEventListener('wheel', (event) => {
  if (event.deltaY === 0) return;
  event.preventDefault();
  moveWheel(Math.sign(event.deltaY));
}, { passive: false });

showView('history');

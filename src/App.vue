<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import PreviewImage from './PreviewImage.vue';
import HistoryList from './HistoryList.vue';
import SettingsView from './SettingsView.vue';
import type { Settings, SettingOptions } from './settings';
import ContextMenu from './ContextMenu.vue';
import GroupSidebar from './GroupSidebar.vue';
import { kindLabel, type Item, type HistoryFilter, type HistoryKind, type Group, type HistoryView } from './history';
interface Panel { items: Item[]; selected: number; visible: boolean; error: string | null }
const quick = new URLSearchParams(location.search).get('panel') === 'quick';
const items = ref<Item[]>([]), selected = ref(0), error = ref(''), busy = ref(false);
const settingsOpen = ref(false), settingsBusy = ref(false), startupError = ref('');
const settings = ref<Settings>(), settingOptions = ref<SettingOptions>();
const mainLabel = computed(() => settingOptions.value?.main_hotkeys.find(k => k.id === settings.value?.main_hotkey)?.label ?? '');
const quickKey = computed(() => settingOptions.value?.quick_hotkeys.find(k => k.id === settings.value?.quick_hotkey));
function toggleSettings() {
  if (settingsBusy.value || deleting.value || busy.value || organizing.value) return;
  conceal(); settingsOpen.value = !settingsOpen.value; contextMenu.value = null; pendingDelete.value = null;
  if (!settingsOpen.value) { void nextTick(checkLoadMore); void focusSearch(); }
}
const searchInput = ref<HTMLInputElement | null>(null);
const query = ref(''), kind = ref<HistoryKind | null>(null), composing = ref(false);
const kinds: { value: HistoryKind | null; label: string }[] = [
  { value: null, label: '全部' }, { value: 'text', label: '文本' }, { value: 'image', label: '图片' },
  { value: 'files', label: '文件' }, { value: 'link', label: '链接' },
];
const groups = ref<Group[]>([]), view = ref<HistoryView>('all'), groupError = ref('');
const mutationBusy = ref(false), groupInteracting = ref(false), groupMenuOpen = ref(false);
const organizing = computed(() => mutationBusy.value || groupInteracting.value);
const selectedItems = computed(() => items.value.filter(item => selectedIds.value.has(item.id)));
const allPinned = computed(() => selectedItems.value.length > 0 && selectedItems.value.every(item => item.pinned));
const allSensitive = computed(() => selectedItems.value.length > 0 && selectedItems.value.every(item => item.sensitive));
const allFavorite = computed(() => selectedItems.value.length > 0 && selectedItems.value.every(item => item.favorite));
const protectedDeleteCount = computed(() => items.value.filter(item => pendingDelete.value?.includes(item.id) && (item.pinned || item.favorite)).length);
const listFilter = computed<HistoryFilter>(() => ({ query: query.value, kind: kind.value,
  ...(view.value === 'favorites' ? { favorite_only: true } : {}),
  ...(view.value === 'ungrouped' ? { group_id: -1 } : typeof view.value === 'number' ? { group_id: view.value } : {}),
}));
let groupGeneration = 0;
async function loadGroups() {
  const generation = ++groupGeneration;
  try {
    const result = await invoke<Group[]>('list_groups');
    if (generation !== groupGeneration) return;
    groups.value = result; groupError.value = '';
    if (typeof view.value === 'number' && !result.some(group => group.id === view.value)) view.value = 'ungrouped';
  } catch (e) { if (generation === groupGeneration) groupError.value = '分组读取失败：' + String(e); }
}
function groupsChanged() { void loadGroups(); void load(); }
async function changeItems(command: 'set_pinned' | 'set_favorite' | 'move_to_group' | 'set_sensitive', groupId?: number | null) {
  if (quick || organizing.value || deleting.value || busy.value || pendingDelete.value || !selectedItems.value.length) return;
  const ids = selectedItems.value.map(item => item.id);
  const args = command === 'move_to_group' ? { ids, groupId } : { ids, value: command === 'set_pinned' ? !allPinned.value : command === 'set_sensitive' ? !allSensitive.value : !allFavorite.value };
  mutationBusy.value = true; contextMenu.value = null; error.value = '';
  ++listGeneration; ++detailGeneration;
  try { await invoke(command, args); }
  catch (e) { error.value = String(e); }
  finally {
    // 即使刷新事件失败，仍重新读库，避免把已提交的修改显示为未保存。
    deferredHistory = false;
    await Promise.all([loadGroups(), load()]);
    mutationBusy.value = false;
    if (deferredHistory) { deferredHistory = false; historyChanged(); }
    void nextTick(checkLoadMore);
  }
}
const filtered = computed(() => !!query.value.trim() || kind.value !== null || view.value !== 'all');
let searchTimer: ReturnType<typeof setTimeout> | undefined;
async function focusSearch() { await nextTick(); if (!quick && !settingsOpen.value) searchInput.value?.focus(); }
function filtersChanged(delay = 150) {
  if (quick) return;
  clearTimeout(searchTimer); searchTimer = undefined;
  ++listGeneration; ++detailGeneration;
  afterDelete = undefined; contextMenu.value = null; pendingDelete.value = null;
  items.value = []; selectOnly(0); nextOffset = 0; hasMore.value = true;
  pageError.value = ''; pageLoading.value = true;
  if (listRoot.value) listRoot.value.scrollTop = 0;
  if (!composing.value) searchTimer = setTimeout(() => { searchTimer = undefined; void load(); }, delay);
}
watch(query, () => filtersChanged());
watch(kind, () => filtersChanged(0));
watch(view, () => filtersChanged(0));
function compositionStart() { composing.value = true; clearTimeout(searchTimer); searchTimer = undefined; }
function compositionEnd() { composing.value = false; filtersChanged(); }
const PAGE_SIZE = 100;
const historyList = ref<InstanceType<typeof HistoryList> | null>(null);
const listRoot = computed(() => historyList.value?.element ?? null);
const pageLoading = ref(false), pageError = ref(''), hasMore = ref(true);
let listGeneration = 0, nextOffset = 0, retryRefresh = false;
const selectedIds = ref(new Set<string>()), pendingDelete = ref<string[] | null>(null), deleting = ref(false);
const contextMenu = ref<{ x: number; y: number; id: string } | null>(null);
const menuInteracting = computed(() => groupMenuOpen.value || contextMenu.value !== null);
let anchorId: string | undefined, deferredHistory = false;
let afterDelete: { index: number; id?: string; count: number } | undefined;
function selectOnly(index: number) {
  selected.value = index;
  anchorId = items.value[index]?.id;
  selectedIds.value = new Set(anchorId ? [anchorId] : []);
}
function choose(index: number, event: MouseEvent) {
  if (quick || deleting.value || organizing.value || pendingDelete.value) return;
  contextMenu.value = null;
  const id = items.value[index]?.id;
  if (!id) return;
  if (event.shiftKey) {
    const anchor = Math.max(0, items.value.findIndex(item => item.id === anchorId));
    const range = items.value.slice(Math.min(anchor, index), Math.max(anchor, index) + 1).map(item => item.id);
    selectedIds.value = new Set(event.ctrlKey ? [...selectedIds.value, ...range] : range);
    selected.value = index;
  } else if (event.ctrlKey) {
    const ids = new Set(selectedIds.value);
    if (ids.has(id)) ids.delete(id); else ids.add(id);
    selectedIds.value = ids; selected.value = index; anchorId = id;
  } else selectOnly(index);
}
function openContext(index: number, event: MouseEvent) {
  if (quick || deleting.value || organizing.value || pendingDelete.value) return;
  const id = items.value[index]?.id;
  if (!id) return;
  if (!selectedIds.value.has(id)) selectOnly(index); else selected.value = index;
  contextMenu.value = { x: event.clientX, y: event.clientY, id };
}
function requestDelete() {
  if (quick || deleting.value || organizing.value || busy.value || pendingDelete.value) return;
  contextMenu.value = null;
  const ids = items.value.filter(item => selectedIds.value.has(item.id)).map(item => item.id);
  if (ids.length > 1) pendingDelete.value = ids;
  else if (ids.length === 1) void performDelete(ids);
}
interface DeleteReport { deleted_items: number; reclaimed_blobs: number; failed_blobs: string[]; maintenance_errors: string[] }
async function performDelete(ids: string[]) {
  if (deleting.value || busy.value || organizing.value) return;
  const removed = new Set(ids), index = items.value.findIndex(item => removed.has(item.id));
  const next = items.value.slice(Math.max(0, index)).find(item => !removed.has(item.id));
  const position = { index: Math.max(0, index), id: next?.id, count: items.value.length };
  deleting.value = true; pendingDelete.value = null; contextMenu.value = null; error.value = '';
  ++listGeneration; ++detailGeneration;
  try {
    const report = await invoke<DeleteReport>('delete_items', { ids });
    afterDelete = position;
    if (report.failed_blobs.length || report.maintenance_errors.length) error.value = '记录已删除，部分维护未完成，将在下次启动重试。' + report.maintenance_errors.join('；');
    deferredHistory = false;
    await load();
  } catch (e) { error.value = String(e); pageLoading.value = false; void loadDetail(); }
  finally {
    deleting.value = false;
    if (deferredHistory) { deferredHistory = false; void load(); }
  }
}
function confirmDelete() { if (pendingDelete.value) void performDelete(pendingDelete.value); }
function historyChanged() {
  contextMenu.value = null;
  void loadGroups();
  if (deleting.value || mutationBusy.value) deferredHistory = true; else if (!searchTimer && !composing.value) void load();
}
function contextPaste() { contextMenu.value = null; void paste(false); }

function checkLoadMore() {
  const list = listRoot.value;
  if (!quick && list && list.scrollHeight - list.scrollTop - list.clientHeight < 120) void loadMore();
}
async function loadMore() {
  if (quick || deleting.value || mutationBusy.value || pageLoading.value || pageError.value || !hasMore.value) return;
  const generation = listGeneration, offset = nextOffset;
  pageLoading.value = true;
  try {
    const page = await invoke<Item[]>('list_items', { filter: listFilter.value, offset, limit: PAGE_SIZE });
    if (generation !== listGeneration) return;
    const known = new Set(items.value.map(item => item.id));
    items.value.push(...page.filter(item => !known.has(item.id)));
    nextOffset = offset + page.length;
    hasMore.value = page.length === PAGE_SIZE;
  } catch (e) {
    if (generation === listGeneration) { pageError.value = '加载下一页失败：' + String(e); retryRefresh = false; }
  } finally {
    if (generation === listGeneration) {
      pageLoading.value = false;
      await nextTick(); checkLoadMore();
    }
  }
}
function retryPage() { pageError.value = ''; void (retryRefresh ? load() : loadMore()); }
interface Detail { text: string; truncated: boolean; files: { path: string; exists: boolean | null }[] }
const detail = ref<Detail | null>(null), detailError = ref(''), detailLoading = ref(false);
const revealed = ref(false);
let detailGeneration = 0;
function conceal() { ++detailGeneration; revealed.value = false; detail.value = null; detailLoading.value = false; }
async function revealDetail() {
  if (quick || !active.value?.sensitive || detailLoading.value) return;
  const id = active.value.id, generation = ++detailGeneration;
  detailLoading.value = true; detailError.value = '';
  try {
    const result = await invoke<Detail>('reveal_item', { id });
    if (generation === detailGeneration && active.value?.id === id) { detail.value = result; revealed.value = true; }
  } catch { if (generation === detailGeneration) detailError.value = '显示原文失败'; }
  finally { if (generation === detailGeneration) detailLoading.value = false; }
}
function visibilityChanged() { if (document.hidden) conceal(); else void loadDetail(); }

async function loadDetail() {
  if (quick) return;
  const id = active.value?.id, generation = ++detailGeneration;
  revealed.value = false; detail.value = null; detailError.value = ''; detailLoading.value = !!id;
  if (!id || active.value?.kind === 'image') { detailLoading.value = false; return; }
  try {
    const result = await invoke<Detail>('item_detail', { id });
    if (generation === detailGeneration) detail.value = result;
  } catch (e) { if (generation === detailGeneration) detailError.value = String(e); }
  finally { if (generation === detailGeneration) detailLoading.value = false; }
}
const panelVisible = ref(false);
const noticeOnly = computed(() => quick && !panelVisible.value && !!error.value);
const panelRoot = ref<HTMLElement | null>(null);
let panelObserver: ResizeObserver | undefined;
let lastPanelSize = '';
function resizeQuick() {
  if (!quick || !panelRoot.value) return;
  const notice = noticeOnly.value;
  const height = notice ? Math.ceil(panelRoot.value.getBoundingClientRect().height) : 530;
  const size = String(notice) + ':' + height;
  if (size === lastPanelSize) return;
  lastPanelSize = size;
  // 使用实际换行后的内容高度；原生窗口调整保持不激活，不重新显示已关闭提示。
  void invoke('resize_quick', { height, notice }).catch(() => { lastPanelSize = ''; });
}
watch([noticeOnly, error, panelVisible], resizeQuick, { flush: 'post' });
function applyPanel(p: Panel) { items.value = p.items; selected.value = p.selected; panelVisible.value = p.visible; error.value = p.error ?? ''; }
const stops: UnlistenFn[] = [];
const active = computed(() => items.value[selected.value]);
watch(() => active.value?.id, () => { void loadDetail(); }, { flush: 'sync' });
async function load() {
  if (quick) { const p = await invoke<Panel>('panel_state'); applyPanel(p); return; }
  conceal();
  const generation = ++listGeneration, targetCount = Math.max(PAGE_SIZE, afterDelete?.count ?? items.value.length);
  const deletion = afterDelete;
  const scrollTop = listRoot.value?.scrollTop ?? 0;
  pageLoading.value = true; pageError.value = '';
  try {
    const refreshed: Item[] = [];
    let more = true;
    // 重建已加载范围；选中项移到下一页时继续查找，保留选择。
    while (more) {
      const page = await invoke<Item[]>('list_items', { filter: listFilter.value, offset: refreshed.length, limit: PAGE_SIZE });
      if (generation !== listGeneration) return;
      refreshed.push(...page); more = page.length === PAGE_SIZE;
      const id = deletion?.id ?? active.value?.id;
      if (refreshed.length >= targetCount && (deletion || !id || refreshed.some(item => item.id === id))) break;
    }
    const id = deletion ? deletion.id : active.value?.id;
    const oldIndex = deletion?.index ?? selected.value;
    const previousSelection = selectedIds.value;
    const unique = new Map(refreshed.map(item => [item.id, item]));
    items.value = [...unique.values()];
    const found = items.value.findIndex(item => item.id === id);
    selected.value = found >= 0 ? found : Math.max(0, Math.min(oldIndex, items.value.length - 1));
    if (deletion || !id || found < 0) selectOnly(selected.value);
    else selectedIds.value = new Set(items.value.filter(item => previousSelection.has(item.id)).map(item => item.id));
    if (afterDelete === deletion) afterDelete = undefined;
    if (!items.value.some(item => item.id === anchorId)) anchorId = active.value?.id;
    nextOffset = refreshed.length; hasMore.value = more;
    if (id === active.value?.id) void loadDetail();
    await nextTick();
    if (generation === listGeneration && listRoot.value) listRoot.value.scrollTop = scrollTop;
  } catch (e) {
    if (generation === listGeneration) { pageError.value = '刷新历史失败：' + String(e); retryRefresh = true; }
  } finally {
    if (generation === listGeneration) {
      pageLoading.value = false;
      await nextTick(); checkLoadMore();
    }
  }
}
async function paste(plain = false) {
  if (quick || !active.value || busy.value || deleting.value || organizing.value || pendingDelete.value) return;
  busy.value = true; error.value = '';
  try { await invoke('paste_item', { id: active.value.id, plain }); }
  catch (e) { error.value = String(e); }
  finally { busy.value = false; }
}
async function keydown(event: KeyboardEvent) {
  if (quick || event.isComposing || composing.value || event.keyCode === 229) return;
  if (menuInteracting.value) {
    if (event.key === 'Escape') { event.preventDefault(); contextMenu.value = null; }
    return;
  }
  const recordShortcut = event.ctrlKey && !event.altKey && !event.metaKey && !event.shiftKey && ['p', 'd'].includes(event.key.toLowerCase());
  if (settingsOpen.value) {
    if (recordShortcut) event.preventDefault();
    if (event.key === 'Escape') { event.preventDefault(); if (!settingsBusy.value) toggleSettings(); }
    return;
  }
  const target = event.target instanceof Element ? event.target : document.activeElement;
  const otherInput = target !== searchInput.value && !!target?.closest('input, textarea, select, [contenteditable]');
  if (otherInput) return;
  if (recordShortcut) {
    event.preventDefault();
    if (!event.repeat && !organizing.value && !pendingDelete.value) void changeItems(event.key.toLowerCase() === 'p' ? 'set_pinned' : 'set_favorite');
    return;
  }
  if (organizing.value) return;
  // 保留控件的原生激活/编辑行为；列表项本身也是 button，不接管其 Enter。
  if (event.key === ' ') return;
  if (event.key === 'Enter') {
    const target = event.target instanceof Element ? event.target : document.activeElement;
    if (target?.closest('button, select, a, textarea, input:not([type="search"]), [contenteditable]')) return;
    const inSearch = target === searchInput.value;
    const inList = !!target?.closest('.list');
    const noFocus = !target || target === document.body || target === document.documentElement;
    if (!inSearch && !inList && !noFocus) return;
  }
  if (pendingDelete.value) {
    if (event.key === 'Escape') { event.preventDefault(); pendingDelete.value = null; }
    // 确认条按钮仍可通过 Tab / Enter 操作，但不触发列表粘贴。
    return;
  }
  if (deleting.value) { event.preventDefault(); return; }
  const inSearch = event.target === searchInput.value;
  if (event.key === 'Escape' && contextMenu.value) { event.preventDefault(); contextMenu.value = null; return; }
  // 搜索框非空时保留原生 Delete，光标在末尾也不会误删历史。
  if (event.key === 'Delete') { if (inSearch && query.value.length) return; event.preventDefault(); requestDelete(); return; }
  if (event.ctrlKey && event.key.toLowerCase() === 'a') {
    if (inSearch) return;
    event.preventDefault(); selectedIds.value = new Set(items.value.map(item => item.id)); contextMenu.value = null; return;
  }
  if (event.key === 'Enter' && contextMenu.value) return;
  if (event.key === 'Enter') { event.preventDefault(); void paste(event.shiftKey); }
  else if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
    event.preventDefault();
    if (event.key === 'ArrowDown' && selected.value === items.value.length - 1 && hasMore.value) {
      if (pageLoading.value || pageError.value) return;
      const generation = listGeneration;
      await loadMore();
      if (generation !== listGeneration) return;
      if (selected.value === items.value.length - 1 && hasMore.value) return;
    }
    if (items.value.length) selectOnly((selected.value + (event.key === 'ArrowDown' ? 1 : -1) + items.value.length) % items.value.length);
    contextMenu.value = null;
    void nextTick(() => document.querySelector('.item.selected')?.scrollIntoView({ block: 'nearest' }));
  } else if (event.key === 'Escape') {
    event.preventDefault();
    if (query.value.length) { query.value = ''; void focusSearch(); } else { conceal(); void invoke('hide_main'); }
  }
}
onMounted(async () => {
  if (quick && panelRoot.value) { panelObserver = new ResizeObserver(resizeQuick); panelObserver.observe(panelRoot.value); }
  try {
    stops.push(await listen<Panel>('panel-state', ({ payload }) => { if (quick) applyPanel(payload); }));
    stops.push(await listen('main-shown', () => { void loadDetail(); void focusSearch(); }));
    stops.push(await listen('main-hidden', conceal));
    stops.push(await listen('history-changed', () => { if (!quick) historyChanged(); }));
    stops.push(await listen<string>('operation-error', ({ payload }) => { if (!quick) error.value = payload; }));
    stops.push(await listen<Settings>('settings-changed', ({ payload }) => { settings.value = payload; }));
    stops.push(await listen<string>('startup-error', ({ payload }) => { startupError.value = payload; }));
    [settings.value, settingOptions.value] = await Promise.all([invoke<Settings>('get_settings'), invoke<SettingOptions>('get_setting_options')]);
    if (!quick) startupError.value = (await invoke<string | null>('startup_error')) ?? '';
    if (!quick) await loadGroups();
    await load(); if (!quick) error.value = (await invoke<string | null>('last_error')) ?? '';
    await invoke('frontend_ready');
    await focusSearch();
  } catch (e) { error.value = '初始化失败：' + String(e); }
  window.addEventListener('keydown', keydown);
  window.addEventListener('focus', loadDetail);
  window.addEventListener('blur', conceal);
  document.addEventListener('visibilitychange', visibilityChanged);
  window.addEventListener('resize', checkLoadMore);
});
onUnmounted(() => { conceal(); window.removeEventListener('blur', conceal); document.removeEventListener('visibilitychange', visibilityChanged); clearTimeout(searchTimer); panelObserver?.disconnect(); stops.forEach(stop => stop()); window.removeEventListener('keydown', keydown); window.removeEventListener('focus', loadDetail); window.removeEventListener('resize', checkLoadMore); listGeneration++; detailGeneration++; groupGeneration++; });
</script>

<template>
  <main ref="panelRoot" :class="['app', { quick, 'notice-only': noticeOnly }]">
    <header>
      <div class="brand"><span class="brand-mark">▤</span><div><h1>{{ noticeOnly ? '操作失败' : quick ? '快速粘贴' : '剪贴板' }} <small v-if="!quick">M1</small></h1><p>{{ noticeOnly ? '提示将在约 2 秒后自动关闭' : quick ? `保持 ${quickKey?.hold_label ?? ''} · ↑ ↓ / V 选择` : `本地加密历史 · ${mainLabel} 随时呼出` }}</p></div></div>
      <button v-if="!quick" type="button" class="settings-toggle" :aria-label="settingsOpen ? '返回列表' : '设置'" :disabled="settingsBusy || deleting || busy || organizing" @click="toggleSettings">{{ settingsOpen ? '返回列表' : '⚙ 设置' }}</button>
      <span v-if="!noticeOnly && !settingsOpen" class="count">{{ quick ? '' : '已加载 ' }}{{ items.length }} 条</span>
    </header>
    <div v-if="!quick && !settingsOpen" class="search-tools">
      <input ref="searchInput" v-model="query" type="search" aria-label="搜索历史" placeholder="搜索文本、文件或拼音…" autocomplete="off" :readonly="mutationBusy" :disabled="deleting || groupInteracting || !!pendingDelete" @compositionstart="compositionStart" @compositionend="compositionEnd">
      <div class="kind-filters" role="group" aria-label="类型筛选">
        <button v-for="option in kinds" :key="option.label" type="button" :aria-pressed="kind === option.value" :disabled="deleting || organizing || !!pendingDelete" @click="kind = option.value">{{ option.label }}</button>
      </div>
    </div>
    <GroupSidebar v-if="!quick && !settingsOpen" :groups="groups" :view="view" :disabled="deleting || mutationBusy || busy || !!pendingDelete" @select="view = $event" @changed="groupsChanged" @interacting="groupInteracting = $event; contextMenu = null" @menu-open="groupMenuOpen = $event; if ($event) contextMenu = null" />
    <div v-if="!quick && !settingsOpen && groupError" class="error" role="alert">{{ groupError }} <button type="button" @click="loadGroups">重试分组</button></div>
    <div v-if="!quick && startupError" class="error" role="alert">{{ startupError }}</div>
    <SettingsView v-if="settingsOpen && !quick" @saved="settings = $event" @busy="settingsBusy = $event" />
    <div v-if="error" class="error" role="alert">{{ error }}</div>
    <div v-if="!quick && !settingsOpen && pageError" class="error" role="alert">{{ pageError }} <button type="button" @click="retryPage">重试</button></div>
    <div v-if="!quick && !settingsOpen && pendingDelete" class="delete-confirm" role="alertdialog" aria-label="确认删除" aria-describedby="delete-description">
      <span id="delete-description">将删除 {{ pendingDelete.length }} 条，其中 {{ protectedDeleteCount }} 条是置顶或收藏记录</span>
      <button type="button" @click="confirmDelete">确认</button><button type="button" @click="pendingDelete = null">取消</button>
    </div>
    <ContextMenu v-if="!quick && !settingsOpen && contextMenu" :key="contextMenu.id + ':' + contextMenu.x + ':' + contextMenu.y" :x="contextMenu.x" :y="contextMenu.y" :groups="groups" :all-pinned="allPinned" :all-favorite="allFavorite" :all-sensitive="allSensitive" @sensitive="changeItems('set_sensitive')" @pin="changeItems('set_pinned')" @favorite="changeItems('set_favorite')" @move="changeItems('move_to_group', $event)" @delete="requestDelete" @paste="contextPaste" @close="contextMenu = null" />
    <section v-if="!settingsOpen && !noticeOnly && !items.length" class="empty"><span>▤</span><h2>{{ !quick && pageLoading ? '正在搜索…' : !quick && filtered ? '没有匹配的记录' : '从一次复制开始' }}</h2><p>{{ !quick && filtered ? '试试其他关键词或类型。' : '复制文本、图片或文件后，会显示在这里。' }}</p><p v-if="!quick && !filtered">{{ quickKey?.label }} 打开快速面板</p></section>
    <div v-else-if="!settingsOpen && !noticeOnly" class="workspace">
    <HistoryList ref="historyList" :items="items" :groups="groups" :selected="selected" :selected-ids="selectedIds" :quick="quick" :filtered="filtered" :loading="pageLoading" :has-more="hasMore" :disabled="deleting || organizing || !!pendingDelete" @choose="choose" @context="openContext" @paste="paste(false)" @scroll="contextMenu = null; checkLoadMore()" @thumbnail-loaded="invoke('thumbnail_loaded').catch(() => {})" />
    <aside v-if="!quick && active" class="detail" aria-label="选中项预览">
      <p v-if="active.source_exe" class="source-name" :title="active.source_exe">来源：{{ active.source_exe }}</p>
      <div v-if="active.sensitive" class="sensitive-actions"><span>敏感内容</span><button v-if="!revealed" type="button" :disabled="detailLoading" @click="revealDetail">显示原文</button><button v-else type="button" @click="loadDetail">隐藏原文</button></div>
      <div class="detail-header"><h2>{{ kindLabel(active) }}预览</h2><button type="button" @click="loadDetail">刷新状态</button></div>
      <PreviewImage v-if="active.kind === 'image' || active.thumbnail || active.preview || active.placeholder" :hash="active.preview || active.thumbnail" :placeholder="active.placeholder" large @loaded="invoke('preview_loaded').catch(() => {})" />
      <p v-if="active.kind === 'image' && !active.preview && active.thumbnail" class="detail-note">旧记录：显示缩略图</p>
      <p v-if="detailLoading" role="status">正在读取详情…</p>
      <p v-else-if="detailError" class="error" role="alert">{{ detailError }}</p>
      <template v-else-if="detail">
        <template v-if="active.kind === 'text' || active.kind === 'rich'">
          <p v-if="detail.truncated" class="detail-note">仅显示前 20000 个字符</p>
          <pre class="text-detail">{{ detail.text || '此记录没有纯文本内容' }}</pre>
        </template>
        <ul v-if="active.kind === 'files'" class="file-detail">
          <li v-for="file in detail.files" :key="file.path"><span :class="['file-status', { missing: file.exists !== true }]">{{ file.exists === true ? '存在' : file.exists === false ? '已失效' : '无法检查' }}</span><span>{{ file.path }}</span></li>
        </ul>
      </template>
    </aside>
    </div>
    <footer v-if="quick && !noticeOnly"><span>松开修饰键即可粘贴</span><span>Esc 取消</span></footer>
    <footer v-else-if="!quick && !settingsOpen"><span>Enter 粘贴 · Shift + Enter 纯文本</span><button :disabled="!active || busy || deleting || organizing || !!pendingDelete" @click="paste(false)">粘贴选中项 ↵</button></footer>
  </main>
</template>

<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import PreviewImage from './PreviewImage.vue';
interface Item { id: string; kind: string; summary: string; formats: string[]; thumbnail: string | null; preview?: string | null; placeholder: string | null; files: string[] }
interface Panel { items: Item[]; selected: number; visible: boolean; error: string | null }
const quick = new URLSearchParams(location.search).get('panel') === 'quick';
const items = ref<Item[]>([]), selected = ref(0), error = ref(''), busy = ref(false);
const PAGE_SIZE = 100;
const listRoot = ref<HTMLElement | null>(null), pageLoading = ref(false), pageError = ref(''), hasMore = ref(true);
let listGeneration = 0, nextOffset = 0, retryRefresh = false;
function checkLoadMore() {
  const list = listRoot.value;
  if (!quick && list && list.scrollHeight - list.scrollTop - list.clientHeight < 120) void loadMore();
}
async function loadMore() {
  if (quick || pageLoading.value || pageError.value || !hasMore.value) return;
  const generation = listGeneration, offset = nextOffset;
  pageLoading.value = true;
  try {
    const page = await invoke<Item[]>('list_items', { filter: {}, offset, limit: PAGE_SIZE });
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
let detailGeneration = 0;
async function loadDetail() {
  if (quick) return;
  const id = active.value?.id, generation = ++detailGeneration;
  detail.value = null; detailError.value = ''; detailLoading.value = !!id;
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
watch(() => active.value?.id, () => { void loadDetail(); });
const kindLabel = (item: Item) => ({ text: '文本', image: '图片', files: '文件', rich: '富文本' }[item.kind] ?? '内容');
const filename = (p: string) => p.split(/[\\/]/).pop() ?? p;
async function load() {
  if (quick) { const p = await invoke<Panel>('panel_state'); applyPanel(p); return; }
  const generation = ++listGeneration, targetCount = Math.max(PAGE_SIZE, items.value.length);
  const scrollTop = listRoot.value?.scrollTop ?? 0;
  pageLoading.value = true; pageError.value = '';
  try {
    const refreshed: Item[] = [];
    let more = true;
    // 重建已加载范围；选中项移到下一页时继续查找，保留选择。
    while (more) {
      const page = await invoke<Item[]>('list_items', { filter: {}, offset: refreshed.length, limit: PAGE_SIZE });
      if (generation !== listGeneration) return;
      refreshed.push(...page); more = page.length === PAGE_SIZE;
      const id = active.value?.id;
      if (refreshed.length >= targetCount && (!id || refreshed.some(item => item.id === id))) break;
    }
    const id = active.value?.id;
    const unique = new Map(refreshed.map(item => [item.id, item]));
    items.value = [...unique.values()];
    selected.value = Math.max(0, items.value.findIndex(item => item.id === id));
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
  if (quick || !active.value || busy.value) return;
  busy.value = true; error.value = '';
  try { await invoke('paste_item', { id: active.value.id, plain }); }
  catch (e) { error.value = String(e); }
  finally { busy.value = false; }
}
async function keydown(event: KeyboardEvent) {
  if (quick || event.isComposing) return;
  if (event.key === 'Enter') { event.preventDefault(); void paste(event.shiftKey); }
  else if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
    event.preventDefault();
    if (event.key === 'ArrowDown' && selected.value === items.value.length - 1 && hasMore.value) {
      if (pageLoading.value || pageError.value) return;
      await loadMore();
      if (selected.value === items.value.length - 1 && hasMore.value) return;
    }
    if (items.value.length) selected.value = (selected.value + (event.key === 'ArrowDown' ? 1 : -1) + items.value.length) % items.value.length;
    void nextTick(() => document.querySelector('.item.selected')?.scrollIntoView({ block: 'nearest' }));
  } else if (event.key === 'Escape') { void invoke('hide_main'); }
}
onMounted(async () => {
  if (quick && panelRoot.value) { panelObserver = new ResizeObserver(resizeQuick); panelObserver.observe(panelRoot.value); }
  try {
    stops.push(await listen<Panel>('panel-state', ({ payload }) => { if (quick) applyPanel(payload); }));
    stops.push(await listen('history-changed', () => { if (!quick) void load(); }));
    stops.push(await listen<string>('operation-error', ({ payload }) => { if (!quick) error.value = payload; }));
    await load(); if (!quick) error.value = (await invoke<string | null>('last_error')) ?? '';
    await invoke('frontend_ready');
  } catch (e) { error.value = '初始化失败：' + String(e); }
  window.addEventListener('keydown', keydown);
  window.addEventListener('focus', loadDetail);
  window.addEventListener('resize', checkLoadMore);
});
onUnmounted(() => { panelObserver?.disconnect(); stops.forEach(stop => stop()); window.removeEventListener('keydown', keydown); window.removeEventListener('focus', loadDetail); window.removeEventListener('resize', checkLoadMore); listGeneration++; detailGeneration++; });
</script>

<template>
  <main ref="panelRoot" :class="['app', { quick, 'notice-only': noticeOnly }]">
    <header>
      <div class="brand"><span class="brand-mark">▤</span><div><h1>{{ noticeOnly ? '操作失败' : quick ? '快速粘贴' : '剪贴板' }} <small v-if="!quick">M1</small></h1><p>{{ noticeOnly ? '提示将在约 2 秒后自动关闭' : quick ? '保持 Ctrl + Shift · ↑ ↓ / V 选择' : '本地加密历史 · Alt + V 随时呼出' }}</p></div></div>
      <span v-if="!noticeOnly" class="count">{{ quick ? '' : '已加载 ' }}{{ items.length }} 条</span>
    </header>
    <div v-if="error" class="error" role="alert">{{ error }}</div>
    <div v-if="!quick && pageError" class="error" role="alert">{{ pageError }} <button type="button" @click="retryPage">重试</button></div>
    <section v-if="!noticeOnly && !items.length" class="empty"><span>▤</span><h2>从一次复制开始</h2><p>复制文本、图片或文件后，会显示在这里。</p><p v-if="!quick">Ctrl + Shift + V 打开快速面板</p></section>
    <div v-else-if="!noticeOnly" class="workspace">
    <section ref="listRoot" class="list" @scroll.passive="checkLoadMore" aria-label="剪贴板历史" role="listbox">
      <button v-for="(item, index) in items" :key="item.id" type="button" :class="['item', { selected: index === selected }]" role="option" :aria-selected="index === selected" :tabindex="quick ? -1 : 0" @click="selected = index" @dblclick="paste(false)">
        <span class="index">{{ index + 1 }}</span>
        <PreviewImage v-if="item.kind === 'image' || item.thumbnail || item.placeholder" :hash="item.thumbnail" :placeholder="item.placeholder" @loaded="invoke('thumbnail_loaded').catch(() => {})" />
        <div class="content"><div class="summary">{{ item.kind === 'files' ? item.files.map(filename).join('、') : item.summary || '空文本' }}</div><div class="meta">{{ kindLabel(item) }}<span v-if="item.kind === 'files'"> · {{ item.files.length }} 个 · {{ item.formats.includes('Preferred DropEffect') ? '保留复制 / 剪切语义' : '' }}</span><span v-else> · {{ item.formats.join(' / ') }}</span></div></div>
        <span v-if="index === selected" class="selected-mark">↵</span>
      </button>
      <p v-if="!quick" class="detail-note" role="status">{{ pageLoading ? '正在加载…' : hasMore ? '向下滚动加载更多' : '已加载全部历史' }}</p>
    </section>
    <aside v-if="!quick && active" class="detail" aria-label="选中项预览">
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
    <footer v-else-if="!quick"><span>Enter 粘贴 · Shift + Enter 纯文本</span><button :disabled="!active || busy" @click="paste(false)">粘贴选中项 ↵</button></footer>
  </main>
</template>

<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import PreviewImage from './PreviewImage.vue';
interface Item { id: string; kind: string; summary: string; formats: string[]; thumbnail: string | null; preview?: string | null; placeholder: string | null; files: string[] }
interface Panel { items: Item[]; selected: number; visible: boolean; error: string | null }
const quick = new URLSearchParams(location.search).get('panel') === 'quick';
const items = ref<Item[]>([]), selected = ref(0), error = ref(''), busy = ref(false);
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
const kindLabel = (item: Item) => ({ text: '文本', image: '图片', files: '文件' }[item.kind] ?? '内容');
const filename = (p: string) => p.split(/[\\/]/).pop() ?? p;
async function load() {
  if (quick) { const p = await invoke<Panel>('panel_state'); applyPanel(p); }
  else { const id = active.value?.id; items.value = await invoke<Item[]>('list_items'); selected.value = Math.max(0, items.value.findIndex(i => i.id === id)); if (id === active.value?.id) void loadDetail(); }
}
async function paste(plain = false) {
  if (quick || !active.value || busy.value) return;
  busy.value = true; error.value = '';
  try { await invoke('paste_item', { id: active.value.id, plain }); }
  catch (e) { error.value = String(e); }
  finally { busy.value = false; }
}
function keydown(event: KeyboardEvent) {
  if (quick || event.isComposing) return;
  if (event.key === 'Enter') { event.preventDefault(); void paste(event.shiftKey); }
  else if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
    event.preventDefault(); if (items.value.length) selected.value = (selected.value + (event.key === 'ArrowDown' ? 1 : -1) + items.value.length) % items.value.length;
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
});
onUnmounted(() => { panelObserver?.disconnect(); stops.forEach(stop => stop()); window.removeEventListener('keydown', keydown); window.removeEventListener('focus', loadDetail); detailGeneration++; });
</script>

<template>
  <main ref="panelRoot" :class="['app', { quick, 'notice-only': noticeOnly }]">
    <header>
      <div class="brand"><span class="brand-mark">▤</span><div><h1>{{ noticeOnly ? '操作失败' : quick ? '快速粘贴' : '剪贴板' }} <small v-if="!quick">M0</small></h1><p>{{ noticeOnly ? '提示将在约 2 秒后自动关闭' : quick ? '保持 Ctrl + Shift · ↑ ↓ / V 选择' : '本地加密历史 · Alt + V 随时呼出' }}</p></div></div>
      <span v-if="!noticeOnly" class="count">{{ items.length }} 条</span>
    </header>
    <div v-if="error" class="error" role="alert">{{ error }}</div>
    <section v-if="!noticeOnly && !items.length" class="empty"><span>▤</span><h2>从一次复制开始</h2><p>复制文本、图片或文件后，会显示在这里。</p><p v-if="!quick">Ctrl + Shift + V 打开快速面板</p></section>
    <div v-else-if="!noticeOnly" class="workspace">
    <section class="list" aria-label="剪贴板历史" role="listbox">
      <button v-for="(item, index) in items" :key="item.id" type="button" :class="['item', { selected: index === selected }]" role="option" :aria-selected="index === selected" :tabindex="quick ? -1 : 0" @click="selected = index" @dblclick="paste(false)">
        <span class="index">{{ index + 1 }}</span>
        <PreviewImage v-if="item.kind === 'image' || item.thumbnail || item.placeholder" :hash="item.thumbnail" :placeholder="item.placeholder" @loaded="invoke('thumbnail_loaded').catch(() => {})" />
        <div class="content"><div class="summary">{{ item.kind === 'files' ? item.files.map(filename).join('、') : item.summary || '空文本' }}</div><div class="meta">{{ kindLabel(item) }}<span v-if="item.kind === 'files'"> · {{ item.files.length }} 个 · {{ item.formats.includes('Preferred DropEffect') ? '保留复制 / 剪切语义' : '' }}</span><span v-else> · {{ item.formats.join(' / ') }}</span></div></div>
        <span v-if="index === selected" class="selected-mark">↵</span>
      </button>
    </section>
    <aside v-if="!quick && active" class="detail" aria-label="选中项预览">
      <div class="detail-header"><h2>{{ kindLabel(active) }}预览</h2><button type="button" @click="loadDetail">刷新状态</button></div>
      <PreviewImage v-if="active.kind === 'image' || active.thumbnail || active.preview || active.placeholder" :hash="active.preview || active.thumbnail" :placeholder="active.placeholder" large @loaded="invoke('preview_loaded').catch(() => {})" />
      <p v-if="active.kind === 'image' && !active.preview && active.thumbnail" class="detail-note">旧记录：显示缩略图</p>
      <p v-if="detailLoading" role="status">正在读取详情…</p>
      <p v-else-if="detailError" class="error" role="alert">{{ detailError }}</p>
      <template v-else-if="detail">
        <template v-if="active.kind === 'text'">
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

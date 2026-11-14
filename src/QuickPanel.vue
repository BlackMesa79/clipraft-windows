<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import HistoryList from './HistoryList.vue';
import UiIcon from './UiIcon.vue';
import type { Item } from './history';
import { createPanelState, type PanelState } from './panel';
import {
  applyTheme,
  shortcutLabel,
  shortcutModifiersLabel,
  type Settings,
  type SettingOptions,
} from './settings';
import './styles/quick.css';

const { panel, applyPanel } = createPanelState();
const items = computed<Item[]>(() => panel.value.items),
  selected = computed(() => panel.value.selected),
  error = ref('');
watch(
  () => panel.value.error,
  (message) => {
    error.value = message ?? '';
  },
);
const settings = ref<Settings>(),
  settingOptions = ref<SettingOptions>();
watch(settings, applyTheme);
const quickKey = computed(() => ({
  hold_label: shortcutModifiersLabel(settings.value?.quick_hotkey),
  trigger: shortcutLabel(settings.value?.quick_hotkey).split(' + ').at(-1) ?? '',
}));
const panelVisible = computed(() => panel.value.visible);
const noticeOnly = computed(() => !panelVisible.value && !!error.value);
const panelRoot = ref<HTMLElement | null>(null);
const selectedIds = new Set<string>();
const positions = computed(() =>
  items.value.map(
    (_, index) =>
      ((panel.value.position + panel.value.total + index - selected.value) % panel.value.total) + 1,
  ),
);
let panelObserver: ResizeObserver | undefined;
let lastPanelSize = '';
function resizeQuick() {
  if (!panelRoot.value) return;
  const notice = noticeOnly.value;
  const height = notice ? Math.ceil(panelRoot.value.getBoundingClientRect().height) : 530;
  const size = String(notice) + ':' + height;
  if (size === lastPanelSize) return;
  lastPanelSize = size;
  // 保持原有不激活窗口的尺寸协议；U1 不改变显示和输入行为。
  void invoke('resize_quick', { height, notice }).catch(() => {
    lastPanelSize = '';
  });
}
watch([noticeOnly, error, panelVisible], resizeQuick, { flush: 'post' });
const stops: UnlistenFn[] = [];
onMounted(async () => {
  if (panelRoot.value) {
    panelObserver = new ResizeObserver(resizeQuick);
    panelObserver.observe(panelRoot.value);
  }
  try {
    stops.push(await listen<PanelState>('panel-state', ({ payload }) => applyPanel(payload)));
    stops.push(
      await listen<Settings>('settings-changed', ({ payload }) => {
        settings.value = payload;
      }),
    );
    [settings.value, settingOptions.value] = await Promise.all([
      invoke<Settings>('get_settings'),
      invoke<SettingOptions>('get_setting_options'),
    ]);
    applyPanel(await invoke<PanelState>('panel_state'));
    await invoke('frontend_ready');
  } catch (e) {
    error.value = '初始化失败：' + String(e);
  }
});
onUnmounted(() => {
  panelObserver?.disconnect();
  stops.forEach((stop) => stop());
});
</script>

<template>
  <main ref="panelRoot" :class="['app', 'quick', { 'notice-only': noticeOnly }]">
    <header>
      <div class="brand"
        ><span class="brand-mark"><UiIcon name="brand" /></span
        ><div
          ><h1>{{ noticeOnly ? '操作失败' : '快速粘贴' }}</h1
          ><p>{{
            noticeOnly
              ? '提示将在约 2 秒后自动关闭'
              : `保持 ${quickKey?.hold_label ?? ''} · ↑ ↓ / ${quickKey?.trigger ?? ''} 选择`
          }}</p></div
        ></div
      >
      <span v-if="!noticeOnly" class="count"
        >{{ panel.total ? panel.position + 1 : 0 }} / {{ panel.total }} 条</span
      >
    </header>
    <div v-if="error" class="error" role="alert">{{ error }}</div>
    <section v-if="!noticeOnly && panel.loading" class="empty" role="status" aria-busy="true"
      ><h2>正在加载所选记录…</h2><p>可继续选择，松开修饰键粘贴当前项。</p></section
    >
    <section v-else-if="!noticeOnly && !items.length && error" class="empty" role="status"
      ><h2>所选记录暂时无法显示</h2><p>继续选择可重试，Esc 取消。</p></section
    >
    <section v-else-if="!noticeOnly && !items.length" class="empty"
      ><span><UiIcon name="copy" /></span><h2>从一次复制开始</h2
      ><p>复制文本、图片或文件后，会显示在这里。</p></section
    >
    <div v-else-if="!noticeOnly" class="workspace">
      <HistoryList
        :items="items"
        :positions="positions"
        :selected="selected"
        :selected-ids="selectedIds"
        quick
        :loading="panel.loading"
        :has-more="true"
        :disabled="false"
        @thumbnail-loaded="invoke('thumbnail_loaded').catch(() => {})"
      />
    </div>
    <footer v-if="!noticeOnly"><span>松开修饰键即可粘贴</span><span>Esc 取消</span></footer>
  </main>
</template>

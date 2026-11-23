<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import QuickWheel from './QuickWheel.vue';
import { wheelGeometry, wheelHeight } from './quickWheel';
import UiIcon from './UiIcon.vue';
import { createPanelState, type PanelState } from './panel';
import { type Settings, type SettingOptions } from './settings';
import { useAppearance } from './useAppearance';
import './styles/quick.css';
import { createPanelDiagnostics } from './panelDiagnostics';

const { panel, applyPanel } = createPanelState();
const diagnostics = createPanelDiagnostics();
watch(panel, diagnostics.record, { flush: 'post' });
const items = computed(() => panel.value.items);
const error = ref('');
watch(
  () => panel.value.error,
  (message) => {
    error.value = message ?? '';
  },
);
const settings = ref<Settings>(),
  settingOptions = ref<SettingOptions>();
useAppearance(settings);
const panelVisible = computed(() => panel.value.visible);
const noticeOnly = computed(() => !panelVisible.value && !!error.value);
const panelRoot = ref<HTMLElement | null>(null);
const viewport = ref({ width: window.innerWidth, height: window.innerHeight });
const stageHeight = computed(() => wheelHeight(panel.value.visible_count || 5));
const stageStyle = computed(() => ({
  width: `${wheelGeometry.width}px`,
  height: `${stageHeight.value}px`,
  transform: `translate(-50%, -50%) scale(${Math.min(1, viewport.value.width / wheelGeometry.width, viewport.value.height / stageHeight.value)})`,
}));
function updateViewport() {
  viewport.value = { width: window.innerWidth, height: window.innerHeight };
}
let panelObserver: ResizeObserver | undefined;
let lastPanelSize = '';
function resizeQuick() {
  if (!panelRoot.value) return;
  const notice = noticeOnly.value;
  const height = notice
    ? Math.ceil(panelRoot.value.getBoundingClientRect().height)
    : stageHeight.value;
  const size = `${notice}:${height}:${panel.value.session}`;
  if (size === lastPanelSize) return;
  lastPanelSize = size;
  // 尺寸由会话条数决定，不随邻域加载或选中变化。
  void invoke('resize_quick', { height, notice, session: panel.value.session }).catch(() => {
    lastPanelSize = '';
  });
}
watch([noticeOnly, error, panelVisible, stageHeight, () => panel.value.session], resizeQuick, {
  flush: 'post',
});
const stops: UnlistenFn[] = [];
onMounted(async () => {
  window.addEventListener('resize', updateViewport);
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
    const [initialSettings, options] = await Promise.all([
      invoke<Settings>('get_settings'),
      invoke<SettingOptions>('get_setting_options'),
    ]);
    // 初始化等待期间可能已收到保存事件，不能用旧快照覆盖新设置。
    settings.value ??= initialSettings;
    settingOptions.value = options;
    applyPanel(await invoke<PanelState>('panel_state'));
    diagnostics.start(await invoke<boolean>('frontend_ready'), panel.value);
  } catch (e) {
    error.value = '初始化失败：' + String(e);
  }
});
onUnmounted(() => {
  diagnostics.stop();
  window.removeEventListener('resize', updateViewport);
  panelObserver?.disconnect();
  stops.forEach((stop) => stop());
});
</script>

<template>
  <main ref="panelRoot" :class="['app', 'quick', { 'notice-only': noticeOnly }]">
    <div v-if="!noticeOnly" class="wheel-stage" :style="stageStyle">
      <span v-if="panelVisible" class="cancel-hint">空格 / 右键取消</span>
      <span
        class="count"
        :aria-label="`第 ${panel.total ? panel.position + 1 : 0} 条，共 ${panel.total} 条记录`"
      >
        <span class="count-current">{{ panel.total ? panel.position + 1 : 0 }}</span>
        <span aria-hidden="true">/</span>
        <span class="count-total">{{ panel.total }}</span><span>条</span>
      </span>
      <section v-if="panel.loading" class="empty" role="status" aria-busy="true"
        ><h2>正在加载所选记录…</h2><p>可继续选择，松开修饰键粘贴当前项。</p></section
      >
      <section v-else-if="!items.length && error" class="empty" role="status"
        ><h2>所选记录暂时无法显示</h2><p>继续选择可重试。</p></section
      >
      <section v-else-if="!items.length" class="empty"
        ><span><UiIcon name="copy" /></span><h2>从一次复制开始</h2
        ><p>复制文本、图片或文件后，会显示在这里。</p></section
      >
      <QuickWheel :panel="panel" @thumbnail-loaded="invoke('thumbnail_loaded').catch(() => {})" />
      <div v-if="error" class="error wheel-capsule" role="alert">{{ error }}</div>
    </div>
    <div v-else class="error wheel-capsule quick-notice" role="alert">
      <span class="notice-symbol" aria-hidden="true">!</span>
      <div class="notice-copy"
        ><strong>操作失败</strong><p>{{ error }}</p></div
      >
    </div>
  </main>
</template>

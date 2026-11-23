<script setup lang="ts">
import { onMounted, onUnmounted, ref, watch, nextTick } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import PreviewImage from './PreviewImage.vue';
import RichPreview from './RichPreview.vue';
import type { RichPreview as RichContent } from './richPreview';
import { kindLabel, type Item } from './history';
import type { Settings } from './settings';
import { useAppearance } from './useAppearance';
import { createDetailImage } from './detailImage';
interface Content {
  generation?: number;
  item: Item;
  detail: {
    rich?: RichContent;
    image?: number[];
    text: string;
    truncated: boolean;
    files: { path: string; exists: boolean | null }[];
  };
}
interface Geometry {
  right: boolean;
  arrow: number;
  width?: number;
}
const geometry = ref<Geometry>({ right: true, arrow: 60 });
let geometryRevision = 0;
const content = ref<Content>();
const { imageSource, replaceImage } = createDetailImage();
const error = ref('');
const panel = ref<HTMLElement | null>(null),
  body = ref<HTMLElement | null>(null);
let observer: ResizeObserver | undefined,
  measureFrame = 0,
  lastMeasure = '';
function measureHeight() {
  if (disposed) return;
  if (measureFrame) return;
  measureFrame = requestAnimationFrame(() => {
    measureFrame = 0;
    if (!content.value || !panel.value || !body.value) return;
    const root = panel.value;
    const header = root.querySelector('header')?.getBoundingClientRect().height ?? 0;
    const footer = root.querySelector('footer')?.getBoundingClientRect().height ?? 0;
    // 内容自然高度与窗口高度分离，短内容可以真正缩回，而不是读scrollHeight的旧下限。
    const height = Math.max(
      120,
      Math.min(
        640,
        Math.ceil(body.value.getBoundingClientRect().height + header + footer + 32 + 2 + 18),
      ),
    );
    const generation = content.value.generation ?? 0;
    const signature = `${generation}:${height}:${root.clientWidth}`;
    if (signature === lastMeasure) return;
    lastMeasure = signature;
    void invoke('resize_compact_preview', { generation, height }).catch(() => {
      if (!disposed && signature === lastMeasure) {
        lastMeasure = '';
        error.value = '预览高度调整失败，重新选择记录后重试';
      }
    });
  });
}

const stops: UnlistenFn[] = [];
const contentRevision = ref(0);
const appearance = ref<Settings>();
useAppearance(appearance);
let revision = 0,
  themeRevision = 0,
  disposed = false;
function receive(value: Content | null) {
  revision++;
  contentRevision.value++;
  content.value = value?.item && value.detail ? value : undefined;
  replaceImage(content.value?.detail.image);
  error.value = '';
}
watch([contentRevision, () => geometry.value.width], async () => {
  await nextTick();
  if (disposed) return;
  observer?.disconnect();
  if (!content.value) return;
  if (!observer) observer = new ResizeObserver(measureHeight);
  for (const element of [
    body.value,
    panel.value?.querySelector('header'),
    panel.value?.querySelector('footer'),
  ])
    if (element) observer.observe(element);
  measureHeight();
});
onMounted(async () => {
  try {
    stops.push(
      await listen<Geometry>('compact-preview-geometry', ({ payload }) => {
        geometryRevision++;
        geometry.value = payload;
      }),
    );
    const geometryVersion = geometryRevision;
    const initialGeometry = await invoke<Geometry | null>('compact_preview_geometry');
    if (geometryVersion === geometryRevision && initialGeometry) geometry.value = initialGeometry;
    stops.push(
      await listen<Content | null>('compact-preview-changed', ({ payload }) => receive(payload)),
    );
    stops.push(
      await listen<Settings>('settings-changed', ({ payload }) => {
        themeRevision++;
        appearance.value = payload;
      }),
    );
    const version = revision,
      themeVersion = themeRevision;
    const [snapshot, settings] = await Promise.all([
      invoke<Content | null>('compact_preview_state'),
      invoke<Settings>('get_settings'),
    ]);
    if (disposed) return;
    if (themeRevision === themeVersion) appearance.value = settings;
    if (revision === version) receive(snapshot);
  } catch {
    if (!disposed) error.value = '预览读取失败，重新选择记录后重试';
  }
});
onUnmounted(() => {
  replaceImage();
  disposed = true;
  revision++;
  observer?.disconnect();
  cancelAnimationFrame(measureFrame);
  stops.forEach((stop) => stop());
});
</script>

<template>
  <div
    class="floating-shell"
    :class="geometry.right ? 'on-right' : 'on-left'"
    :style="{ '--arrow-y': geometry.arrow + 'px' }"
    ><i class="preview-arrow" aria-hidden="true" /><main
      ref="panel"
      class="floating-preview"
      aria-label="选中记录浮动预览"
    >
      <template v-if="content">
        <header
          ><span class="floating-title"
            >{{ content.detail.rich ? '图文' : kindLabel(content.item) }}预览</span
          ><span v-if="content.item.sensitive">敏感内容 · 已打码</span
          ><span v-else>只读预览</span></header
        >
        <section class="floating-content" :key="contentRevision"
          ><div ref="body" class="floating-body">
            <PreviewImage
              v-if="imageSource && content.item.sensitive"
              :source="imageSource"
              large
              @loaded="measureHeight"
            />
            <PreviewImage
              v-if="
                !content.item.sensitive &&
                (content.item.kind === 'image' ||
                  content.item.preview ||
                  content.item.thumbnail ||
                  content.item.placeholder)
              "
              :hash="content.item.preview || content.item.thumbnail"
              :placeholder="content.item.placeholder"
              large
              @loaded="measureHeight"
            />
            <RichPreview
              v-if="content.detail.rich"
              :content="content.detail.rich"
              @loaded="measureHeight"
            />
            <pre v-else-if="content.detail.text" class="floating-text">{{
              content.detail.text
            }}</pre>
            <ul v-if="content.detail.files.length" class="floating-files"
              ><li v-for="file in content.detail.files" :key="file.path"
                >{{ file.path }}<small v-if="file.exists === false">源文件已不存在</small
                ><small v-else-if="file.exists == null">无法检查源文件</small></li
              ></ul
            >
            <p v-if="content.detail.truncated" class="detail-note">内容过长，仅显示部分预览</p>
            <p
              v-if="
                !imageSource &&
                !content.detail.text &&
                !content.detail.files.length &&
                content.item.sensitive
              "
              >{{ content.item.summary }}</p
            >
          </div></section
        >
        <footer v-if="content.item.source_exe">来源：{{ content.item.source_exe }}</footer>
      </template>
      <p v-if="error" role="alert">{{ error }}</p>
    </main></div
  >
</template>

<style scoped>
.floating-shell {
  height: 100vh;
  position: relative;
  background: transparent;
  padding: 8px 8px 10px 18px;
}
.floating-shell.on-left {
  padding-left: 8px;
  padding-right: 18px;
}
.preview-arrow {
  position: absolute;
  left: 8px;
  top: calc(var(--arrow-y) - 7px);
  width: 11px;
  height: 14px;
  background: var(--preview-border);
  clip-path: polygon(100% 0, 0 50%, 100% 100%);
}
.preview-arrow::after {
  content: '';
  position: absolute;
  inset: 1px 0 1px 1px;
  background: var(--bg);
  clip-path: polygon(100% 0, 0 50%, 100% 100%);
}
.on-left .preview-arrow {
  left: auto;
  right: 8px;
  transform: rotate(180deg);
}
.floating-preview {
  height: 100%;
  display: flex;
  flex-direction: column;
  background: var(--bg);
  color: var(--text);
  border: 1px solid var(--preview-border);
  border-radius: var(--radius-preview);
  box-shadow: var(--preview-shadow);
  overflow: hidden;
}
header {
  flex-shrink: 0;
  padding: 14px 16px;
  font-size: var(--font-note-size);
  color: var(--muted);
  border-bottom: 1px solid var(--line);
  gap: 10px;
}
.floating-content {
  flex: 1;
  min-height: 0;
  overflow: auto;
  scrollbar-gutter: stable;
  padding: 16px;
  overflow-wrap: anywhere;
}
.floating-body {
  display: flow-root;
}
.floating-text {
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  font: var(--font-body-size) / 1.8 var(--font-ui);
  margin: 0;
}
.floating-files {
  list-style: none;
  padding: 0;
  margin: 0;
  font-size: var(--font-label-size);
}
.floating-files li {
  padding: 10px 0;
  border-bottom: 1px solid var(--line);
}
small {
  display: block;
  color: var(--warning);
  margin-top: 5px;
}
.floating-content :deep(.large-preview) {
  min-height: 0;
  display: block;
  max-width: 100%;
  background: var(--surface);
  border-radius: var(--radius-placeholder);
  overflow: hidden;
  margin-bottom: 12px;
}
.floating-content :deep(img) {
  object-fit: contain;
  width: 100%;
  height: auto;
  max-width: 100%;
  max-height: none;
}
.floating-content :deep(.image-placeholder) {
  background: var(--surface);
  color: var(--muted);
  padding: 16px;
  overflow-wrap: anywhere;
}
footer {
  flex-shrink: 0;
  border-top: 1px solid var(--line);
  padding: 10px 16px;
  font-size: var(--font-caption-size);
  color: var(--muted);
  overflow: hidden;
  white-space: nowrap;
  text-overflow: ellipsis;
}
[role='alert'] {
  padding: 16px;
  color: var(--danger);
  font-size: var(--font-label-size);
}
</style>

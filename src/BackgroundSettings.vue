<script setup lang="ts">
import { computed, reactive, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import {
  backgroundLayouts,
  backgroundStyle,
  scrimRange,
  type BackgroundLayout,
  type BackgroundOptions,
  type Backgrounds,
  type MainBackground,
} from './mainBackground';

const props = defineProps<{
  backgrounds: Backgrounds;
  activeLayout: BackgroundLayout;
  disabled: boolean;
}>();
const emit = defineEmits<{
  changed: [layout: BackgroundLayout, background: MainBackground | null];
  busy: [value: boolean];
}>();
const busy = ref(false);
const error = ref('');
const status = ref('');
const labels = { regular: '普通布局', compact: '紧凑布局' } satisfies Record<
  BackgroundLayout,
  string
>;
const editingLayout = ref(props.activeLayout);
const drafts = reactive<Partial<Record<BackgroundLayout, BackgroundOptions>>>({});
const background = computed(() => props.backgrounds[editingLayout.value].asset);
const loadError = computed(() => props.backgrounds[editingLayout.value].error);
const draft = computed(() => drafts[editingLayout.value]);
const scrimProgress = computed(() => {
  const { min, max } = scrimRange;
  return `${(((draft.value?.scrim ?? min) - min) * 100) / (max - min)}%`;
});
const changed = computed(
  () => JSON.stringify(draft.value) !== JSON.stringify(background.value?.options),
);

for (const layout of backgroundLayouts) {
  watch(
    () => props.backgrounds[layout].asset,
    (asset) => {
      drafts[layout] = asset ? { ...asset.options } : undefined;
    },
    { immediate: true },
  );
}
watch(editingLayout, () => {
  error.value = '';
  status.value = '';
});

function previewStyle(layout: BackgroundLayout): Record<string, string> | undefined {
  const asset = props.backgrounds[layout].asset;
  const options = drafts[layout];
  return asset && options ? backgroundStyle(asset.jpeg, options) : undefined;
}

async function run(action: () => Promise<void>): Promise<void> {
  if (props.disabled || busy.value) return;
  busy.value = true;
  emit('busy', true);
  error.value = '';
  status.value = '';
  try {
    await action();
  } catch (cause) {
    error.value = String(cause);
  } finally {
    busy.value = false;
    emit('busy', false);
  }
}

function importBackground(): Promise<void> {
  return run(async () => {
    const layout = editingLayout.value;
    const asset = await invoke<MainBackground | null>('import_main_background', { layout });
    if (!asset) return;
    emit('changed', layout, asset);
    status.value = `${labels[layout]}背景已应用`;
  });
}

function applyOptions(): Promise<void> {
  return run(async () => {
    if (!background.value || !draft.value) return;
    const layout = editingLayout.value;
    const asset = { ...background.value, options: { ...draft.value } };
    await invoke('set_main_background_options', { layout, options: asset.options });
    emit('changed', layout, asset);
    status.value = '背景调整已应用';
  });
}

function removeBackground(): Promise<void> {
  return run(async () => {
    const layout = editingLayout.value;
    await invoke('remove_main_background', { layout });
    emit('changed', layout, null);
    status.value = '已恢复默认背景';
  });
}
</script>

<template>
  <section class="background-settings" aria-labelledby="main-background-heading" :aria-busy="busy">
    <h3 id="main-background-heading">自定义背景</h3>
    <div class="background-targets" role="group" aria-label="选择要设置背景的布局">
      <button
        v-for="layout in backgroundLayouts"
        :key="layout"
        class="background-target"
        :data-layout="layout"
        :aria-pressed="editingLayout === layout"
        :disabled="disabled || busy"
        type="button"
        @click="editingLayout = layout"
      >
        <span
          class="background-preview"
          :class="{ 'has-wallpaper': backgrounds[layout].asset }"
          :style="previewStyle(layout)"
          aria-hidden="true"
        >
          <span class="preview-bar"></span>
          <span class="preview-list"></span>
          <span v-if="layout === 'regular'" class="preview-detail"></span>
        </span>
        {{ labels[layout] }}
      </button>
    </div>
    <div class="background-controls">
      <div class="background-row">
        <div class="background-row-copy">
          <strong>{{ labels[editingLayout] }}背景</strong>
          <small v-if="!background" class="detail-note">PNG、JPEG 或 WebP</small>
        </div>
        <div class="background-actions">
          <button type="button" :disabled="disabled || busy" @click="importBackground">{{
            background ? '替换图片' : '选择图片'
          }}</button>
          <button
            v-if="background || loadError"
            type="button"
            :disabled="disabled || busy"
            @click="removeBackground"
            >移除图片</button
          >
        </div>
      </div>
      <template v-if="background && draft">
        <label class="background-row">
          <span>显示方式</span>
          <select v-model="draft.fit" :disabled="disabled || busy">
            <option value="cover">铺满窗口</option>
            <option value="contain">完整显示</option>
          </select>
        </label>
        <label class="background-row">
          <span>对齐位置</span>
          <select v-model="draft.position" :disabled="disabled || busy">
            <option value="center">居中</option>
            <option value="top">靠上</option>
            <option value="bottom">靠下</option>
            <option value="left">靠左</option>
            <option value="right">靠右</option>
          </select>
        </label>
        <label class="background-row">
          <span>遮罩强度</span>
          <span class="background-strength">
            <input
              v-model.number="draft.scrim"
              type="range"
              :min="scrimRange.min"
              :max="scrimRange.max"
              :disabled="disabled || busy"
              :aria-valuetext="`${draft.scrim}%`"
              :style="{ '--range-progress': scrimProgress }"
            />
            <output aria-hidden="true">{{ draft.scrim }}%</output>
          </span>
        </label>
        <div class="background-row background-apply">
          <button type="button" :disabled="disabled || busy || !changed" @click="applyOptions"
            >应用背景调整</button
          >
        </div>
      </template>
    </div>
    <p v-if="error || loadError" class="error" role="alert">{{ error || loadError }}</p>
    <p v-if="status" role="status">{{ status }}</p>
  </section>
</template>

<style scoped>
.background-settings {
  display: grid;
  gap: 10px;
  margin-block: 16px;
}
.background-settings p {
  margin: 0;
}
.background-targets {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}
.background-settings .background-target {
  display: grid;
  justify-items: center;
  gap: 6px;
  padding: 8px;
  border: 1px solid var(--line);
  border-radius: var(--radius-control);
  background: var(--surface);
  color: var(--text);
}
.background-settings .background-target[aria-pressed='true'] {
  border-color: var(--accent);
  background: var(--select);
}
.background-preview {
  --preview-content-fill: color-mix(in srgb, var(--surface) 64%, transparent);
  display: grid;
  grid-template: 7px 1fr / 5fr 4fr;
  gap: 5px;
  width: 150px;
  aspect-ratio: 10 / 7;
  padding: 6px;
  border-radius: var(--radius-control);
  border: 1px solid var(--line);
  background-color: var(--bg);
}
[data-layout='compact'] .background-preview {
  grid-template-columns: 1fr;
  width: 59px;
  aspect-ratio: 39 / 70;
}
.preview-bar {
  grid-column: 1 / -1;
  border-radius: 2px;
  background: var(--select);
}
.preview-list {
  border-radius: 2px;
  background: repeating-linear-gradient(var(--preview-content-fill) 0 7px, transparent 7px 11px);
}
.preview-detail {
  border-radius: 3px;
  background: var(--preview-content-fill);
}
.background-controls {
  border: 1px solid var(--line);
  border-radius: var(--radius-control);
  background: var(--fluent-card, var(--bg));
}
.background-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex-wrap: wrap;
  gap: 8px 12px;
  padding: 8px 12px;
}
.background-row + .background-row {
  border-top: 1px solid var(--line);
}
.background-row-copy {
  display: grid;
  gap: 2px;
}
.background-row select {
  width: min(175px, 55%);
}
.background-apply {
  justify-content: end;
}
.background-strength,
.background-actions {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 8px;
}
.background-strength {
  width: min(220px, 60%);
  flex-wrap: nowrap;
}
.background-strength input {
  width: 100%;
  min-width: 0;
}
</style>

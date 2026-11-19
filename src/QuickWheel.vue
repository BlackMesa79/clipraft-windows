<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from 'vue';
import PreviewImage from './PreviewImage.vue';
import UiIcon from './UiIcon.vue';
import { filename } from './history';
import type { PanelState } from './panel';
import { slotStyle, wheelGeometry, wheelSlots } from './quickWheel';

const props = defineProps<{ panel: PanelState }>();
const emit = defineEmits<{ thumbnailLoaded: [] }>();
const slots = computed(() => wheelSlots(props.panel));
const settled = ref(true);
const transitionEnabled = ref(false);
const starts = new Map<string, number>();
let previous = new Map<string, number>();
let lastSession = -1;
let lastPosition = 0;
let frame = 0;
const now = ref(Date.now());
const clock = setInterval(() => {
  now.value = Date.now();
}, 60_000);
watch(
  slots,
  (next) => {
    cancelAnimationFrame(frame);
    now.value = Date.now();
    if (!props.panel.visible) previous.clear();
    if (!next.length) return;
    const total = props.panel.total;
    const delta = (props.panel.position - lastPosition + total) % total;
    const step = delta === 1 ? 1 : -1;
    const animate =
      lastSession === props.panel.session && total > 2 && (delta === 1 || delta === total - 1);
    transitionEnabled.value = animate;
    starts.clear();
    if (animate)
      next.forEach((slot) => starts.set(slot.key, previous.get(slot.key) ?? slot.distance + step));
    settled.value = !animate;
    previous = new Map(next.map((slot) => [slot.key, slot.distance]));
    lastSession = props.panel.session;
    lastPosition = props.panel.position;
    // 两帧只建立当前安全视图的起止位置；新状态会取消旧帧，不排队播放。
    frame = requestAnimationFrame(() => {
      frame = requestAnimationFrame(() => {
        settled.value = true;
      });
    });
  },
  { immediate: true },
);
function expiryLabel(expiry: number | null | undefined): string {
  if (!expiry) return '无计时';
  if (expiry <= now.value) return '即将删除';
  return `${Math.ceil((expiry - now.value) / 60_000)} 分钟后删除`;
}
onUnmounted(() => {
  cancelAnimationFrame(frame);
  clearInterval(clock);
});
</script>

<template>
  <section
    class="wheel list"
    role="listbox"
    aria-label="快速粘贴历史"
    :aria-busy="panel.loading"
    :style="{
      '--motion-quick-duration': `${wheelGeometry.duration}ms`,
      '--wheel-card-height': `${wheelGeometry.cardHeight}px`,
    }"
  >
    <article
      v-for="slot in slots"
      :key="slot.key"
      :data-id="slot.item.id"
      :data-slot="slot.distance"
      :class="[
        'wheel-card',
        'item',
        { selected: slot.distance === 0, 'wheel-snap': !settled || !transitionEnabled },
      ]"
      :style="slotStyle(settled ? slot.distance : (starts.get(slot.key) ?? slot.distance))"
      role="option"
      :aria-selected="slot.distance === 0"
      tabindex="-1"
    >
      <span class="index">{{ slot.position }}</span>
      <PreviewImage
        v-if="
          !slot.item.sensitive &&
          (slot.item.kind === 'image' || slot.item.thumbnail || slot.item.placeholder)
        "
        :hash="slot.item.thumbnail"
        :placeholder="slot.item.placeholder"
        symbolic
        @loaded="emit('thumbnailLoaded')"
      />
      <span v-else class="wheel-icon"
        ><UiIcon
          :name="slot.item.sensitive ? 'lock' : slot.item.kind === 'files' ? 'files' : 'text'"
      /></span>
      <div class="content">
        <div class="summary">{{
          slot.item.kind === 'files' && !slot.item.sensitive
            ? slot.item.files.map(filename).join('、')
            : slot.item.summary || '空文本'
        }}</div>
        <div v-if="slot.item.sensitive" class="sensitive-badge"
          >敏感 · {{ expiryLabel(slot.item.expires_at) }}</div
        >
        <div
          v-else-if="slot.item.kind === 'files' && slot.item.file_status === 'missing'"
          class="file-status missing"
          >源文件已不存在</div
        >
        <div
          v-else-if="slot.item.kind === 'files' && slot.item.file_status === 'unknown'"
          class="file-status"
          >无法检查源文件</div
        >
      </div>
    </article>
  </section>
</template>

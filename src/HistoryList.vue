<script setup lang="ts">
import { computed, ref, onMounted, onUnmounted, watch } from 'vue';
const now = ref(Date.now());
const MAX_DATE_MS = 8_640_000_000_000_000;
const recordTime = new Intl.DateTimeFormat('zh-CN', {
  hour: '2-digit',
  minute: '2-digit',
  hourCycle: 'h23',
});
let clock: ReturnType<typeof setInterval> | undefined;
onMounted(() => {
  clock = setInterval(() => {
    now.value = Date.now();
  }, 60_000);
});
onUnmounted(() => clearInterval(clock));
function expiryLabel(expiry: number) {
  return expiry <= now.value
    ? '即将删除'
    : `${Math.ceil((expiry - now.value) / 60_000)} 分钟后删除`;
}

import PreviewImage from './PreviewImage.vue';
import UiIcon from './UiIcon.vue';
import { groupColorStyle } from './groupColors';
import { filename, kindLabel, type Group, type Item } from './history';
const props = defineProps<{
  items: Item[];
  positions?: number[];
  groups?: Group[];
  selected: number;
  selectedIds: Set<string>;
  quick: boolean;
  fluent: boolean;
  filtered?: boolean;
  loading: boolean;
  hasMore: boolean;
  disabled: boolean;
}>();
// 新到期时间必须与当前时刻比较，不能沿用上一次分钟 tick 的旧时刻。
watch(
  () => props.items.map((item) => item.expires_at),
  () => {
    now.value = Date.now();
  },
  { flush: 'sync' },
);
const emit = defineEmits<{
  choose: [index: number, event: MouseEvent];
  context: [index: number, event: MouseEvent];
  paste: [];
  scroll: [];
  thumbnailLoaded: [];
}>();
const element = ref<HTMLElement | null>(null);
const selectionOrder = computed(() => {
  if (props.quick || props.selectedIds.size < 2) return new Map<string, number>();
  return new Map([...props.selectedIds].map((id, index) => [id, index + 1]));
});
defineExpose({ element });
</script>

<template>
  <section
    ref="element"
    class="list"
    @scroll.passive="emit('scroll')"
    aria-label="剪贴板历史"
    role="listbox"
    :aria-multiselectable="!quick"
    :aria-busy="loading || disabled"
  >
    <button
      v-for="(item, index) in items"
      :key="item.id"
      :data-id="item.id"
      type="button"
      :class="['item', { selected: quick ? index === selected : selectedIds.has(item.id) }]"
      role="option"
      :aria-selected="quick ? index === selected : selectedIds.has(item.id)"
      :tabindex="quick ? -1 : 0"
      :disabled="disabled"
      @click="emit('choose', index, $event)"
      @contextmenu.prevent="emit('context', index, $event)"
      @dblclick="emit('paste')"
    >
      <span class="index">{{ positions?.[index] ?? index + 1 }}</span>
      <span
        v-if="
          !quick &&
          (item.sensitive || !(item.kind === 'image' || item.thumbnail || item.placeholder))
        "
        class="type-icon"
        aria-hidden="true"
        ><UiIcon
          :name="
            item.sensitive
              ? 'lock'
              : item.kind === 'files'
                ? 'files'
                : item.kind === 'image'
                  ? 'image'
                  : /^https?:/.test(item.summary)
                    ? 'link'
                    : 'text'
          "
      /></span>
      <PreviewImage
        v-if="
          quick && !item.sensitive && (item.kind === 'image' || item.thumbnail || item.placeholder)
        "
        :hash="item.thumbnail"
        :placeholder="item.placeholder"
        @loaded="emit('thumbnailLoaded')"
      />
      <PreviewImage
        v-if="
          !quick && !item.sensitive && (item.kind === 'image' || item.thumbnail || item.placeholder)
        "
        class="record-thumbnail"
        symbolic
        :hash="item.thumbnail"
        :placeholder="item.placeholder"
        @loaded="emit('thumbnailLoaded')"
      />
      <div class="content"
        ><div class="summary" :title="item.summary">{{
          item.kind === 'files' && !item.sensitive
            ? item.files.map(filename).join('、')
            : item.summary || '空文本'
        }}</div
        ><div class="card-status"
          ><div v-if="!quick && (item.pinned || item.group_id != null)" class="item-badges"
            ><span v-if="item.pinned" class="pin-badge"><UiIcon name="pin" />置顶</span
            ><span
              v-if="item.group_id != null"
              class="group-badge"
              :style="groupColorStyle(item.group_id)"
              >{{ groups?.find((g) => g.id === item.group_id)?.name ?? '分组' }}</span
            ></div
          ><div v-if="!quick && item.source_exe" class="source-name" :title="item.source_exe"
            >来源：{{ item.source_exe }}</div
          ><div v-if="item.sensitive" class="sensitive-badge"
            >敏感<span v-if="item.expires_at"> · {{ expiryLabel(item.expires_at) }}</span></div
          ><div
            v-if="item.kind === 'files' && !item.sensitive && item.file_status === 'missing'"
            class="file-status missing"
            >源文件已不存在</div
          ><div
            v-else-if="item.kind === 'files' && !item.sensitive && item.file_status === 'unknown'"
            class="file-status"
            >无法检查源文件</div
          ></div
        ><div class="meta"
          >{{ kindLabel(item)
          }}<span v-if="item.kind === 'files'">
            · {{ item.files.length }} 个 ·
            {{ item.formats.includes('Preferred DropEffect') ? '保留复制 / 剪切语义' : '' }}</span
          ><span v-else> · {{ item.formats.join(' / ') }}</span></div
        ></div
      >

      <span v-if="!quick && item.pinned" class="compact-pin" title="置顶" aria-label="置顶"
        ><UiIcon name="pin"
      /></span>
      <time
        v-if="fluent && item.created && item.created > 0 && item.created <= MAX_DATE_MS"
        class="record-time"
        >{{ recordTime.format(item.created) }}</time
      >
      <span
        v-if="selectionOrder.has(item.id)"
        class="selected-mark selection-order"
        :aria-label="`粘贴顺序 ${selectionOrder.get(item.id)}`"
        >{{ selectionOrder.get(item.id) }}</span
      >
      <span
        v-else-if="index === selected && (!fluent || selectedIds.size > 1)"
        class="selected-mark"
        >↵</span
      >
    </button>
    <p v-if="!quick" class="detail-note" role="status">{{
      loading
        ? '正在加载…'
        : hasMore
          ? '向下滚动加载更多'
          : filtered
            ? '已加载全部匹配记录'
            : '已加载全部历史'
    }}</p>
  </section>
</template>

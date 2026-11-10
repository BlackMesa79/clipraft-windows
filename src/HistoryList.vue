<script setup lang="ts">
import { ref } from 'vue';
import PreviewImage from './PreviewImage.vue';
import { filename, kindLabel, type Item } from './history';
defineProps<{ items: Item[]; selected: number; selectedIds: Set<string>; quick: boolean; loading: boolean; hasMore: boolean; disabled: boolean }>();
const emit = defineEmits<{
  choose: [index: number, event: MouseEvent];
  context: [index: number, event: MouseEvent];
  paste: [];
  scroll: [];
  thumbnailLoaded: [];
}>();
const element = ref<HTMLElement | null>(null);
defineExpose({ element });
</script>

<template>
  <section ref="element" class="list" @scroll.passive="emit('scroll')" aria-label="剪贴板历史" role="listbox" :aria-multiselectable="!quick" :aria-busy="loading || disabled">
    <button v-for="(item, index) in items" :key="item.id" type="button" :class="['item', { selected: quick ? index === selected : selectedIds.has(item.id) }]" role="option" :aria-selected="quick ? index === selected : selectedIds.has(item.id)" :tabindex="quick ? -1 : 0" :disabled="disabled" @click="emit('choose', index, $event)" @contextmenu.prevent="emit('context', index, $event)" @dblclick="emit('paste')">
      <span class="index">{{ index + 1 }}</span>
      <PreviewImage v-if="item.kind === 'image' || item.thumbnail || item.placeholder" :hash="item.thumbnail" :placeholder="item.placeholder" @loaded="emit('thumbnailLoaded')" />
      <div class="content"><div class="summary">{{ item.kind === 'files' ? item.files.map(filename).join('、') : item.summary || '空文本' }}</div><div class="meta">{{ kindLabel(item) }}<span v-if="item.kind === 'files'"> · {{ item.files.length }} 个 · {{ item.formats.includes('Preferred DropEffect') ? '保留复制 / 剪切语义' : '' }}</span><span v-else> · {{ item.formats.join(' / ') }}</span></div></div>
      <span v-if="index === selected" class="selected-mark">↵</span>
    </button>
    <p v-if="!quick" class="detail-note" role="status">{{ loading ? '正在加载…' : hasMore ? '向下滚动加载更多' : '已加载全部历史' }}</p>
  </section>
</template>

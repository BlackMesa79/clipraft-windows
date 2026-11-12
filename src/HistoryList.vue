<script setup lang="ts">
import { ref, onMounted, onUnmounted } from 'vue';
const now = ref(Date.now());
let clock: ReturnType<typeof setInterval> | undefined;
onMounted(() => { clock = setInterval(() => { now.value = Date.now(); }, 60_000); });
onUnmounted(() => clearInterval(clock));
function expiryLabel(expiry: number) { return expiry <= now.value ? '即将删除' : `${Math.ceil((expiry - now.value) / 60_000)} 分钟后删除`; }

import PreviewImage from './PreviewImage.vue';
import { filename, kindLabel, type Group, type Item } from './history';
defineProps<{ items: Item[]; groups?: Group[]; selected: number; selectedIds: Set<string>; quick: boolean; filtered?: boolean; loading: boolean; hasMore: boolean; disabled: boolean }>();
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
      <div class="content"><div class="summary">{{ item.kind === 'files' && !item.sensitive ? item.files.map(filename).join('、') : item.summary || '空文本' }}</div><div v-if="!quick && (item.pinned || item.favorite || item.group_id != null)" class="item-badges"><span v-if="item.pinned">📌 置顶</span><span v-if="item.favorite">★ 收藏</span><span v-if="item.group_id != null" class="group-badge">{{ groups?.find(g => g.id === item.group_id)?.name ?? '分组' }}</span></div><div v-if="!quick && item.source_exe" class="source-name" :title="item.source_exe">来源：{{ item.source_exe }}</div><div v-if="item.sensitive" class="sensitive-badge">敏感<span v-if="item.expires_at"> · {{ expiryLabel(item.expires_at) }}</span></div><div v-if="item.kind === 'files' && !item.sensitive && item.file_status === 'missing'" class="file-status missing">源文件已不存在</div><div v-else-if="item.kind === 'files' && !item.sensitive && item.file_status === 'unknown'" class="file-status">无法检查源文件</div><div class="meta">{{ kindLabel(item) }}<span v-if="item.kind === 'files'"> · {{ item.files.length }} 个 · {{ item.formats.includes('Preferred DropEffect') ? '保留复制 / 剪切语义' : '' }}</span><span v-else> · {{ item.formats.join(' / ') }}</span></div></div>
      <span v-if="index === selected" class="selected-mark">↵</span>
    </button>
    <p v-if="!quick" class="detail-note" role="status">{{ loading ? '正在加载…' : hasMore ? '向下滚动加载更多' : filtered ? '已加载全部匹配记录' : '已加载全部历史' }}</p>
  </section>
</template>

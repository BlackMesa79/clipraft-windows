<script setup lang="ts">
import { ref, watch } from 'vue';
import type { RichPreview } from './richPreview';
const props = defineProps<{ content: RichPreview }>();
const emit = defineEmits<{ loaded: [] }>();
const failed = ref(new Set<number>());
watch(
  () => props.content,
  () => {
    failed.value = new Set();
  },
);
function imageFailed(index: number): void {
  failed.value = new Set([...failed.value, index]);
  emit('loaded');
}
function safeImage(source: string | null): string | undefined {
  // 预览只接受后端重新编码的位图，绝不把资源原地址交给浏览器。
  return source && /^data:image\/(png|jpeg);base64,[A-Za-z0-9+/]+={0,2}$/.test(source)
    ? source
    : undefined;
}
</script>
<template>
  <div class="rich-preview" aria-label="图文内容预览">
    <template v-for="(block, index) in content.blocks" :key="index">
      <p v-if="block.kind === 'text'" class="rich-text">{{ block.text }}</p>
      <figure v-else class="rich-picture">
        <img
          v-if="safeImage(block.src) && !failed.has(index)"
          :src="safeImage(block.src)"
          alt="图文中的图片"
          @load="emit('loaded')"
          @error="imageFailed(index)"
        />
        <figcaption v-else role="status">{{ block.message || '图片预览无法显示' }}</figcaption>
      </figure>
    </template>
    <p v-if="content.truncated" class="rich-note"
      >内容较长，仅显示部分图文预览；原始粘贴内容未改变。</p
    >
    <p v-if="content.linked_images" class="rich-note"
      >图片来自原记录引用的本地文件；原文件移动或删除后可能无法预览。</p
    >
  </div>
</template>
<style scoped>
.rich-preview {
  display: flex;
  flex-direction: column;
  gap: 12px;
  min-width: 0;
}
.rich-text {
  margin: 0;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  line-height: 1.65;
  font-size: var(--font-body-size);
}
.rich-picture {
  margin: 0;
  min-width: 0;
}
.rich-picture img {
  display: block;
  max-width: 100%;
  height: auto;
  border-radius: var(--radius-media);
}
.rich-picture figcaption {
  padding: 12px;
  border: 1px dashed var(--line);
  border-radius: var(--radius-placeholder);
  color: var(--muted);
  font-size: var(--font-label-size);
  overflow-wrap: anywhere;
}
.rich-note {
  margin: 0;
  color: var(--muted);
  font-size: var(--font-note-size);
  line-height: 1.6;
}
</style>

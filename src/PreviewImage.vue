<script setup lang="ts">
import { ref, watch } from 'vue';
import { convertFileSrc, invoke } from '@tauri-apps/api/core';
const props = defineProps<{ hash?: string | null; placeholder?: string | null; large?: boolean }>();
const emit = defineEmits<{ loaded: [] }>();
const failure = ref('');
let generation = 0;
watch(() => props.hash, () => { generation++; failure.value = ''; });
async function failed() {
  const current = ++generation;
  failure.value = '图片加载失败，正在检查对象…';
  try {
    const reason = await invoke<string>('image_failure_reason', { id: props.hash });
    if (current === generation) failure.value = reason;
  } catch { if (current === generation) failure.value = '图片加载失败，无法读取后端诊断信息'; }
}
</script>
<template>
  <div :class="large ? 'large-preview' : 'preview'">
    <img v-if="hash && !failure" :key="hash" :src="convertFileSrc(hash, 'thumb')" :alt="large ? '选中项图片预览' : '图片缩略图'" @load="emit('loaded')" @error="failed" />
    <span v-else class="image-placeholder" role="status">{{ failure || placeholder || '记录没有可用的图片预览' }}</span>
  </div>
</template>

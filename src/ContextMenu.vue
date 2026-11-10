<script setup lang="ts">
import { nextTick, onMounted, onUnmounted, ref } from 'vue';
const props = defineProps<{ x: number; y: number }>();
const emit = defineEmits<{ delete: []; paste: []; close: [] }>();
const root = ref<HTMLElement | null>(null), left = ref(props.x), top = ref(props.y);
function outside(event: PointerEvent) { if (!root.value?.contains(event.target as Node)) emit('close'); }
function close() { emit('close'); }
onMounted(async () => {
  await nextTick();
  if (root.value) {
    const box = root.value.getBoundingClientRect();
    left.value = Math.max(4, Math.min(props.x, window.innerWidth - box.width - 4));
    top.value = Math.max(4, Math.min(props.y, window.innerHeight - box.height - 4));
  }
  window.addEventListener('pointerdown', outside);
  window.addEventListener('blur', close);
  window.addEventListener('resize', close);
});
onUnmounted(() => { window.removeEventListener('pointerdown', outside); window.removeEventListener('blur', close); window.removeEventListener('resize', close); });
</script>

<template>
  <div ref="root" class="context-menu" role="menu" aria-label="记录操作" :style="{ left: left + 'px', top: top + 'px' }" @contextmenu.prevent>
    <button type="button" role="menuitem" @click="emit('delete')">删除</button>
    <button type="button" role="menuitem" @click="emit('paste')">粘贴</button>
  </div>
</template>

<script setup lang="ts">
import { onMounted, onUnmounted, ref } from 'vue';
import { containDialogFocus } from './dialogFocus';
import { directoryLabel } from './directoryDisplay';
const props = defineProps<{
  preview: { root: string; files: number; bytes: number };
  busy: boolean;
  error: string;
}>();
const emit = defineEmits<{ confirm: []; cancel: [] }>();
const element = ref<HTMLDialogElement>();
const previousFocus = document.activeElement;
function cancel(): void {
  if (!props.busy) emit('cancel');
}
onMounted(() => {
  element.value?.showModal();
  element.value?.querySelector<HTMLButtonElement>('[data-cancel-staging]')?.focus();
});
onUnmounted(() => {
  element.value?.close();
  if (previousFocus instanceof HTMLElement && previousFocus.isConnected) previousFocus.focus();
});
</script>
<template>
  <Teleport to="body">
    <dialog
      ref="element"
      class="staging-cleanup-dialog"
      aria-labelledby="staging-cleanup-title"
      @cancel.prevent="cancel"
      @keydown.stop="containDialogFocus(element, $event)"
    >
      <h2 id="staging-cleanup-title">确认清理中断暂存</h2>
      <p
        >将清理经检查的 {{ preview.files }} 个临时文件（{{
          preview.bytes.toLocaleString()
        }}
        字节）：</p
      >
      <p class="staging-path">{{ directoryLabel(preview.root) }}</p>
      <p
        >这些是导出或恢复中断后留下的工作文件。确认后不可撤销；不会删除正式归档、当前数据库或迁移备份。</p
      >
      <p>执行时会再次核对文件身份和内容，变化或未知文件保留。</p>
      <p v-if="error" role="alert" class="error">{{ error }}</p>
      <p v-if="busy" role="status">正在清理，请稍候…</p>
      <footer
        ><button type="button" data-cancel-staging :disabled="busy" autofocus @click="cancel"
          >取消清理</button
        ><button type="button" data-confirm-staging :disabled="busy" @click="emit('confirm')"
          >确认清理暂存</button
        ></footer
      >
    </dialog>
  </Teleport>
</template>
<style scoped>
.staging-cleanup-dialog {
  width: min(480px, calc(100vw - 64px));
  max-height: calc(100vh - 64px);
  overflow-y: auto;
  padding: var(--dialog-padding);
  border: 1px solid var(--line);
  border-radius: var(--radius-dialog);
  background: var(--bg);
  color: var(--text);
  box-shadow: var(--shadow-dialog);
}
.staging-cleanup-dialog:focus {
  outline: none;
}
.staging-cleanup-dialog::backdrop {
  background: var(--dialog-backdrop);
}
h2 {
  margin: 0 0 16px;
  font-size: var(--font-heading-size);
}
p {
  font-size: var(--font-body-size);
  line-height: 1.6;
}
.staging-path {
  overflow-wrap: anywhere;
  user-select: text;
}
footer {
  display: flex;
  justify-content: flex-end;
  gap: var(--gap-actions);
  margin-top: 24px;
}
button {
  padding: 8px 14px;
  border: 1px solid var(--line);
  border-radius: var(--radius-control);
  background: var(--bg);
  color: var(--text);
  cursor: pointer;
}
button[data-confirm-staging] {
  background: var(--primary-bg);
  color: var(--primary-text);
}
button:disabled {
  opacity: var(--opacity-form-disabled);
  cursor: default;
}
</style>

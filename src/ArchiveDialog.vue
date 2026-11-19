<script setup lang="ts">
import { onMounted, onUnmounted, ref } from 'vue';
import { containDialogFocus } from './dialogFocus';
const props = defineProps<{
  mode: 'export' | 'restore' | 'replace';
  busy: boolean;
  error: string;
}>();
const emit = defineEmits<{ confirm: [password: string]; cancel: [] }>();
const element = ref<HTMLDialogElement>();
const password = ref('');
const confirmation = ref('');
const acknowledged = ref(false);
const issue = ref('');
const previousFocus = document.activeElement;
function submit(): void {
  if (props.busy) return;
  issue.value = '';
  if (props.mode === 'replace') {
    if (acknowledged.value) emit('confirm', '');
    return;
  }
  const bytes = new TextEncoder().encode(password.value).length;
  if (!bytes || bytes > 1024) {
    issue.value = '密码长度必须为 1 到 1024 UTF-8 字节';
    return;
  }
  if (props.mode === 'export' && password.value !== confirmation.value) {
    issue.value = '两次输入的密码不一致';
    return;
  }
  emit('confirm', password.value);
  password.value = '';
  confirmation.value = '';
}
function cancel(): void {
  if (!props.busy) emit('cancel');
}
onMounted(() => {
  element.value?.showModal();
});
onUnmounted(() => {
  password.value = '';
  confirmation.value = '';
  element.value?.close();
  if (previousFocus instanceof HTMLElement && previousFocus.isConnected) previousFocus.focus();
});
</script>

<template>
  <Teleport to="body">
    <dialog
      ref="element"
      class="archive-dialog"
      aria-labelledby="archive-dialog-title"
      @cancel.prevent="cancel"
      @keydown.stop="containDialogFocus(element, $event)"
    >
      <form @submit.prevent="submit">
        <h2 id="archive-dialog-title">{{
          { export: '设置备份密码', restore: '输入归档密码', replace: '替换当前数据并重启' }[mode]
        }}</h2>
        <template v-if="mode === 'replace'">
          <p
            >归档已解密并验证通过。确认后将重启应用，在当前数据目录替换历史、分组、收藏与设置；数据目录位置不变。</p
          >
          <p
            >不会合并两份历史。替换前会将旧数据保存在独立备份目录，备份不随新库同步，可能仍含敏感记录。</p
          >
          <label class="archive-ack"
            ><input
              v-model="acknowledged"
              data-replace-ack
              type="checkbox"
              :disabled="busy"
            />我确认使用导入数据替换当前历史</label
          >
        </template>
        <template v-else>
          <p>{{
            mode === 'export'
              ? '请为备份设置密码并妥善保存；忘记密码将无法恢复归档。'
              : '输入导出时设置的密码。验证成功后可选择替换当前数据，原库会保留。'
          }}</p>
          <label
            >密码<input
              v-model="password"
              data-archive-password
              type="password"
              :autocomplete="mode === 'export' ? 'new-password' : 'current-password'"
              :disabled="busy"
              autofocus
          /></label>
          <label v-if="mode === 'export'"
            >再次输入密码<input
              v-model="confirmation"
              data-archive-confirmation
              type="password"
              autocomplete="new-password"
              :disabled="busy"
          /></label>
        </template>
        <p v-if="issue || error" role="alert" class="archive-dialog-error">{{ issue || error }}</p>
        <p v-if="busy" role="status">正在重新验证并提交切换，请稍候…</p>
        <footer
          ><button type="button" data-dismiss-archive-dialog :disabled="busy" @click="cancel">{{
            mode === 'replace' ? '暂不替换' : '取消'
          }}</button
          ><button
            type="submit"
            data-confirm-archive-dialog
            :disabled="busy || (mode === 'replace' && !acknowledged)"
            >{{
              { export: '确认并导出', restore: '解密并导入', replace: '确认替换并重启' }[mode]
            }}</button
          ></footer
        >
      </form>
    </dialog>
  </Teleport>
</template>

<style scoped>
.archive-dialog {
  width: min(440px, calc(100vw - 64px));
  max-height: calc(100vh - 64px);
  overflow-y: auto;
  padding: var(--dialog-padding);
  border: 1px solid var(--line);
  border-radius: var(--radius-dialog);
  background: var(--control-bg);
  color: var(--control-text);
  box-shadow: var(--shadow-dialog);
}
.archive-dialog::backdrop {
  background: var(--dialog-backdrop);
}
h2 {
  margin: 0 0 12px;
  font-size: var(--font-heading-size);
}
p {
  font-size: var(--font-body-size);
  line-height: 1.6;
}
label {
  display: flex;
  flex-direction: column;
  gap: 7px;
  margin: 16px 0;
  font-size: var(--font-subtitle-size);
}
input[type='password'] {
  width: 100%;
  box-sizing: border-box;
  padding: 9px 11px;
  border: 1px solid var(--line);
  border-radius: var(--radius-control);
  background: var(--control-bg);
  color: inherit;
}
.archive-ack {
  flex-direction: row;
  align-items: flex-start;
}
.archive-ack input {
  width: auto;
  margin-top: 3px;
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
  background: var(--control-bg);
  color: inherit;
  cursor: pointer;
}
button[type='submit'] {
  background: var(--primary-bg);
  color: var(--primary-text);
}
button:disabled {
  opacity: var(--opacity-form-disabled);
  cursor: default;
}
.archive-dialog-error {
  color: var(--error-text);
  overflow-wrap: anywhere;
}
</style>

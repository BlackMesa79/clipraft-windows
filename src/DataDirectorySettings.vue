<script setup lang="ts">
import { onMounted, onUnmounted, ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { directoryLabel } from './directoryDisplay';
const props = defineProps<{ disabled: boolean; unsaved: boolean }>();
const emit = defineEmits<{ busy: [active: boolean] }>();
interface DirectoryStatus {
  current: string;
  default: string;
  custom: boolean;
  enabled: boolean;
  previous: string | null;
  displaced?: string | null;
  cleanup?: {
    backup: string;
    source: string;
    initialized: boolean;
    complete: boolean;
    remaining: number;
    error: string | null;
  } | null;
}
interface MigrationPlan {
  backup?: string;
  ticket: string;
  source: string;
  target: string;
  bytes: number;
  displaced?: string | null;
}
const status = ref<DirectoryStatus>();
const plan = ref<MigrationPlan>();
const error = ref('');
const working = ref(false);
const restarting = ref(false);
function setBusy(active: boolean): void {
  working.value = active;
  emit('busy', active);
}
async function prepare(command: 'choose_data_directory' | 'restore_data_directory'): Promise<void> {
  if (props.disabled || working.value || !status.value?.enabled) return;
  error.value = '';
  plan.value = undefined;
  setBusy(true);
  try {
    plan.value = (await invoke<MigrationPlan | null>(command)) ?? undefined;
  } catch (reason) {
    error.value = String(reason);
  } finally {
    setBusy(false);
  }
}
function changeDirectory(): void {
  void prepare('choose_data_directory');
}
function restoreDirectory(): void {
  void prepare('restore_data_directory');
}
async function migrate(): Promise<void> {
  if (!plan.value || working.value || props.disabled || props.unsaved) return;
  error.value = '';
  setBusy(true);
  try {
    await invoke('migrate_data_directory', { ticket: plan.value.ticket });
    restarting.value = true;
  } catch (reason) {
    error.value = String(reason);
    setBusy(false);
  }
}
let disposed = false;
let timer: ReturnType<typeof setTimeout> | undefined;
async function refresh(): Promise<void> {
  try {
    status.value = await invoke<DirectoryStatus>('data_directory_status');
  } catch (reason) {
    if (!disposed) error.value = '读取数据目录失败：' + String(reason);
  }
  if (!disposed)
    timer = setTimeout(() => {
      void refresh();
    }, 1500);
}
async function retryCleanup(): Promise<void> {
  if (props.disabled || working.value) return;
  setBusy(true);
  error.value = '';
  try {
    await invoke('retry_migration_cleanup');
  } catch (reason) {
    error.value = String(reason);
  } finally {
    setBusy(false);
    clearTimeout(timer);
    await refresh();
  }
}
onMounted(() => {
  void refresh();
});
onUnmounted(() => {
  disposed = true;
  clearTimeout(timer);
});
</script>

<template>
  <section class="data-directory-settings" aria-label="数据存储" :aria-busy="working">
    <template v-if="status">
      <p>{{ status.custom ? '自定义数据目录' : '默认数据目录' }}</p>
      <p class="directory-path" data-current-directory>{{ directoryLabel(status.current) }}</p>
      <p class="detail-note"
        >历史记录、分组、收藏与设置保存在此目录。迁移前会在源目录外保留完整加密备份，切换并成功初始化后按清单清理旧源文件，请勿手工清空目录。</p
      >
      <p v-if="!status.enabled" class="detail-note">当前为隔离测试环境，已禁用数据目录更改。</p>
      <div class="directory-actions">
        <button
          type="button"
          :disabled="disabled || working || !status.enabled"
          @click="changeDirectory"
          >更改目录</button
        >
        <button
          type="button"
          :disabled="disabled || working || !status.enabled || !status.custom"
          @click="restoreDirectory"
          >恢复默认位置</button
        >
      </div>
      <template
        v-if="status.previous && (!status.cleanup || status.previous !== status.cleanup.source)"
      >
        <p class="detail-note">上次切换保留副本（不随新数据同步，可能仍含已删除的敏感记录）：</p>
        <p class="directory-path" data-previous-directory>{{ directoryLabel(status.previous) }}</p>
      </template>
      <div v-if="status.cleanup" class="directory-confirm" data-migration-cleanup>
        <p>迁移前完整备份（使用原账户密钥，保留后不会与新库同步）：</p>
        <p class="directory-path">{{ directoryLabel(status.cleanup.backup) }}</p>
        <p
          >旧源位置：<span class="directory-path">{{
            directoryLabel(status.cleanup.source)
          }}</span></p
        >
        <p role="status">{{
          status.cleanup.complete
            ? '迁移成功，源清单文件已清理；日志、缓存和额外文件保留。'
            : '迁移成功，旧文件待清理；当前继续使用新库。'
        }}</p>
        <p v-if="status.cleanup.error" class="error">{{ status.cleanup.error }}</p>
        <button
          v-if="!status.cleanup.complete"
          type="button"
          data-retry-migration-cleanup
          :disabled="disabled || working"
          @click="retryCleanup"
          >重试源文件清理</button
        >
        <p class="detail-note"
          >备份仍含完整旧历史及可能的敏感记录，请妥善保管。只有身份和内容未变的源文件会被清理。</p
        >
      </div>
      <template v-if="status.displaced">
        <p class="detail-note">恢复默认时保留的旧数据（未与当前历史合并）：</p>
        <p class="directory-path" data-displaced-directory>{{
          directoryLabel(status.displaced)
        }}</p>
      </template>
    </template>
    <p v-else-if="!error" role="status">正在读取数据目录…</p>
    <div v-if="plan" class="directory-confirm" role="group" aria-label="确认数据迁移">
      <template v-if="plan.backup"
        ><p>迁移前备份位置</p
        ><p class="directory-path" data-migration-backup>{{ directoryLabel(plan.backup) }}</p
        ><p class="detail-note"
          >验证备份和目标成功、首次初始化完成后，清理源管理文件；备份继续保留。失败清理可重试，不回滚新库。</p
        ></template
      >
      <p>源目录</p
      ><p class="directory-path" data-source-directory>{{ directoryLabel(plan.source) }}</p>
      <p>目标目录</p
      ><p class="directory-path" data-target-directory>{{ directoryLabel(plan.target) }}</p>
      <template v-if="plan.displaced">
        <p class="detail-note"
          >默认目录已有旧数据，将先移至以下位置保留，再迁入当前数据；不会合并或删除旧记录，无需手工清空目录。</p
        >
        <p class="directory-path" data-planned-backup>{{ directoryLabel(plan.displaced) }}</p>
      </template>
      <p class="detail-note"
        >预计复制 {{ (plan.bytes / 1048576).toFixed(2) }} MB，以重启后的实际数据为准。</p
      >
      <p class="detail-note"
        >迁移期间暂停使用。原目录保留副本，不自动删除；文件记录引用的源文件不会搬迁。请先保存设置页中尚未保存的更改。</p
      >
      <p v-if="unsaved" role="status" class="detail-note"
        >设置尚未保存，请先保存或取消其他设置更改，再执行迁移。</p
      >
      <div class="directory-actions">
        <button type="button" :disabled="disabled || working || unsaved" @click="migrate"
          >迁移并重启</button
        >
        <button type="button" :disabled="disabled || working" @click="plan = undefined"
          >取消迁移</button
        >
      </div>
    </div>
    <p v-if="restarting" role="status">正在重启，随后复制并验证数据。请勿关闭迁移进度窗口。</p>
    <p v-if="error" role="alert" class="error">{{ error }}</p>
  </section>
</template>

<style scoped>
.directory-path {
  overflow-wrap: anywhere;
  user-select: text;
  font-size: var(--font-body-size);
}
.directory-actions {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}
.directory-confirm {
  margin-top: 16px;
  padding-top: 12px;
  border-top: 1px solid var(--line);
}
</style>

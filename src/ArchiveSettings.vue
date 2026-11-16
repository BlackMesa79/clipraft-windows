<script setup lang="ts">
import { directoryLabel } from './directoryDisplay';
import { useArchiveTask } from './archiveTask';
import ArchiveDialog from './ArchiveDialog.vue';
const props = defineProps<{ disabled: boolean; unsaved: boolean }>();
const emit = defineEmits<{ busy: [active: boolean] }>();
const task = useArchiveTask((active) => emit('busy', active));
const {
  mode,
  restoreTarget,
  destination,
  archive,
  dialog,
  working,
  restarting,
  report,
  error,
  pollingError,
  occupied,
  phase,
  percent,
  cancellable,
} = task;
function start(): void {
  if (!props.disabled && !props.unsaved) task.openPassword();
}
function confirm(password: string): void {
  if (props.disabled || props.unsaved) return;
  if (dialog.value === 'replace') {
    void task.replaceCurrent();
    return;
  }
  void task.start(password);
}
function openReplacement(): void {
  if (!props.disabled && !props.unsaved && !occupied.value) {
    error.value = '';
    dialog.value = 'replace';
  }
}
</script>

<template>
  <section class="archive-settings" aria-label="密码备份与恢复" @keydown.enter.prevent.stop="start">
    <h3>密码备份与恢复</h3>
    <p class="detail-note"
      >完整历史、分组、收藏及可迁移设置会进入密码归档。文件记录只保存引用，不包含原始文件。</p
    >
    <p class="detail-note">忘记密码无法恢复归档。创建快照期间暂停记录复制，随后在后台继续导出。</p>
    <div class="archive-actions">
      <button
        type="button"
        :aria-pressed="mode === 'export'"
        :disabled="disabled || occupied"
        @click="mode = 'export'"
        >导出备份</button
      >
      <button
        type="button"
        :aria-pressed="mode === 'restore'"
        :disabled="disabled || occupied"
        @click="mode = 'restore'"
        >恢复归档</button
      >
    </div>
    <label v-if="mode === 'restore'" class="archive-field"
      >恢复方式
      <select v-model="restoreTarget" data-restore-target :disabled="disabled || occupied">
        <option value="current">恢复并替换当前数据（目录保持不变）</option>
        <option value="other">恢复到其他目录</option>
      </select>
    </label>
    <label v-if="mode === 'restore'" class="archive-field"
      >归档文件
      <span
        ><input
          v-model="archive"
          data-archive-source
          :disabled="disabled || occupied"
          placeholder="选择密码归档文件"
        /><button type="button" :disabled="disabled || occupied" @click="task.choose('archive')"
          >选择文件</button
        ></span
      >
    </label>
    <label v-if="mode === 'export' || restoreTarget === 'other'" class="archive-field"
      >{{ mode === 'export' ? '导出文件路径（不能已存在）' : '恢复到新目录（不能已存在）' }}
      <span
        ><input
          v-model="destination"
          data-archive-destination
          :disabled="disabled || occupied"
          placeholder="选择父文件夹后可修改名称"
        /><button type="button" :disabled="disabled || occupied" @click="task.choose('destination')"
          >选择父文件夹</button
        ></span
      >
    </label>
    <p v-if="mode === 'restore'" class="detail-note">{{
      restoreTarget === 'current'
        ? '程序自动暂存并验证归档。确认后重启，在原数据目录替换历史，旧库保存在独立备份目录，不合并历史。'
        : '恢复到所选的新目录，不修改当前历史，也不自动切换当前数据目录。'
    }}</p>
    <p v-if="unsaved" class="detail-note">请先保存或取消其他设置更改，再开始归档任务。</p>
    <div class="archive-actions">
      <button
        type="button"
        data-start-archive
        :disabled="disabled || unsaved || occupied"
        @click="start"
        >{{ mode === 'export' ? '开始导出' : '开始恢复' }}</button
      >
      <button
        v-if="report?.running"
        type="button"
        data-cancel-archive
        :disabled="!cancellable"
        @click="task.cancel"
        >取消任务</button
      >
    </div>
    <div v-if="report" class="archive-result" role="status" aria-live="polite">
      <p
        >{{ report.kind === 'export' ? '导出' : '恢复' }}：{{ phase
        }}{{ percent !== null && report.running ? `（写入 ${percent}%）` : '' }}</p
      >
      <p v-if="report.running" class="detail-note"
        >可以关闭设置页，任务会继续；取消后需等待清理结束，请勿退出应用或关机。</p
      >
      <template v-if="report.outcome?.state === 'succeeded'">
        <p v-if="report.outcome.replace_current"
          >归档已验证，可以确认替换当前数据；当前数据目录保持不变。</p
        >
        <p v-else-if="report.outcome.restored_root"
          >新库已验证并恢复：<span class="archive-path">{{
            directoryLabel(report.outcome.restored_root)
          }}</span
          >。当前数据库尚未切换。</p
        >
        <p v-else>备份已完成。</p>
        <button
          v-if="report.outcome.replace_current"
          type="button"
          data-replace-current
          :disabled="disabled || unsaved || occupied"
          @click="openReplacement"
          >使用导入数据替换当前数据</button
        >
        <p v-if="report.outcome.cleanup_warning" class="error">{{
          report.outcome.cleanup_warning
        }}</p>
      </template>
      <p v-else-if="report.outcome" class="archive-path">{{ report.outcome.message }}</p>
    </div>
    <p v-if="restarting" role="status"
      >已安排原目录替换，正在重启。旧数据将在替换前保存到独立备份目录。</p
    >
    <p v-if="error && !dialog" role="alert" class="error">{{ error }}</p>
    <p v-if="pollingError" role="alert" class="error"
      >无法查询任务状态：{{ pollingError }}。将在后台重试。</p
    >
    <ArchiveDialog
      v-if="dialog"
      :mode="dialog"
      :busy="working"
      :error="error"
      @confirm="confirm"
      @cancel="dialog = null"
    />
  </section>
</template>

<style scoped>
.archive-settings {
  border-top: 1px solid var(--line);
  margin-top: 18px;
  padding-top: 12px;
}
h3 {
  font-size: 15px;
  margin: 0 0 10px;
}
.archive-actions {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  margin: 10px 0;
}
.archive-field {
  display: flex;
  flex-direction: column;
  align-items: stretch;
  gap: 6px;
  margin: 12px 0;
}
.archive-field > span {
  display: flex;
  gap: 8px;
}
.archive-field input {
  min-width: 0;
  width: 100%;
  box-sizing: border-box;
}
.archive-field button {
  flex-shrink: 0;
}
.archive-path {
  overflow-wrap: anywhere;
  user-select: text;
}
.archive-result {
  margin-top: 12px;
}
</style>

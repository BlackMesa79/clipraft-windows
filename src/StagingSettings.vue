<script setup lang="ts">
import { directoryLabel } from './directoryDisplay';
import StagingCleanupDialog from './StagingCleanupDialog.vue';
import { stagingLabel, useStagingCleanup } from './stagingCleanup';
const props = defineProps<{ disabled: boolean }>();
const emit = defineEmits<{ busy: [active: boolean] }>();
const task = useStagingCleanup((active) => emit('busy', active));
const { parent, report, preview, busy, error, message } = task;
function prepare(root: string): void {
  if (!props.disabled) void task.prepare(root);
}
</script>
<template>
  <section class="staging-settings" aria-label="中断暂存管理">
    <h3>中断暂存管理</h3>
    <p class="detail-note"
      >这里管理导出或恢复中断后遗留的临时工作文件，不是自动备份目录。选择当时使用的父文件夹进行检查。只处理本程序可识别的暂存目录；运行中的任务、旧版无标记目录和未知文件会保留。清理不删除正式归档、当前数据库或迁移备份。</p
    >
    <div class="staging-actions"
      ><button type="button" :disabled="disabled || busy" data-scan-staging @click="task.choose"
        >选择文件夹并扫描</button
      ><button v-if="parent" type="button" :disabled="disabled || busy" @click="task.scan"
        >重新扫描</button
      ></div
    >
    <p v-if="parent" class="staging-path">{{ directoryLabel(parent) }}</p>
    <p v-if="report && !report.candidates.length" role="status">没有发现可识别前缀的暂存目录。</p>
    <p v-if="report?.truncated" class="detail-note"
      >扫描达到上限，结果不完整，请选择更具体的父文件夹。</p
    >
    <ul v-if="report?.candidates.length"
      ><li v-for="candidate in report.candidates" :key="candidate.root"
        ><p class="staging-path">{{ directoryLabel(candidate.root) }}</p
        ><p>{{ stagingLabel(candidate.status) }}</p
        ><p v-if="candidate.status.state === 'unrecognized'" class="detail-note">{{
          candidate.status.reason
        }}</p
        ><button
          v-else
          type="button"
          :disabled="disabled || busy || candidate.status.state === 'active'"
          data-review-staging
          @click="prepare(candidate.root)"
          >预检清理</button
        ></li
      ></ul
    >
    <StagingCleanupDialog
      v-if="preview"
      :preview="preview"
      :busy="busy"
      :error="error"
      @confirm="task.confirm"
      @cancel="task.cancel"
    />
    <p v-if="busy" role="status">正在检查或清理，请稍候…</p
    ><p v-if="message" role="status">{{ message }}</p
    ><p v-if="error && !preview" role="alert" class="error">{{ error }}</p>
  </section>
</template>
<style scoped>
.staging-settings {
  border-top: 1px solid var(--line);
  padding-top: 12px;
  margin-top: 18px;
}
h3 {
  font-size: 15px;
}
.staging-actions {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}
.staging-path {
  overflow-wrap: anywhere;
  user-select: text;
  font-size: 13px;
}
ul {
  padding-left: 18px;
}
li {
  padding-bottom: 12px;
}
</style>

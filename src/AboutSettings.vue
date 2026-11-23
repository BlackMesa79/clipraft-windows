<script setup lang="ts">
import { ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { brandName } from './brand';
import type { Updates } from './updates';

const { updates } = defineProps<{ updates: Updates }>();
const { version, updateState, check } = updates;
const launchError = ref('');
const opening = ref(false);

async function openRelease(destination: 'latest' | 'history') {
  if (opening.value) return;
  opening.value = true;
  launchError.value = '';
  try {
    await invoke('open_release_page', { destination });
  } catch (error) {
    launchError.value = String(error);
  } finally {
    opening.value = false;
  }
}
</script>

<template>
  <div class="about-release-card">
    <div class="about-version">
      <strong>{{ brandName }}</strong>
      <span>当前版本 {{ version || '—' }}</span>
    </div>
    <button
      class="about-check"
      type="button"
      :disabled="updateState.kind === 'checking'"
      @click="check"
    >
      {{ updateState.kind === 'checking' ? '正在检查…' : '检查更新' }}
    </button>
  </div>
  <slot />
  <div v-if="updateState.kind !== 'idle'" class="about-result">
    <p v-if="updateState.kind === 'checking'" role="status">正在连接 GitHub…</p>
    <p v-else-if="updateState.kind === 'current'" role="status">暂无可用更新。</p>
    <p v-else-if="updateState.kind === 'empty'" role="status">GitHub 暂无可用的正式版本。</p>
    <p v-else-if="updateState.kind === 'failed'" class="error" role="alert">{{
      updateState.message
    }}</p>
    <p v-else-if="updateState.kind === 'available'" role="status"
      >发现新版本 {{ updateState.version }}</p
    >
    <button
      v-if="updateState.kind === 'available' || updateState.kind === 'failed'"
      class="about-open-release"
      type="button"
      :disabled="opening"
      @click="openRelease('latest')"
    >
      {{ updateState.kind === 'available' ? '前往 GitHub 下载' : '打开 GitHub 发布页' }}
    </button>
    <section v-if="updateState.kind === 'available'" aria-label="更新内容">
      <h3>更新内容</h3>
      <section
        v-for="release in updateState.releases"
        :key="release.version"
        class="about-release-notes"
        :aria-label="`版本 ${release.version}`"
      >
        <h4>{{ release.version }}</h4>
        <div v-if="release.notes" class="release-notes">{{ release.notes }}</div>
        <p v-else>可前往 GitHub 查看完整发布说明。</p>
      </section>
      <p v-if="updateState.historyIncomplete" role="status">
        部分发布记录暂未读取。<button
          type="button"
          :disabled="opening"
          @click="openRelease('history')"
          >查看 GitHub 发布记录</button
        >
      </p>
    </section>
    <p v-if="launchError" class="error" role="alert">{{ launchError }}</p>
  </div>
</template>

<style scoped>
.about-release-card {
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  align-items: center;
  gap: 12px;
  min-height: 66px;
  margin: 4px 0;
  padding: 8px 12px;
  background: var(--fluent-card, var(--bg));
  border: 1px solid var(--line);
  border-radius: var(--radius-control);
}
.about-version {
  display: grid;
  gap: 3px;
  overflow-wrap: anywhere;
}
.about-version > span {
  font-size: var(--font-caption-size);
}
.about-check {
  white-space: nowrap;
}
.about-result {
  margin-top: 12px;
}
.about-result > p {
  margin: 0 0 8px;
}
.about-release-notes {
  border-top: 1px solid var(--line);
  padding: 8px 0;
}
.about-release-notes > h4 {
  margin: 0 0 8px;
}
.release-notes {
  white-space: pre-wrap;
  overflow-wrap: anywhere;
}
</style>

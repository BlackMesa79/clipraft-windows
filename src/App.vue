<script setup lang="ts">
import MainWindow from './MainWindow.vue';
import QuickPanel from './QuickPanel.vue';
import FloatingPreview from './FloatingPreview.vue';
const panel = new URLSearchParams(location.search).get('panel');
const quick = panel === 'quick';
if (quick) document.documentElement.dataset.panel = 'quick';
</script>

<template>
  <FloatingPreview v-if="panel === 'preview'" />
  <QuickPanel v-else-if="quick" />
  <MainWindow v-else />
</template>

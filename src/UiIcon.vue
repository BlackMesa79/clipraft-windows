<script setup lang="ts">
defineProps<{ name: string }>();
const paths: Record<string, string> = {
  brand: 'M7 11V4h9v10H7 M16 7h3v10h-9v-3 M5 20h14 M7 23h10',
  copy: 'M8 8h12v13H8z M15 8V3H3v13h5',
  layers: 'm12 3 10 5-10 5L2 8z M2 12l10 5 10-5 M2 16l10 5 10-5',
  search: 'M16 10a6 6 0 1 1-12 0 6 6 0 0 1 12 0z m-2 4 7 7',
  text: 'M5 5h14 M12 5v15 M8 20h8',
  files: 'M13 3H5v18h14V9z M13 3v6h6 M8 14h8 M8 17h5',
  image: 'M3 3h18v18H3z M3 18l6-6 4 4 4-6 4 5 M7 7h1',
  link: 'm10 14 4-4 M8 16l-2 2a4 4 0 0 1-5-5l5-5a4 4 0 0 1 6 0 M16 8l2-2a4 4 0 0 1 5 5l-5 5a4 4 0 0 1-6 0',
  lock: 'M4 10h16v11H4z M8 10V7a4 4 0 0 1 8 0v3 M12 15v2',
  pin: 'm8 3 8 0 M9 3v6l-3 4v2h12v-2l-3-4V3 M12 15v7',
  refresh: 'M20 7v5h-5 M4 17v-5h5 M6 7a7 7 0 0 1 12-1l2 3 M4 15l2 3a7 7 0 0 0 12-1',
  compact: 'M3 4h18v16H3z M8 4v16 M12 9h5 M12 13h5 M12 17h5',
  expand: 'M3 4h18v16H3z M9 4v16 M13 8h5v8h-5z',
  settings:
    'M16 12a4 4 0 1 1-8 0 4 4 0 0 1 8 0z M12 2v3 M12 19v3 M2 12h3 M19 12h3 M5 5l2 2 M17 17l2 2 M5 19l2-2 M17 7l2-2',
};
</script>

<template>
  <svg
    class="ui-icon"
    viewBox="0 0 24 24"
    fill="none"
    stroke="currentColor"
    stroke-width="1.6"
    stroke-linecap="round"
    stroke-linejoin="round"
    aria-hidden="true"
  >
    <path :d="paths[name] ?? paths.text" />
  </svg>
</template>

<style>
.ui-icon {
  width: 18px;
  height: 18px;
  flex-shrink: 0;
  vertical-align: middle;
}
</style>

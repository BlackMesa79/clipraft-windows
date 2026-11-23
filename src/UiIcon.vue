<script setup lang="ts">
defineProps<{ name: string }>();
const paths: Record<string, string> = {
  copy: 'M8 8h12v13H8z M15 8V3H3v13h5',
  layers: 'm12 3 10 5-10 5L2 8z M2 12l10 5 10-5 M2 16l10 5 10-5',
  history: 'M3 12a9 9 0 1 1 2.64 6.36 M3 6v6h6 M12 7v5l3 2',
  back: 'M20 12H4m0 0 6-6m-6 6 6 6',
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
    'M10.2 5.1 10.1 2.5 13.9 2.5 13.8 5.1 17 7 19.3 5.6 21.2 8.8 18.9 10.2 18.9 13.8 21.2 15.2 19.3 18.4 17 17 13.8 18.9 13.9 21.5 10.1 21.5 10.2 18.9 7 17 4.7 18.4 2.8 15.2 5.1 13.8 5.1 10.2 2.8 8.8 4.7 5.6 7 7Z M14.7 12a2.7 2.7 0 1 1-5.4 0 2.7 2.7 0 1 1 5.4 0Z',
};
</script>

<template>
  <svg
    class="ui-icon"
    :viewBox="name === 'update' ? '0 0 16 16' : '0 0 24 24'"
    fill="none"
    stroke="currentColor"
    stroke-width="1.6"
    stroke-linecap="round"
    stroke-linejoin="round"
    aria-hidden="true"
  >
    <template v-if="name === 'update'">
      <g class="ui-update-line">
        <path d="M8 11.25V3.1m-3.2 3.2L8 3.1l3.2 3.2M5 13.15h6" stroke-width="1.4" />
      </g>
      <g class="ui-update-fluent">
        <rect x="1" y="1" width="14" height="14" rx="4" fill="currentColor" stroke="none" />
        <path
          d="M8 11.7V4.65M5.35 7.3 8 4.65l2.65 2.65"
          stroke="var(--primary-text)"
          stroke-width="1.65"
        />
      </g>
    </template>
    <path v-else :d="paths[name] ?? paths.text" />
  </svg>
</template>

<style>
.ui-icon {
  width: 18px;
  height: 18px;
  flex-shrink: 0;
  vertical-align: middle;
}
.ui-update-fluent,
:root[data-skin='fluent'] .ui-update-line {
  display: none;
}
:root[data-skin='fluent'] .ui-update-fluent {
  display: inline;
}
</style>

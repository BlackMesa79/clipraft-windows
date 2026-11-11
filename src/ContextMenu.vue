<script setup lang="ts">
import { nextTick, onMounted, onUnmounted, ref, watch } from 'vue';
import type { Group } from './history';
const props = defineProps<{ x: number; y: number; groups?: Group[]; allPinned?: boolean; allFavorite?: boolean; label?: string }>();
const expanded = ref(false);
const previousFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
const moveButton = ref<HTMLButtonElement | null>(null);
let restoreOnClose = false;
const emit = defineEmits<{ delete: []; paste: []; close: []; pin: []; favorite: []; move: [groupId: number | null] }>();
const root = ref<HTMLElement | null>(null), left = ref(props.x), top = ref(props.y);
function outside(event: PointerEvent) { if (!root.value?.contains(event.target as Node)) emit('close'); }
function close() { emit('close'); }
function menuItems(menu: Element | null | undefined) {
  return Array.from(menu?.querySelectorAll<HTMLButtonElement>('[role="menuitem"]:not(:disabled)') ?? [])
    .filter(item => item.closest('[role="menu"]') === menu);
}
async function openSubmenu() {
  expanded.value = true;
  await nextTick();
  menuItems(root.value?.querySelector('.group-submenu'))[0]?.focus();
}
function closeSubmenu() { expanded.value = false; moveButton.value?.focus(); }
function keydown(event: KeyboardEvent) {
  // 菜单独占键盘；组合输入不取消默认行为，也不传给列表快捷键。
  event.stopImmediatePropagation();
  if (event.isComposing || event.keyCode === 229) return;
  event.preventDefault();
  if (event.key === 'Escape') { restoreOnClose = true; close(); return; }
  const focused = document.activeElement;
  const item = focused instanceof HTMLButtonElement && root.value?.contains(focused) && focused.matches('[role="menuitem"]') ? focused : null;
  const menu = item?.closest('[role="menu"]') ?? root.value;
  const entries = menuItems(menu);
  if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
    const index = item ? entries.indexOf(item) : -1;
    const next = index < 0 ? (event.key === 'ArrowDown' ? 0 : entries.length - 1)
      : (index + (event.key === 'ArrowDown' ? 1 : -1) + entries.length) % entries.length;
    entries[next]?.focus();
  } else if (event.key === 'ArrowRight' && item === moveButton.value) {
    void openSubmenu();
  } else if (event.key === 'ArrowLeft' && menu !== root.value) {
    closeSubmenu();
  } else if ((event.key === 'Enter' || event.key === ' ') && !event.repeat) {
    item?.click();
  } else if (event.key === 'Tab' && !item) {
    menuItems(root.value)[0]?.focus();
  }
}
async function position() {
  await nextTick();
  if (root.value) {
    const box = root.value.getBoundingClientRect();
    left.value = Math.max(4, Math.min(props.x, window.innerWidth - box.width - 4));
    top.value = Math.max(4, Math.min(props.y, window.innerHeight - box.height - 4));
  }
}
watch(expanded, position);
onMounted(() => {
  menuItems(root.value)[0]?.focus({ preventScroll: true });
  void position();
  window.addEventListener('pointerdown', outside);
  window.addEventListener('blur', close);
  window.addEventListener('keydown', keydown, true);
  window.addEventListener('resize', close);
});
onUnmounted(() => { window.removeEventListener('pointerdown', outside); window.removeEventListener('blur', close); window.removeEventListener('keydown', keydown, true); window.removeEventListener('resize', close);
  void nextTick(() => {
    // 新编辑器或外部点击已主动取得焦点时，保留新的焦点。
    if (previousFocus?.isConnected && (restoreOnClose || document.activeElement === document.body)) previousFocus.focus({ preventScroll: true });
  });
});
</script>

<template>
  <div ref="root" class="context-menu" role="menu" :aria-label="label ?? '记录操作'" :style="{ left: left + 'px', top: top + 'px' }" @contextmenu.prevent>
    <slot>
    <button type="button" role="menuitem" @click="emit('pin')">{{ allPinned ? '取消置顶' : '置顶' }}</button>
    <button type="button" role="menuitem" @click="emit('favorite')">{{ allFavorite ? '取消收藏' : '收藏' }}</button>
    <button ref="moveButton" type="button" role="menuitem" aria-haspopup="menu" :aria-expanded="expanded" @click="expanded ? closeSubmenu() : openSubmenu()">移到分组 ▸</button>
    <div v-if="expanded" role="menu" aria-label="移到分组" class="group-submenu">
      <button type="button" role="menuitem" @click="emit('move', null)">移出分组</button>
      <button v-for="group in groups" :key="group.id" type="button" role="menuitem" @click="emit('move', group.id)">{{ group.name }}</button>
    </div>
    <button type="button" role="menuitem" @click="emit('delete')">删除</button>
    <button type="button" role="menuitem" @click="emit('paste')">粘贴</button>
    </slot>
  </div>
</template>

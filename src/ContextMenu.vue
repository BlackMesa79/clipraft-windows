<script setup lang="ts">
import { nextTick, onMounted, onUnmounted, ref, watch } from 'vue';
import type { Group } from './history';
import { groupColorStyle } from './groupColors';
const props = defineProps<{ x: number; y: number; groups?: Group[]; allPinned?: boolean; allSensitive?: boolean; label?: string; files?: string[] }>();
const expanded = ref(false);
const previousFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
const moveButton = ref<HTMLButtonElement | null>(null);
let restoreOnClose = false;
const emit = defineEmits<{ delete: []; paste: []; close: []; pin: []; sensitive: []; move: [groupId: number | null]; openFile: [index: number]; revealFile: [index: number] }>();
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
function focusedMenuItem(): HTMLButtonElement | null {
  const focused = document.activeElement;
  if (!(focused instanceof HTMLButtonElement)) return null;
  return root.value?.contains(focused) && focused.matches('[role="menuitem"]') ? focused : null;
}
function moveMenuFocus(item: HTMLButtonElement | null, direction: number): void {
  const entries = menuItems(item?.closest('[role="menu"]') ?? root.value);
  const index = item ? entries.indexOf(item) : -1;
  if (index < 0) {
    entries[direction > 0 ? 0 : entries.length - 1]?.focus();
    return;
  }
  entries[(index + direction + entries.length) % entries.length]?.focus();
}
function activateMenuItem(event: KeyboardEvent): void {
  if (!event.repeat) focusedMenuItem()?.click();
}
const menuKeyActions: Readonly<Record<string, (event: KeyboardEvent) => void>> = {
  Escape: () => { restoreOnClose = true; close(); },
  ArrowDown: () => moveMenuFocus(focusedMenuItem(), 1),
  ArrowUp: () => moveMenuFocus(focusedMenuItem(), -1),
  ArrowRight: () => { if (focusedMenuItem() === moveButton.value) void openSubmenu(); },
  ArrowLeft: () => {
    const menu = focusedMenuItem()?.closest('[role="menu"]') ?? root.value;
    if (menu !== root.value) closeSubmenu();
  },
  Enter: activateMenuItem,
  ' ': activateMenuItem,
  Tab: () => { if (!focusedMenuItem()) menuItems(root.value)[0]?.focus(); },
};
function keydown(event: KeyboardEvent): void {
  // 菜单独占键盘；组合输入不取消默认行为，也不传给列表快捷键。
  event.stopImmediatePropagation();
  if (event.isComposing || event.keyCode === 229) return;
  event.preventDefault();
  menuKeyActions[event.key]?.(event);
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
    <button ref="moveButton" type="button" role="menuitem" aria-haspopup="menu" :aria-expanded="expanded" @click="expanded ? closeSubmenu() : openSubmenu()">移到分组 ▸</button>
    <div v-if="expanded" role="menu" aria-label="移到分组" class="group-submenu">
      <button v-for="group in groups" :key="group.id" class="colored-group" :style="groupColorStyle(group.id)" type="button" role="menuitem" @click="emit('move', group.id)"><span class="group-dot" aria-hidden="true"></span>{{ group.name }}</button>
      <div role="separator" class="menu-separator"></div>
      <button type="button" role="menuitem" class="remove-group" title="保留记录，仅移出当前分组" @click="emit('move', null)"><svg aria-hidden="true" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><path d="m9 15-2 2a4 4 0 0 1-6-6l4-4a4 4 0 0 1 6 0m4 2 2-2a4 4 0 0 1 6 6l-4 4a4 4 0 0 1-6 0M4 2v3M2 4h3m15 18v-3m2 1h-3"/></svg> 移出分组</button>
    </div>
    <button type="button" role="menuitem" @click="emit('sensitive')">{{ allSensitive ? '取消敏感标记' : '标记为敏感' }}</button>
    <button type="button" role="menuitem" @click="emit('delete')">删除</button>
    <button type="button" role="menuitem" @click="emit('paste')">粘贴</button>
    <template v-if="files?.length">
      <div role="separator" class="menu-separator"></div>
      <button v-if="files.length === 1" type="button" role="menuitem" @click="emit('openFile', 0)">用默认应用打开</button>
      <button type="button" role="menuitem" @click="emit('revealFile', 0)">在资源管理器中定位</button>
    </template>
    </slot>
  </div>
</template>

<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import ContextMenu from './ContextMenu.vue';
import UiIcon from './UiIcon.vue';
import { groupColorStyle } from './groupColors';
import type { Group, HistoryView } from './history';
const props = defineProps<{
  groups: Group[];
  view: HistoryView;
  disabled: boolean;
  fluent: boolean;
}>();
const emit = defineEmits<{
  select: [view: HistoryView];
  changed: [];
  interacting: [value: boolean];
  menuOpen: [value: boolean];
}>();
const editing = ref<number | 'new' | null>(null),
  name = ref(''),
  saving = ref(false),
  error = ref('');
const input = ref<HTMLInputElement | null>(null);
const menu = ref<{ id: number; x: number; y: number } | null>(null);
const deleting = ref<number | null>(null);
const pending = computed(() => props.groups.find((g) => g.id === deleting.value));
watch(menu, (value) => emit('menuOpen', value !== null), { flush: 'sync' });
watch([editing, deleting, saving], () =>
  emit('interacting', editing.value !== null || deleting.value !== null || saving.value),
);
watch(
  () => props.disabled,
  (disabled) => {
    if (disabled) menu.value = null;
  },
);
function edit(group?: Group) {
  menu.value = null;
  error.value = '';
  deleting.value = null;
  editing.value = group?.id ?? 'new';
  name.value = group?.name ?? '';
  void nextTick(() => {
    input.value?.focus();
    input.value?.select();
  });
}
function openMenu(group: Group, event: MouseEvent) {
  if (props.disabled || saving.value || editing.value !== null || deleting.value !== null) return;
  menu.value = { id: group.id, x: event.clientX, y: event.clientY };
}
function cancel() {
  if (!saving.value) {
    editing.value = null;
    deleting.value = null;
    menu.value = null;
    error.value = '';
  }
}
function keydown(event: KeyboardEvent) {
  if (
    event.isComposing ||
    event.keyCode === 229 ||
    (editing.value === null && deleting.value === null)
  )
    return;
  if (event.key === 'Escape') {
    event.preventDefault();
    event.stopPropagation();
    cancel();
  }
}
onMounted(() => window.addEventListener('keydown', keydown, true));
onUnmounted(() => {
  window.removeEventListener('keydown', keydown, true);
  emit('menuOpen', false);
});
async function save() {
  if (saving.value || props.disabled || editing.value === null) return;
  saving.value = true;
  error.value = '';
  try {
    await invoke(editing.value === 'new' ? 'create_group' : 'rename_group', {
      id: editing.value,
      name: name.value,
    });
    editing.value = null;
    emit('changed');
  } catch (e) {
    error.value = String(e);
  } finally {
    saving.value = false;
  }
}
async function remove() {
  if (saving.value || props.disabled || deleting.value === null) return;
  saving.value = true;
  error.value = '';
  try {
    await invoke('delete_group', { id: deleting.value });
    deleting.value = null;
    emit('changed');
  } catch (e) {
    error.value = String(e);
  } finally {
    saving.value = false;
  }
}
</script>

<template>
  <nav class="group-sidebar" aria-label="历史视图与分组" title="Ctrl+↑/↓ 切换分组">
    <div class="group-scroll">
      <div class="group-views">
        <button
          v-for="entry in [
            ['all', '全部'],
            ['ungrouped', '未分组'],
          ] as const"
          :key="entry[0]"
          :data-group-view="entry[0]"
          type="button"
          :aria-pressed="view === entry[0]"
          :disabled="disabled || saving || editing !== null || deleting !== null"
          @click="emit('select', entry[0])"
          ><UiIcon v-if="entry[0] === 'all'" :name="fluent ? 'history' : 'layers'" /><span
            v-else
            class="group-dot ungrouped-dot"
            aria-hidden="true"
          ></span
          >{{ fluent && entry[0] === 'all' ? '全部分组' : entry[1] }}</button
        >
        <div class="groups-heading">我的分组</div>
        <template v-for="group in groups" :key="group.id">
          <form v-if="editing === group.id" class="group-editor" @submit.prevent="save">
            <input
              :ref="(el) => (input = el as HTMLInputElement)"
              v-model="name"
              aria-label="重命名分组"
              :disabled="saving"
              @keydown.enter="
                ($event.isComposing || $event.keyCode === 229) && $event.preventDefault()
              "
            />
            <button type="submit" :disabled="saving || disabled">保存</button
            ><button type="button" :disabled="saving" @click="cancel">取消</button>
          </form>
          <button
            v-else
            class="colored-group"
            :data-group-view="group.id"
            :style="groupColorStyle(group.id)"
            type="button"
            :aria-pressed="view === group.id"
            :disabled="disabled || saving || editing !== null || deleting !== null"
            @click="emit('select', group.id)"
            @contextmenu.prevent="openMenu(group, $event)"
            ><span class="group-dot" aria-hidden="true"></span
            ><span class="group-name">{{ group.name }}</span
            >{{ ' ' }}<span class="group-count">{{ group.count }}</span></button
          >
        </template>
        <button
          type="button"
          :disabled="
            disabled || saving || editing !== null || deleting !== null || groups.length >= 100
          "
          @click="edit()"
          >+ 新建分组</button
        >
      </div>
      <form v-if="editing === 'new'" class="group-editor" @submit.prevent="save">
        <input
          ref="input"
          v-model="name"
          aria-label="新分组名称"
          placeholder="1–40 个字符"
          :disabled="saving"
          @keydown.enter="($event.isComposing || $event.keyCode === 229) && $event.preventDefault()"
        />
        <button type="submit" :disabled="saving || disabled">创建</button
        ><button type="button" :disabled="saving" @click="cancel">取消</button>
      </form>
      <div v-if="deleting !== null" class="delete-confirm" role="alertdialog" aria-label="删除分组">
        <span
          >删除“{{ pending?.name }}”？组内
          {{ pending?.count ?? 0 }} 条记录会回到未分组，不会删除记录。</span
        >
        <button type="button" :disabled="saving || disabled" @click="remove">确认删除分组</button
        ><button type="button" :disabled="saving" @click="cancel">取消</button>
      </div>
      <p v-if="error" class="error" role="alert">{{ error }}</p>
    </div>
    <div class="sidebar-bottom"><slot /></div>
    <ContextMenu v-if="menu" :x="menu.x" :y="menu.y" label="分组操作" @close="menu = null">
      <button type="button" role="menuitem" @click="edit(groups.find((g) => g.id === menu?.id))"
        >重命名</button
      >
      <button
        type="button"
        role="menuitem"
        @click="
          deleting = menu.id;
          menu = null;
          error = '';
        "
        >删除分组</button
      >
    </ContextMenu>
  </nav>
</template>

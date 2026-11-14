<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
async function windowAction(action: 'minimize' | 'toggleMaximize' | 'close') {
  try {
    await getCurrentWindow()[action]();
  } catch (e) {
    error.value = '窗口操作失败：' + String(e);
  }
}
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import './styles/main.css';
import PreviewImage from './PreviewImage.vue';
import { createItemDetail } from './itemDetail';
import { createCompactPreview } from './compactPreview';
import HistoryList from './HistoryList.vue';
import SettingsView from './SettingsView.vue';
import { applyTheme, shortcutLabel, type Settings, type SettingOptions } from './settings';
import ContextMenu from './ContextMenu.vue';
import GroupSidebar from './GroupSidebar.vue';
import UiIcon from './UiIcon.vue';
import { groupColorStyle } from './groupColors';
import { brandName } from './brand';
import {
  kindLabel,
  type Item,
  type HistoryFilter,
  type HistoryKind,
  type Group,
  type HistoryView,
} from './history';
let fileTimer: ReturnType<typeof setInterval> | undefined;
const items = ref<Item[]>([]),
  selected = ref(0),
  error = ref(''),
  busy = ref(false);
const settingsOpen = ref(false),
  settingsBusy = ref(false),
  settingsRecording = ref(false),
  startupError = ref('');
const settings = ref<Settings>(),
  settingOptions = ref<SettingOptions>();
const appearanceBusy = ref(false);
const compact = computed(() => settings.value?.compact ?? false);
const layout = computed(() => settings.value?.layout ?? 'list');
watch(settings, applyTheme);
let appearanceGeneration = 0;
let savedAnchor: { id: string; offset: number } | undefined;
function rememberAnchor() {
  const list = listRoot.value;
  const top = list?.getBoundingClientRect().top ?? 0;
  const row =
    list &&
    Array.from(list.querySelectorAll<HTMLElement>('.item')).find(
      (el) => el.getBoundingClientRect().bottom > top + 1,
    );
  if (row?.dataset.id)
    savedAnchor = { id: row.dataset.id, offset: row.getBoundingClientRect().top - top };
}
function recordElement(id: string | undefined): HTMLElement | undefined {
  if (!id) return undefined;
  return Array.from(listRoot.value?.querySelectorAll<HTMLElement>('.item') ?? []).find(
    (element) => element.dataset.id === id,
  );
}
function restoreAnchor() {
  const list = listRoot.value;
  const row = recordElement(savedAnchor?.id);
  if (row && list && savedAnchor) {
    // 卡片切成短行时，旧负偏移可能超过整行高度；至少保留锚点的一部分可见。
    const offset = Math.max(savedAnchor.offset, 2 - row.getBoundingClientRect().height);
    list.scrollTop += row.getBoundingClientRect().top - list.getBoundingClientRect().top - offset;
  }
}
watch(settingsOpen, async (open) => {
  if (open) rememberAnchor();
  else {
    await nextTick();
    restoreAnchor();
  }
});
watch(
  () => [settings.value?.theme, layout.value, compact.value],
  async () => {
    const generation = ++appearanceGeneration;
    rememberAnchor();
    conceal();
    await nextTick();
    if (generation !== appearanceGeneration) return;
    restoreAnchor();
    if (!compact.value) void loadDetail();
    void invoke('sync_main_layout')
      .then(syncFloatingPreview)
      .catch((e) => {
        error.value = '窗口尺寸调整失败：' + String(e);
      });
  },
  { flush: 'pre' },
);
async function toggleCompact() {
  if (
    !settings.value ||
    appearanceBusy.value ||
    settingsOpen.value ||
    organizing.value ||
    deleting.value ||
    busy.value ||
    pendingDelete.value
  )
    return;
  appearanceBusy.value = true;
  try {
    settings.value = await invoke<Settings>('update_settings', {
      settings: { ...settings.value, compact: !compact.value },
    });
  } catch (e) {
    error.value = String(e);
  } finally {
    appearanceBusy.value = false;
  }
}
const mainLabel = computed(() => shortcutLabel(settings.value?.main_hotkey));
const quickKey = computed(() => ({ label: shortcutLabel(settings.value?.quick_hotkey) }));
function toggleSettings() {
  if (settingsBusy.value || deleting.value || busy.value || organizing.value) return;
  if (!settingsOpen.value) rememberAnchor();
  conceal();
  settingsOpen.value = !settingsOpen.value;
  contextMenu.value = null;
  pendingDelete.value = null;
  if (!settingsOpen.value) {
    void nextTick(checkLoadMore);
    void focusSearch();
  }
}
const detailRoot = ref<HTMLElement | null>(null);
const searchInput = ref<HTMLInputElement | null>(null);
const query = ref(''),
  kind = ref<HistoryKind | null>(null),
  composing = ref(false);
const kinds: { value: HistoryKind | null; label: string }[] = [
  { value: null, label: '全部' },
  { value: 'text', label: '文本' },
  { value: 'image', label: '图片' },
  { value: 'files', label: '文件' },
  { value: 'link', label: '链接' },
];
function selectView(next: HistoryView) {
  view.value = next;
}
function selectCompactView(event: Event) {
  const value = (event.target as HTMLSelectElement).value;
  selectView(value === 'all' || value === 'ungrouped' ? value : Number(value));
  // 下拉选择提交后交还记录导航；弹层选择过程中仍由原生select处理。
  void focusSearch();
}

const groups = ref<Group[]>([]),
  view = ref<HistoryView>('all'),
  groupError = ref('');
const mutationBusy = ref(false),
  groupInteracting = ref(false),
  groupMenuOpen = ref(false);
const organizing = computed(() => mutationBusy.value || groupInteracting.value);
const selectedItems = computed(() => items.value.filter((item) => selectedIds.value.has(item.id)));
const allPinned = computed(
  () => selectedItems.value.length > 0 && selectedItems.value.every((item) => item.pinned),
);
const allSensitive = computed(
  () => selectedItems.value.length > 0 && selectedItems.value.every((item) => item.sensitive),
);
const protectedDeleteCount = computed(
  () =>
    items.value.filter(
      (item) =>
        pendingDelete.value?.includes(item.id) &&
        (item.pinned || item.favorite || item.group_id != null),
    ).length,
);
const listFilter = computed<HistoryFilter>(() => {
  const filter: HistoryFilter = { query: query.value, kind: kind.value };
  if (view.value === 'ungrouped') filter.group_id = -1;
  if (typeof view.value === 'number') filter.group_id = view.value;
  return filter;
});
let groupGeneration = 0;
async function loadGroups() {
  const generation = ++groupGeneration;
  try {
    const result = await invoke<Group[]>('list_groups');
    if (generation !== groupGeneration) return;
    groups.value = result;
    groupError.value = '';
    if (typeof view.value === 'number' && !result.some((group) => group.id === view.value))
      view.value = 'ungrouped';
  } catch (e) {
    if (generation === groupGeneration) groupError.value = '分组读取失败：' + String(e);
  }
}
function groupsChanged() {
  void loadGroups();
  void load();
}
async function changeItems(
  command: 'set_pinned' | 'move_to_group' | 'set_sensitive',
  groupId?: number | null,
) {
  if (
    organizing.value ||
    deleting.value ||
    busy.value ||
    pendingDelete.value ||
    !selectedItems.value.length
  )
    return;
  const ids = selectedItems.value.map((item) => item.id);
  const args =
    command === 'move_to_group'
      ? { ids, groupId }
      : { ids, value: command === 'set_pinned' ? !allPinned.value : !allSensitive.value };
  mutationBusy.value = true;
  contextMenu.value = null;
  error.value = '';
  ++listGeneration;
  invalidateDetail();
  try {
    await invoke(command, args);
  } catch (e) {
    error.value = String(e);
  } finally {
    // 即使刷新事件失败，仍重新读库，避免把已提交的修改显示为未保存。
    deferredHistory = false;
    await Promise.all([loadGroups(), load()]);
    mutationBusy.value = false;
    if (deferredHistory) {
      deferredHistory = false;
      historyChanged();
    }
    void nextTick(checkLoadMore);
  }
}
const filtered = computed(
  () => !!query.value.trim() || kind.value !== null || view.value !== 'all',
);
let resetOnShow = false;
let searchTimer: ReturnType<typeof setTimeout> | undefined;
async function focusSearch() {
  await nextTick();
  if (!settingsOpen.value) searchInput.value?.focus();
}
function filtersChanged(delay = 150) {
  resetOnShow = false;
  clearTimeout(searchTimer);
  searchTimer = undefined;
  ++listGeneration;
  invalidateDetail();
  afterDelete = undefined;
  contextMenu.value = null;
  pendingDelete.value = null;
  items.value = [];
  selectOnly(0);
  nextOffset = 0;
  hasMore.value = true;
  pageError.value = '';
  pageLoading.value = true;
  if (listRoot.value) listRoot.value.scrollTop = 0;
  if (!composing.value)
    searchTimer = setTimeout(() => {
      searchTimer = undefined;
      void load();
    }, delay);
}
watch(query, () => filtersChanged());
watch(kind, () => filtersChanged(0));
watch(view, () => filtersChanged(0));
function compositionStart() {
  composing.value = true;
  clearTimeout(searchTimer);
  searchTimer = undefined;
}
function compositionEnd() {
  composing.value = false;
  filtersChanged();
}
const PAGE_SIZE = 100;
const historyList = ref<InstanceType<typeof HistoryList> | null>(null);
const listRoot = computed(() => historyList.value?.element ?? null);
const pageLoading = ref(false),
  pageError = ref(''),
  hasMore = ref(true);
let listGeneration = 0,
  nextOffset = 0,
  retryRefresh = false;
const selectedIds = ref(new Set<string>()),
  pendingDelete = ref<string[] | null>(null),
  deleting = ref(false);
const contextMenu = ref<{ x: number; y: number; id: string } | null>(null);
const menuInteracting = computed(() => groupMenuOpen.value || contextMenu.value !== null);
let anchorId: string | undefined,
  deferredHistory = false;
let afterDelete: { index: number; id?: string; count: number } | undefined;
function selectOnly(index: number) {
  selected.value = index;
  anchorId = items.value[index]?.id;
  selectedIds.value = new Set(anchorId ? [anchorId] : []);
}
function choose(index: number, event: MouseEvent) {
  resetOnShow = false;
  if (deleting.value || organizing.value || pendingDelete.value) return;
  contextMenu.value = null;
  const id = items.value[index]?.id;
  if (!id) return;
  if (event.shiftKey) {
    const anchor = Math.max(
      0,
      items.value.findIndex((item) => item.id === anchorId),
    );
    const range = items.value
      .slice(Math.min(anchor, index), Math.max(anchor, index) + 1)
      .map((item) => item.id);
    selectedIds.value = new Set(event.ctrlKey ? [...selectedIds.value, ...range] : range);
    selected.value = index;
  } else if (event.ctrlKey) {
    const ids = new Set(selectedIds.value);
    if (ids.has(id)) ids.delete(id);
    else ids.add(id);
    selectedIds.value = ids;
    selected.value = index;
    anchorId = id;
  } else selectOnly(index);
}
function openContext(index: number, event: MouseEvent) {
  resetOnShow = false;
  if (deleting.value || organizing.value || pendingDelete.value) return;
  const id = items.value[index]?.id;
  if (!id) return;
  if (!selectedIds.value.has(id)) selectOnly(index);
  else selected.value = index;
  contextMenu.value = { x: event.clientX, y: event.clientY, id };
}
function requestDelete() {
  if (deleting.value || organizing.value || busy.value || pendingDelete.value) return;
  contextMenu.value = null;
  const ids = items.value.filter((item) => selectedIds.value.has(item.id)).map((item) => item.id);
  if (ids.length > 1) pendingDelete.value = ids;
  else if (ids.length === 1) void performDelete(ids);
}
interface DeleteReport {
  deleted_items: number;
  reclaimed_blobs: number;
  failed_blobs: string[];
  maintenance_errors: string[];
}
async function performDelete(ids: string[]) {
  if (deleting.value || busy.value || organizing.value) return;
  const removed = new Set(ids),
    index = items.value.findIndex((item) => removed.has(item.id));
  const next = items.value.slice(Math.max(0, index)).find((item) => !removed.has(item.id));
  const position = { index: Math.max(0, index), id: next?.id, count: items.value.length };
  deleting.value = true;
  pendingDelete.value = null;
  contextMenu.value = null;
  error.value = '';
  ++listGeneration;
  invalidateDetail();
  try {
    const report = await invoke<DeleteReport>('delete_items', { ids });
    afterDelete = position;
    if (report.failed_blobs.length || report.maintenance_errors.length)
      error.value =
        '记录已删除，部分维护未完成，将在下次启动重试。' + report.maintenance_errors.join('；');
    deferredHistory = false;
    await load();
  } catch (e) {
    error.value = String(e);
    pageLoading.value = false;
    void loadDetail();
  } finally {
    deleting.value = false;
    if (deferredHistory) {
      deferredHistory = false;
      void load();
    }
  }
}
function confirmDelete() {
  if (pendingDelete.value) void performDelete(pendingDelete.value);
}
function historyChanged() {
  invalidateFloatingPreview();
  contextMenu.value = null;
  void loadGroups();
  if (deleting.value || mutationBusy.value) deferredHistory = true;
  else if (!searchTimer && !composing.value) void load();
}
function contextPaste() {
  contextMenu.value = null;
  void paste(false);
}

function checkLoadMore() {
  const list = listRoot.value;
  if (list && list.scrollHeight - list.scrollTop - list.clientHeight < 120) void loadMore();
}
async function loadMore() {
  if (
    deleting.value ||
    mutationBusy.value ||
    pageLoading.value ||
    pageError.value ||
    !hasMore.value
  )
    return;
  const generation = listGeneration,
    offset = nextOffset;
  pageLoading.value = true;
  try {
    const page = await invoke<Item[]>('list_items', {
      filter: listFilter.value,
      offset,
      limit: PAGE_SIZE,
    });
    if (generation !== listGeneration) return;
    const known = new Set(items.value.map((item) => item.id));
    items.value.push(...page.filter((item) => !known.has(item.id)));
    nextOffset = offset + page.length;
    hasMore.value = page.length === PAGE_SIZE;
  } catch (e) {
    if (generation === listGeneration) {
      pageError.value = '加载下一页失败：' + String(e);
      retryRefresh = false;
    }
  } finally {
    if (generation === listGeneration) {
      pageLoading.value = false;
      await nextTick();
      checkLoadMore();
    }
  }
}
function retryPage() {
  pageError.value = '';
  void (retryRefresh ? load() : loadMore());
}
const stops: UnlistenFn[] = [];
const active = computed(() => items.value[selected.value]);
const {
  detail,
  detailError,
  detailLoading,
  revealed,
  conceal,
  revealDetail,
  loadDetail,
  invalidateDetail,
} = createItemDetail({
  active: () => active.value,
  compact: () => compact.value,
  resetScroll: () => {
    if (detailRoot.value) detailRoot.value.scrollTop = 0;
  },
});
function visibilityChanged() {
  if (document.hidden) hiddenMain();
  else focusMain();
}
const {
  floatingActive,
  syncFloatingPreview,
  suspendFloatingPreview,
  schedulePreviewGeometry,
  cancelPreviewGeometry,
  invalidateFloatingPreview,
} = createCompactPreview({
  active: () => active.value,
  compact: () => compact.value,
  settingsOpen: () => settingsOpen.value,
  listRoot: () => listRoot.value,
  recordElement,
  reportError: (message) => {
    error.value = message;
  },
});
function focusMain() {
  floatingActive.value = true;
  if (
    !compact.value &&
    !settingsOpen.value &&
    active.value?.kind !== 'image' &&
    !detail.value &&
    !detailLoading.value
  )
    void loadDetail();
  syncFloatingPreview();
}
// 浮窗显隐由原生前台归属复核处理，避免标题栏拖动时的 WebView blur 误关浮窗。
function blurMain() {
  if (active.value?.sensitive) conceal();
}
function hiddenMain() {
  resetOnShow = false;
  conceal();
  suspendFloatingPreview();
}
watch([active, compact, settingsOpen, floatingActive], syncFloatingPreview);

watch(
  () => active.value?.id,
  () => {
    void loadDetail();
  },
  { flush: 'sync' },
);
type DeletionPosition = { index: number; id?: string; count: number };
interface RefreshedPage {
  refreshed: Item[];
  more: boolean;
}
function refreshTargetId(deletion: DeletionPosition | undefined): string | undefined {
  return resetOnShow ? undefined : (deletion?.id ?? active.value?.id);
}
async function refreshPages(
  generation: number,
  targetCount: number,
  deletion: DeletionPosition | undefined,
): Promise<RefreshedPage | undefined> {
  const refreshed: Item[] = [];
  let more = true;
  // 重建已加载范围；选中项移到下一页时继续查找，保留选择。
  while (more) {
    const page = await invoke<Item[]>('list_items', {
      filter: listFilter.value,
      offset: refreshed.length,
      limit: PAGE_SIZE,
    });
    if (generation !== listGeneration) return;
    refreshed.push(...page);
    more = page.length === PAGE_SIZE;
    const id = refreshTargetId(deletion);
    if (
      refreshed.length >= targetCount &&
      (deletion || !id || refreshed.some((item) => item.id === id))
    )
      break;
  }
  return { refreshed, more };
}
interface SelectionRestore {
  id?: string;
  oldIndex: number;
  deletion?: DeletionPosition;
}
function restoreRefreshedSelection(refreshed: Item[], position: SelectionRestore): void {
  const { id, oldIndex, deletion } = position;
  const previousSelection = selectedIds.value;
  const unique = new Map(refreshed.map((item) => [item.id, item]));
  items.value = [...unique.values()];
  const found = items.value.findIndex((item) => item.id === id);
  selected.value = found >= 0 ? found : Math.max(0, Math.min(oldIndex, items.value.length - 1));
  if (deletion || !id || found < 0) selectOnly(selected.value);
  else
    selectedIds.value = new Set(
      items.value.filter((item) => previousSelection.has(item.id)).map((item) => item.id),
    );
}
function refreshSelectionPosition(deletion: DeletionPosition | undefined): {
  resetSelection: boolean;
  id: string | undefined;
  oldIndex: number;
} {
  const resetSelection = resetOnShow && !!settings.value?.select_first_on_show;
  const retainedId = deletion ? deletion.id : active.value?.id;
  const id = resetSelection ? undefined : retainedId;
  const oldIndex = resetSelection ? 0 : (deletion?.index ?? selected.value);
  return { resetSelection, id, oldIndex };
}
function applyRefreshedPage(
  result: RefreshedPage,
  deletion: DeletionPosition | undefined,
): boolean {
  const { refreshed, more } = result;
  const { resetSelection, id, oldIndex } = refreshSelectionPosition(deletion);
  resetOnShow = false;
  restoreRefreshedSelection(refreshed, { id, oldIndex, deletion });
  if (afterDelete === deletion) afterDelete = undefined;
  if (!items.value.some((item) => item.id === anchorId)) anchorId = active.value?.id;
  nextOffset = refreshed.length;
  hasMore.value = more;
  if (id === active.value?.id) void loadDetail();
  return resetSelection;
}
async function restoreRefreshScroll(generation: number, scrollTop: number): Promise<void> {
  await nextTick();
  if (generation === listGeneration && listRoot.value) listRoot.value.scrollTop = scrollTop;
}
function refreshTargetCount(): number {
  return resetOnShow ? PAGE_SIZE : Math.max(PAGE_SIZE, afterDelete?.count ?? items.value.length);
}
async function load() {
  conceal();
  const generation = ++listGeneration,
    targetCount = refreshTargetCount();
  const deletion = afterDelete;
  const scrollTop = listRoot.value?.scrollTop ?? 0;
  pageLoading.value = true;
  pageError.value = '';
  try {
    const result = await refreshPages(generation, targetCount, deletion);
    if (!result || generation !== listGeneration) return;
    const resetSelection = applyRefreshedPage(result, deletion);
    await restoreRefreshScroll(generation, resetSelection ? 0 : scrollTop);
  } catch (e) {
    if (generation === listGeneration) {
      pageError.value = '刷新历史失败：' + String(e);
      retryRefresh = true;
    }
  } finally {
    if (generation === listGeneration) {
      pageLoading.value = false;
      await nextTick();
      checkLoadMore();
    }
  }
}
const fileActionBusy = ref(false);
const fileActionBlocked = computed(
  () =>
    fileActionBusy.value || busy.value || deleting.value || organizing.value || pendingDelete.value,
);
async function runFileAction(action: 'open' | 'reveal', index: number) {
  const id = contextMenu.value?.id ?? active.value?.id;
  if (!id || fileActionBlocked.value) return;
  contextMenu.value = null;
  fileActionBusy.value = true;
  error.value = '';
  try {
    await invoke('file_action', { id, index, action });
  } catch (e) {
    error.value = String(e);
  } finally {
    fileActionBusy.value = false;
  }
}
async function paste(plain = false) {
  if (!active.value || busy.value || deleting.value || organizing.value || pendingDelete.value)
    return;
  busy.value = true;
  error.value = '';
  try {
    await invoke('paste_item', { id: active.value.id, plain });
  } catch (e) {
    error.value = String(e);
  } finally {
    busy.value = false;
  }
}
function selectFocusedRecord(target: Element | null): void {
  const row = target?.closest<HTMLElement>('.list .item[data-id]');
  if (row) {
    const index = items.value.findIndex((item) => item.id === row.dataset.id);
    if (index >= 0 && index !== selected.value) {
      resetOnShow = false;
      selectOnly(index);
    }
  }
}
function prepareEnterTarget(event: KeyboardEvent): boolean {
  const target = event.target instanceof Element ? event.target : document.activeElement;
  if (
    target?.closest(
      'button:not(.item), select, a, textarea, input:not([type="search"]), [contenteditable]',
    )
  )
    return false;
  const inSearch = target === searchInput.value;
  const inList = !!target?.closest('.list');
  selectFocusedRecord(target);
  const noFocus = !target || target === document.body || target === document.documentElement;
  if (!inSearch && !inList && !noFocus) return false;
  return true;
}
function isPinShortcut(event: KeyboardEvent): boolean {
  return (
    event.ctrlKey &&
    !event.altKey &&
    !event.metaKey &&
    !event.shiftKey &&
    event.key.toLowerCase() === 'p'
  );
}
function settingsKeydown(event: KeyboardEvent): void {
  if (isPinShortcut(event)) event.preventDefault();
  if (event.key !== 'Escape') return;
  event.preventDefault();
  if (!settingsBusy.value) toggleSettings();
}
function isSearchShortcut(event: KeyboardEvent): boolean {
  return !settingsOpen.value && event.ctrlKey && !event.altKey && event.key.toLowerCase() === 'f';
}
async function keydown(event: KeyboardEvent): Promise<void> {
  if (settingsOpen.value && settingsRecording.value) return;
  if (event.isComposing || composing.value || event.keyCode === 229) return;
  if (menuInteracting.value) {
    if (event.key === 'Escape') {
      event.preventDefault();
      contextMenu.value = null;
    }
    return;
  }
  if (isSearchShortcut(event)) {
    event.preventDefault();
    void focusSearch();
    return;
  }
  if (settingsOpen.value) {
    settingsKeydown(event);
    return;
  }
  await recordKeydown(event);
}
function handleRecordInput(event: KeyboardEvent): boolean {
  const recordShortcut = isPinShortcut(event);
  const target = event.target instanceof Element ? event.target : document.activeElement;
  const otherInput =
    target !== searchInput.value && !!target?.closest('input, textarea, select, [contenteditable]');
  if (otherInput) return true;
  if (recordShortcut) {
    event.preventDefault();
    if (!event.repeat && !organizing.value && !pendingDelete.value) void changeItems('set_pinned');
    return true;
  }
  return false;
}
async function recordKeydown(event: KeyboardEvent): Promise<void> {
  if (handleRecordInput(event)) return;
  if (organizing.value) return;
  // 保留工具按钮的原生激活；记录按钮的 Enter 属于记录粘贴协议。
  if (event.key === ' ') return;
  if (event.key === 'Enter' && !prepareEnterTarget(event)) return;
  if (pendingDelete.value) {
    if (event.key === 'Escape') {
      event.preventDefault();
      pendingDelete.value = null;
    }
    // 确认条按钮仍可通过 Tab / Enter 操作，但不触发列表粘贴。
    return;
  }
  if (deleting.value) {
    event.preventDefault();
    return;
  }
  await dispatchRecordKeydown(event);
}
async function prepareNavigation(event: KeyboardEvent): Promise<boolean> {
  if (event.key === 'ArrowDown' && selected.value === items.value.length - 1 && hasMore.value) {
    if (pageLoading.value || pageError.value) return false;
    const generation = listGeneration;
    await loadMore();
    if (generation !== listGeneration) return false;
    if (selected.value === items.value.length - 1 && hasMore.value) return false;
  }
  return true;
}
async function navigateRecords(event: KeyboardEvent): Promise<void> {
  const inSearch = event.target === searchInput.value;
  event.preventDefault();
  resetOnShow = false;
  if (!(await prepareNavigation(event))) return;
  if (items.value.length)
    selectOnly(
      (selected.value + (event.key === 'ArrowDown' ? 1 : -1) + items.value.length) %
        items.value.length,
    );
  contextMenu.value = null;
  void nextTick(() => {
    const row = listRoot.value?.querySelector<HTMLElement>(`.item[data-id="${active.value?.id}"]`);
    row?.scrollIntoView({ block: 'nearest' });
    // 从侧栏/筛选器进入记录导航后，焦点与选中记录一致；搜索中仍可直接输入。
    if (!inSearch) row?.focus({ preventScroll: true });
  });
}
function pasteFromKeyboard(event: KeyboardEvent): void {
  if (contextMenu.value) return;
  event.preventDefault();
  void paste(event.shiftKey);
}
function escapeRecords(event: KeyboardEvent): void {
  event.preventDefault();
  if (query.value.length) {
    query.value = '';
    void focusSearch();
    return;
  }
  conceal();
  void invoke('hide_main');
}
const recordKeyActions: Readonly<Record<string, (event: KeyboardEvent) => void | Promise<void>>> = {
  Enter: pasteFromKeyboard,
  ArrowDown: navigateRecords,
  ArrowUp: navigateRecords,
  Escape: escapeRecords,
};
async function dispatchRecordKeydown(event: KeyboardEvent): Promise<void> {
  const inSearch = event.target === searchInput.value;
  if (event.key === 'Escape' && contextMenu.value) {
    event.preventDefault();
    contextMenu.value = null;
    return;
  }
  // 搜索框非空时保留原生 Delete，光标在末尾也不会误删历史。
  if (event.key === 'Delete') {
    if (inSearch && query.value.length) return;
    event.preventDefault();
    requestDelete();
    return;
  }
  if (event.ctrlKey && event.key.toLowerCase() === 'a') {
    if (inSearch) return;
    event.preventDefault();
    selectedIds.value = new Set(items.value.map((item) => item.id));
    contextMenu.value = null;
    return;
  }
  await recordKeyActions[event.key]?.(event);
}

function mainShown() {
  focusMain();
  void focusSearch();
  resetOnShow = !!settings.value?.select_first_on_show;
  if (resetOnShow) {
    selectOnly(0);
    if (listRoot.value) listRoot.value.scrollTop = 0;
    // 重新读取当前筛选的首屏，包含隐藏期间新增/重新排序的记录。
    void load();
  } else if (items.value.some((i) => i.kind === 'files')) historyChanged();
}
// 此入口按顺序注册窗口事件并初始化状态；保留统一生命周期编排。
// eslint-disable-next-line max-lines-per-function
onMounted(async () => {
  try {
    stops.push(await listen('main-shown', mainShown));
    stops.push(await listen('main-hidden', hiddenMain));
    stops.push(
      await listen('main-deactivated', () => {
        blurMain();
        suspendFloatingPreview();
      }),
    );
    stops.push(await listen('main-activated', focusMain));
    stops.push(
      await listen('open-settings', () => {
        conceal();
        settingsOpen.value = true;
        contextMenu.value = null;
        pendingDelete.value = null;
      }),
    );
    stops.push(
      await listen('file-status-changed', () => {
        historyChanged();
      }),
    );
    stops.push(
      await listen('history-changed', () => {
        historyChanged();
      }),
    );
    stops.push(
      await listen<string>('operation-error', ({ payload }) => {
        error.value = payload;
      }),
    );
    stops.push(
      await listen<Settings>('settings-changed', ({ payload }) => {
        settings.value = payload;
      }),
    );
    stops.push(
      await listen<string>('startup-error', ({ payload }) => {
        startupError.value = payload;
      }),
    );
    [settings.value, settingOptions.value] = await Promise.all([
      invoke<Settings>('get_settings'),
      invoke<SettingOptions>('get_setting_options'),
    ]);
    startupError.value = (await invoke<string | null>('startup_error')) ?? '';
    await loadGroups();
    await load();
    error.value = (await invoke<string | null>('last_error')) ?? '';
    await invoke('frontend_ready');
    await focusSearch();
  } catch (e) {
    error.value = '初始化失败：' + String(e);
  }
  fileTimer = setInterval(() => {
    if (!document.hidden && !settingsOpen.value && items.value.some((i) => i.kind === 'files'))
      historyChanged();
  }, 30_000);
  window.addEventListener('keydown', keydown);
  window.addEventListener('focus', focusMain);
  window.addEventListener('blur', blurMain);
  document.addEventListener('visibilitychange', visibilityChanged);
  window.addEventListener('resize', checkLoadMore);
  window.addEventListener('resize', schedulePreviewGeometry);
});
onUnmounted(() => {
  cancelPreviewGeometry();
  window.removeEventListener('resize', schedulePreviewGeometry);
  suspendFloatingPreview();
  clearInterval(fileTimer);
  conceal();
  window.removeEventListener('blur', blurMain);
  document.removeEventListener('visibilitychange', visibilityChanged);
  clearTimeout(searchTimer);
  stops.forEach((stop) => stop());
  window.removeEventListener('keydown', keydown);
  window.removeEventListener('focus', focusMain);
  window.removeEventListener('resize', checkLoadMore);
  listGeneration++;
  invalidateDetail();
  groupGeneration++;
});
</script>

<template>
  <main
    :class="[
      'app',
      'main-window',
      { compact, 'cards-layout': layout === 'cards', 'settings-open': settingsOpen },
    ]"
  >
    <header data-tauri-drag-region>
      <div class="brand" data-tauri-drag-region
        ><span class="brand-mark"><UiIcon name="brand" /></span
        ><div
          ><h1>{{ brandName }}</h1
          ><p>你的内容，随时可用 · {{ mainLabel }}</p></div
        ></div
      >
      <button
        type="button"
        class="compact-toggle icon-button"
        :aria-label="compact ? '展开视图' : '紧凑模式'"
        :title="compact ? '展开视图' : '紧凑模式'"
        :aria-pressed="compact"
        :disabled="
          appearanceBusy || settingsOpen || organizing || deleting || busy || !!pendingDelete
        "
        @click="toggleCompact"
        ><UiIcon :name="compact ? 'expand' : 'compact'" /><span class="sr-only">{{
          compact ? '展开视图' : '紧凑模式'
        }}</span></button
      >
      <div class="window-controls" aria-label="窗口操作">
        <button type="button" aria-label="最小化" title="最小化" @click="windowAction('minimize')"
          >−</button
        >
        <button
          type="button"
          aria-label="最大化或还原"
          title="最大化或还原"
          @click="windowAction('toggleMaximize')"
          >□</button
        >
        <button
          type="button"
          aria-label="隐藏到托盘"
          title="隐藏到托盘"
          @click="windowAction('close')"
          >×</button
        >
      </div>
    </header>
    <div class="app-body">
      <GroupSidebar
        v-if="!settingsOpen"
        v-show="!compact"
        :groups="groups"
        :view="view"
        :disabled="deleting || mutationBusy || busy || !!pendingDelete"
        @select="selectView"
        @changed="groupsChanged"
        @interacting="
          groupInteracting = $event;
          contextMenu = null;
        "
        @menu-open="
          groupMenuOpen = $event;
          if ($event) contextMenu = null;
        "
      >
        <button
          v-if="!compact"
          type="button"
          class="settings-toggle"
          aria-label="设置"
          :disabled="settingsBusy || deleting || busy || organizing || !!pendingDelete"
          @click="toggleSettings"
          ><UiIcon name="settings" /> 设置</button
        >
      </GroupSidebar>
      <section class="main-content">
        <div v-if="!settingsOpen" class="search-tools">
          <div class="searchbox"
            ><UiIcon name="search" /><input
              ref="searchInput"
              v-model="query"
              type="search"
              aria-label="搜索历史"
              placeholder="搜索文本、文件或拼音…"
              autocomplete="off"
              :readonly="mutationBusy"
              :disabled="deleting || groupInteracting || !!pendingDelete"
              @compositionstart="compositionStart"
              @compositionend="compositionEnd"
            /><kbd>Ctrl F</kbd></div
          >
          <label
            v-if="compact"
            class="compact-groups"
            :style="groupColorStyle(typeof view === 'number' ? view : null)"
            >分组<select
              aria-label="分组筛选"
              :value="view"
              :disabled="deleting || organizing || !!pendingDelete"
              @change="selectCompactView"
              ><option value="all">全部分组</option
              ><option value="ungrouped">未分组</option
              ><option v-for="group in groups" :key="group.id" :value="group.id">{{
                group.name
              }}</option></select
            ></label
          >
          <div class="filter-row"
            ><span class="filter-label">类型</span
            ><div class="kind-filters" role="group" aria-label="类型筛选">
              <button
                v-for="option in kinds"
                :key="option.label"
                type="button"
                :aria-pressed="kind === option.value"
                :disabled="deleting || organizing || !!pendingDelete"
                @click="kind = option.value"
                >{{ compact && option.value === null ? '全部类型' : option.label }}</button
              > </div
            ><span class="count">已加载 {{ items.length }} 条</span></div
          >
        </div>

        <div v-if="!settingsOpen && groupError" class="error" role="alert"
          >{{ groupError }} <button type="button" @click="loadGroups">重试分组</button></div
        >
        <div v-if="startupError" class="error" role="alert">{{ startupError }}</div>
        <SettingsView
          v-if="settingsOpen"
          @saved="settings = $event"
          @busy="settingsBusy = $event"
          @recording="settingsRecording = $event"
          @cancel="toggleSettings"
        />
        <div v-if="error" class="error" role="alert">{{ error }}</div>
        <div v-if="!settingsOpen && pageError" class="error" role="alert"
          >{{ pageError }} <button type="button" @click="retryPage">重试</button></div
        >
        <div
          v-if="!settingsOpen && pendingDelete"
          class="delete-confirm"
          role="alertdialog"
          aria-label="确认删除"
          aria-describedby="delete-description"
        >
          <span id="delete-description"
            >将删除 {{ pendingDelete.length }} 条，其中
            {{ protectedDeleteCount }} 条受清理保护</span
          >
          <button type="button" @click="confirmDelete">确认</button
          ><button type="button" @click="pendingDelete = null">取消</button>
        </div>
        <ContextMenu
          v-if="!settingsOpen && contextMenu"
          :key="contextMenu.id + ':' + contextMenu.x + ':' + contextMenu.y"
          :x="contextMenu.x"
          :y="contextMenu.y"
          :groups="groups"
          :all-pinned="allPinned"
          :all-sensitive="allSensitive"
          :files="active?.kind === 'files' && !active.sensitive ? active.files : undefined"
          @open-file="runFileAction('open', $event)"
          @reveal-file="runFileAction('reveal', $event)"
          @sensitive="changeItems('set_sensitive')"
          @pin="changeItems('set_pinned')"
          @move="changeItems('move_to_group', $event)"
          @delete="requestDelete"
          @paste="contextPaste"
          @close="contextMenu = null"
        />
        <section v-if="!settingsOpen && !items.length" class="empty"
          ><span>▤</span
          ><h2>{{ pageLoading ? '正在搜索…' : filtered ? '没有匹配的记录' : '从一次复制开始' }}</h2
          ><p>{{
            filtered ? '试试其他关键词或类型。' : '复制文本、图片或文件后，会显示在这里。'
          }}</p
          ><p v-if="!filtered">{{ quickKey?.label }} 打开快速面板</p></section
        >
        <div v-else-if="!settingsOpen" class="workspace">
          <HistoryList
            ref="historyList"
            :items="items"
            :groups="groups"
            :selected="selected"
            :selected-ids="selectedIds"
            :quick="false"
            :filtered="filtered"
            :loading="pageLoading"
            :has-more="hasMore"
            :disabled="deleting || organizing || !!pendingDelete"
            @choose="choose"
            @context="openContext"
            @paste="paste(false)"
            @scroll="
              contextMenu = null;
              checkLoadMore();
              schedulePreviewGeometry();
            "
            @thumbnail-loaded="invoke('thumbnail_loaded').catch(() => {})"
          />
          <aside v-if="active && !compact" class="detail" aria-label="选中项预览">
            <div ref="detailRoot" class="detail-scroll" :key="active.id">
              <div v-if="active.sensitive" class="sensitive-actions"
                ><span>敏感内容</span
                ><button
                  v-if="!revealed"
                  type="button"
                  :disabled="detailLoading"
                  @click="revealDetail"
                  >显示原文</button
                ><button v-else type="button" @click="loadDetail">隐藏原文</button></div
              >
              <div class="detail-caption detail-header"
                ><span>{{ kindLabel(active) }}预览</span
                ><button
                  type="button"
                  class="preview-refresh icon-button"
                  aria-label="刷新状态"
                  title="刷新状态"
                  :disabled="detailLoading"
                  @click="loadDetail"
                  ><UiIcon name="refresh" /><span class="sr-only">刷新状态</span></button
                ></div
              ><h2 class="preview-title">{{ active.sensitive ? '敏感内容' : active.summary }}</h2>
              <PreviewImage
                v-if="
                  active.kind === 'image' ||
                  active.thumbnail ||
                  active.preview ||
                  active.placeholder
                "
                :hash="active.preview || active.thumbnail"
                :placeholder="active.placeholder"
                large
                @loaded="invoke('preview_loaded').catch(() => {})"
              />
              <p
                v-if="active.kind === 'image' && !active.preview && active.thumbnail"
                class="detail-note"
                >旧记录：显示缩略图</p
              >
              <p v-if="detailLoading" role="status">正在读取详情…</p>
              <p v-else-if="detailError" class="error" role="alert">{{ detailError }}</p>
              <template v-else-if="detail">
                <template v-if="active.kind === 'text' || active.kind === 'rich'">
                  <p v-if="detail.truncated" class="detail-note">仅显示前 20000 个字符</p>
                  <pre class="text-detail">{{ detail.text || '此记录没有纯文本内容' }}</pre>
                </template>
                <ul v-if="active.kind === 'files'" class="file-detail">
                  <li v-for="(file, index) in detail.files" :key="file.path"
                    ><span :class="['file-status', { missing: file.exists !== true }]">{{
                      file.exists === true ? '存在' : file.exists === false ? '已失效' : '无法检查'
                    }}</span
                    ><span>{{ file.path }}</span
                    ><div v-if="!active.sensitive" class="file-actions"
                      ><button
                        type="button"
                        :disabled="fileActionBusy || file.exists === false"
                        @click="runFileAction('open', index)"
                        >打开</button
                      ></div
                    ></li
                  >
                </ul>
              </template>
            </div>
            <div class="detail-bottom">
              <div class="detail-origin"
                ><span v-if="active.source_exe" class="source-name" :title="active.source_exe"
                  >来源：{{ active.source_exe }}</span
                ><span
                  v-if="active.group_id != null"
                  class="detail-group group-badge"
                  :style="groupColorStyle(active.group_id)"
                  >分组：{{ groups.find((g) => g.id === active.group_id)?.name || '分组' }}</span
                ></div
              >
              <div class="detail-actions"
                ><button
                  class="paste-primary"
                  :disabled="busy || deleting || organizing || !!pendingDelete"
                  @click="paste(false)"
                  >粘贴到原窗口</button
                ></div
              >
            </div>
          </aside>
        </div>
      </section>
    </div>
    <footer v-if="!settingsOpen" class="main-statusbar">
      <button
        v-if="compact"
        type="button"
        class="settings-toggle"
        aria-label="设置"
        :disabled="settingsBusy || deleting || busy || organizing || !!pendingDelete"
        @click="toggleSettings"
        ><UiIcon name="settings" /> 设置</button
      >
      <span v-else>本地加密保存</span
      ><span class="keyboard-hint">↑↓ 选择 · Enter 粘贴 · Shift + Enter 纯文本 · Esc 隐藏</span>
      <button
        v-if="compact"
        :disabled="!active || busy || deleting || organizing || !!pendingDelete"
        @click="paste(false)"
        >粘贴选中项 ↵</button
      >
    </footer>
  </main>
</template>

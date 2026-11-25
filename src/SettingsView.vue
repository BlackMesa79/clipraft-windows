<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import DataDirectorySettings from './DataDirectorySettings.vue';
import UiIcon from './UiIcon.vue';
import SettingToggle from './SettingToggle.vue';
import ArchiveSettings from './ArchiveSettings.vue';
import StagingSettings from './StagingSettings.vue';
import BackgroundSettings from './BackgroundSettings.vue';
import AboutSettings from './AboutSettings.vue';
import type { Updates } from './updates';
import type { BackgroundLayout, Backgrounds, MainBackground } from './mainBackground';
import { createHotkeyRecording, type HotkeyField } from './hotkeyRecording';
import {
  shortcutLabel,
  shortcutParts,
  shortcutError,
  tightened,
  type Settings,
  type SettingOptions,
  type HotkeyReport,
} from './settings';
defineProps<{
  fluent: boolean;
  backgrounds: Backgrounds;
  backgroundLayout: BackgroundLayout;
  backgroundLoading: boolean;
  updates: Updates;
}>();
const emit = defineEmits<{
  saved: [settings: Settings];
  backgroundChanged: [layout: BackgroundLayout, background: MainBackground | null];
  busy: [busy: boolean];
  recording: [active: boolean];
  cancel: [];
}>();
const draft = ref<Settings>(),
  original = ref<Settings>(),
  options = ref<SettingOptions>();
const error = ref(''),
  status = ref(''),
  saving = ref(false),
  pending = ref<{ settings: Settings; count: number }>();
const backgroundBusy = ref(false);
const directoryBusy = ref(false);
const busy = computed(() => saving.value || backgroundBusy.value || directoryBusy.value);
const appName = ref('');
const archiveBusy = ref(false);
const cleanupBusy = ref(false);
const categories = [
  { id: 'general', label: '通用' },
  { id: 'appearance', label: '外观' },
  { id: 'history', label: '历史记录' },
  { id: 'privacy', label: '隐私' },
  { id: 'hotkeys', label: '快捷键' },
  { id: 'storage', label: '数据存储' },
  { id: 'about', label: '关于' },
];
const category = ref('general');
const unsavedSettings = computed(
  () => JSON.stringify(draft.value) !== JSON.stringify(original.value),
);
const {
  recordingField,
  recordOperations,
  recordIssues,
  clearRecordIssue,
  record,
  stopRecording,
  blurRecording,
  recordingKeydown,
  receiveRecording,
  isDisposed,
  markDisposed,
} = createHotkeyRecording({ draft, scheduleHotkeyCheck });
function presetChanged(field: HotkeyField) {
  clearRecordIssue(field);
  scheduleHotkeyCheck();
}
let recordStop: UnlistenFn | undefined;
const checkResult = ref<HotkeyReport>(),
  checkError = ref('');
let checkTimer: ReturnType<typeof setTimeout> | undefined,
  checkSequence = 0,
  checkAfterRecording = false;
const hotkeysChanged = computed(
  () =>
    draft.value?.main_hotkey !== original.value?.main_hotkey ||
    draft.value?.quick_hotkey !== original.value?.quick_hotkey,
);
const checkBlocked = computed(
  () =>
    !!checkResult.value &&
    [checkResult.value.main, checkResult.value.quick].some(
      (f) => f.status === 'conflict' || f.status === 'unavailable',
    ),
);
watch(
  () => !!recordingField.value || recordOperations.value > 0,
  (value) => {
    emit('recording', value);
    if (!value && checkAfterRecording && !isDisposed()) scheduleHotkeyCheck();
  },
  { flush: 'sync' },
);

function cancel(): void {
  if (busy.value) return;
  if (recordingField.value || recordOperations.value) {
    void stopRecording();
    return;
  }
  leaveSettings();
}
function leaveSettings(): void {
  if (busy.value) return;
  void stopRecording();
  if (original.value) draft.value = clone(original.value);
  appName.value = '';
  pending.value = undefined;
  error.value = '';
  status.value = '';
  emit('cancel');
}
const clone = (value: Settings): Settings => JSON.parse(JSON.stringify(value));
const fields = [
  { key: 'max_items', label: '最大条数', group: 'history' },
  { key: 'max_bytes', label: '最大空间（MB）', group: 'history' },
  { key: 'max_days', label: '保留天数（0 表示不限）', group: 'history' },
  { key: 'sensitive_ttl_minutes', label: '敏感内容保留时长', group: 'privacy' },
  { key: 'quick_visible_count', label: '快速面板可见条数', group: 'hotkeys' },
] as const;
type NumericKey = (typeof fields)[number]['key'];
function factor(key: NumericKey) {
  return key === 'max_bytes' ? options.value!.bytes_per_mb : 1;
}
function range(key: NumericKey) {
  const r = options.value!.ranges[key],
    unit = factor(key);
  return { min: r.min / unit, max: r.max / unit, step: r.step / unit };
}
function numeric(key: NumericKey, event: Event) {
  draft.value![key] = (event.target as HTMLInputElement).valueAsNumber * factor(key);
}
const hotkeyValidation = computed(() => {
  if (!draft.value || !options.value) return '';
  if (recordingField.value || recordOperations.value) return '正在录制快捷键，按 Esc 或点取消录制';
  for (const field of ['main_hotkey', 'quick_hotkey'] as const) {
    const reason = shortcutError(draft.value[field]);
    if (reason) return reason;
  }
  const a = shortcutParts(draft.value.main_hotkey),
    b = shortcutParts(draft.value.quick_hotkey);
  if (a && b && a[0] === b[0]) return '两个快捷键不能相同或使用完全相同的修饰键';
  return '';
});
interface HotkeyWarning {
  text: string;
  detail: string;
}
function pairHasConflict(settings: Settings): boolean {
  if (shortcutError(settings.main_hotkey) || shortcutError(settings.quick_hotkey)) return false;
  const main = shortcutParts(settings.main_hotkey),
    quick = shortcutParts(settings.quick_hotkey);
  return !!main && !!quick && main[0] === quick[0];
}
function fieldWarning(
  field: HotkeyField,
  resultKey: keyof HotkeyReport,
): HotkeyWarning | undefined {
  const invalid = shortcutError(draft.value![field]);
  if (invalid) return { text: '检测到快捷键冲突', detail: invalid };
  if (pairHasConflict(draft.value!))
    return { text: '检测到快捷键冲突', detail: hotkeyValidation.value };
  const issue = recordIssues.value[field];
  if (issue)
    return { text: issue.conflict ? '检测到快捷键冲突' : '录制失败', detail: issue.detail };
  if (checkError.value) return { text: '检测失败', detail: checkError.value };
  const finding = checkResult.value?.[resultKey];
  if (!finding) return;
  const findingLabels = {
    owned: undefined,
    available: undefined,
    conflict: '检测到快捷键冲突',
    unavailable: '检测失败',
  } satisfies Record<typeof finding.status, string | undefined>;
  const text = findingLabels[finding.status];
  return text ? { text, detail: finding.message } : undefined;
}
const hotkeyWarnings = computed(() => {
  const warnings: Partial<Record<HotkeyField, HotkeyWarning>> = {};
  if (!draft.value || recordingField.value || recordOperations.value) return warnings;
  for (const [field, resultKey] of [
    ['main_hotkey', 'main'],
    ['quick_hotkey', 'quick'],
  ] as const) {
    const warning = fieldWarning(field, resultKey);
    if (warning) warnings[field] = warning;
  }
  return warnings;
});
function validateContentRules(
  rules: Settings['content_ignore_rules'],
  maximumCount: number,
  maximumLength: number,
): string {
  if (rules.length > maximumCount) return `忽略内容规则最多 ${maximumCount} 条`;
  for (const [index, rule] of rules.entries()) {
    const error = contentIgnoreRuleError(rule, index, maximumLength);
    if (error) return error;
  }
  return '';
}
function contentIgnoreRuleError(
  rule: Settings['content_ignore_rules'][number],
  index: number,
  maximumLength = options.value?.content_ignore_rule_length_max ?? 200,
): string {
  if (!rule.value.trim()) return `第 ${index + 1} 条忽略内容规则不能为空`;
  if ([...rule.value].length > maximumLength)
    return `第 ${index + 1} 条忽略内容规则不能超过 ${maximumLength} 个字符`;
  if (rule.mode !== 'regex') return '';
  if (/\\[1-9]|\\k<|\(\?(?:[=!]|<[=!])/.test(rule.value))
    return `第 ${index + 1} 条正则表达式包含不支持的语法`;
  try {
    new RegExp(rule.value);
  } catch {
    return `第 ${index + 1} 条正则表达式无效`;
  }
  return '';
}
const validation = computed(() => {
  if (!draft.value || !options.value) return '';
  if (hotkeyValidation.value) return hotkeyValidation.value;
  for (const field of fields) {
    const value = draft.value[field.key],
      r = options.value.ranges[field.key];
    if (!Number.isSafeInteger(value) || value < r.min || value > r.max)
      return `${field.label}必须在 ${range(field.key).min} 到 ${range(field.key).max} 之间，且字节数或条数为整数`;
  }
  if (appName.value.trim()) return '请先添加忽略程序，或清空输入框';
  return validateContentRules(
    draft.value.content_ignore_rules,
    options.value.content_ignore_rules_max,
    options.value.content_ignore_rule_length_max,
  );
});
function addContentIgnoreRule(): void {
  if (!draft.value || !options.value) return;
  if (draft.value.content_ignore_rules.length >= options.value.content_ignore_rules_max) {
    error.value = `忽略内容规则最多 ${options.value.content_ignore_rules_max} 条`;
    return;
  }
  draft.value.content_ignore_rules.push({ mode: 'keyword', value: '' });
  error.value = '';
}
watch(
  draft,
  () => {
    pending.value = undefined;
    status.value = '';
  },
  { deep: true, flush: 'sync' },
);
watch(busy, (value) => emit('busy', value), { flush: 'sync' });
async function checkHotkeys(): Promise<boolean> {
  if (!draft.value) return false;
  if (recordingField.value || recordOperations.value) {
    checkAfterRecording = true;
    return false;
  }
  if (hotkeyValidation.value) return false;
  clearTimeout(checkTimer);
  const sequence = ++checkSequence;
  checkResult.value = undefined;
  checkError.value = '';
  try {
    const result = await invoke<HotkeyReport>('check_hotkeys', {
      mainHotkey: draft.value.main_hotkey,
      quickHotkey: draft.value.quick_hotkey,
    });
    if (isDisposed() || sequence !== checkSequence) return false;
    checkResult.value = result;
    return !checkBlocked.value;
  } catch (e) {
    if (!isDisposed() && sequence === checkSequence) checkError.value = String(e);
    return false;
  }
}
function prepareHotkeyCheck(): boolean {
  clearTimeout(checkTimer);
  checkSequence++;
  checkResult.value = undefined;
  checkError.value = '';
  if (!draft.value) return false;
  if (recordingField.value || recordOperations.value) {
    checkAfterRecording = true;
    return false;
  }
  checkAfterRecording = false;
  return !hotkeyValidation.value;
}
function scheduleHotkeyCheck(): void {
  if (prepareHotkeyCheck()) checkTimer = setTimeout(() => void checkHotkeys(), 300);
}
function checkInitialHotkeys(): void {
  if (prepareHotkeyCheck()) void checkHotkeys();
}
watch(
  () => [draft.value?.main_hotkey, draft.value?.quick_hotkey],
  ([main, quick], [oldMain, oldQuick]) => {
    if (main === oldMain && quick === oldQuick) return;
    if (main !== oldMain) clearRecordIssue('main_hotkey');
    if (quick !== oldQuick) clearRecordIssue('quick_hotkey');
    if (oldMain === undefined && oldQuick === undefined) {
      checkInitialHotkeys();
      return;
    }
    scheduleHotkeyCheck();
  },
);
function compatibleAppearanceSettings(settings: Settings): Partial<Settings> {
  return {
    theme: settings.theme ?? 'light',
    auto_check_updates: settings.auto_check_updates === true,
    skin: settings.skin ?? 'clipraft',
    layout: settings.layout ?? 'list',
    compact: settings.compact ?? false,
    hide_on_blur: settings.hide_on_blur ?? false,
    select_first_on_show: settings.select_first_on_show ?? false,
  };
}
function compatibleContentSettings(settings: Settings): Partial<Settings> {
  return {
    content_ignore_rules: settings.content_ignore_rules ?? [],
  };
}
function compatibleMainPosition(settings: Settings): Settings['main_position'] {
  if ((settings as { main_position?: string }).main_position === 'input') return 'mouse';
  return settings.main_position ?? 'remember';
}
function compatibleSettings(settings: Settings): Settings {
  return {
    ...settings,
    ...compatibleAppearanceSettings(settings),
    ...compatibleContentSettings(settings),
    main_position: compatibleMainPosition(settings),
  };
}
async function load() {
  error.value = '';
  saving.value = true;
  try {
    const [settings, opts] = await Promise.all([
      invoke<Settings>('get_settings'),
      invoke<SettingOptions>('get_setting_options'),
    ]);
    const compatible = compatibleSettings(settings);
    original.value = clone(compatible);
    draft.value = clone(compatible);
    options.value = opts;
  } catch (e) {
    error.value = String(e);
  } finally {
    saving.value = false;
  }
}
function addApp() {
  const name = appName.value.trim();
  if (!name || !draft.value) return;
  const filename = name.split(/[\\/]/).pop()!.trim();
  // 文件名边界必须拒绝控制字符，保留此正则检查。
  if (
    // eslint-disable-next-line no-control-regex
    !/^[^\\/:*?"<>|\x00-\x1f\x7f-\x9f]+\.exe$/i.test(filename) ||
    new TextEncoder().encode(filename).length > 255
  ) {
    error.value = '请输入有效的 exe 文件名或路径（文件名最多 255 UTF-8 字节）';
    return;
  }
  if (draft.value.ignored_apps.length >= options.value!.ignored_apps_max) {
    error.value = `忽略程序最多 ${options.value!.ignored_apps_max} 个`;
    return;
  }
  if (!draft.value.ignored_apps.some((app) => app.toLowerCase() === name.toLowerCase()))
    draft.value.ignored_apps.push(name);
  appName.value = '';
  error.value = '';
}
async function save() {
  if (busy.value || archiveBusy.value || !draft.value || !original.value || validation.value)
    return;
  error.value = '';
  status.value = '';
  saving.value = true;
  try {
    if (hotkeysChanged.value && !(await checkHotkeys())) {
      category.value = 'hotkeys';
      return;
    }
    const next = clone(draft.value);
    if (tightened(next, original.value)) {
      const count = await invoke<number>('preview_cleanup', {
        policy: { max_items: next.max_items, max_bytes: next.max_bytes, max_days: next.max_days },
      });
      pending.value = { settings: next, count };
    } else await submit(next);
  } catch (e) {
    error.value = String(e);
  } finally {
    saving.value = false;
  }
}
async function submit(next: Settings) {
  const saved = await invoke<Settings>('update_settings', { settings: next });
  clearRecordIssue('main_hotkey');
  clearRecordIssue('quick_hotkey');
  original.value = clone(saved);
  draft.value = clone(saved);
  pending.value = undefined;
  const normalized = JSON.stringify(next.ignored_apps) !== JSON.stringify(saved.ignored_apps);
  status.value = normalized
    ? '设置已保存；忽略程序已规范化（路径转文件名、去除空白及重复项）'
    : '设置已保存';
  emit('saved', saved);
}
async function confirm() {
  if (!pending.value || busy.value || archiveBusy.value) return;
  const next = pending.value.settings;
  saving.value = true;
  error.value = '';
  try {
    await submit(next);
  } catch (e) {
    error.value = String(e);
    pending.value = undefined;
  } finally {
    saving.value = false;
  }
}
onMounted(async () => {
  await load();
  const stop = await listen<{ session: number; hotkey: string | null }>(
    'hotkey-recorded',
    ({ payload }) => receiveRecording(payload),
  );
  if (isDisposed()) {
    stop();
    return;
  }
  recordStop = stop;
  window.addEventListener('blur', blurRecording);
  window.addEventListener('keydown', recordingKeydown, true);
});
onUnmounted(() => {
  markDisposed();
  clearTimeout(checkTimer);
  checkSequence++;
  recordStop?.();
  window.removeEventListener('blur', blurRecording);
  window.removeEventListener('keydown', recordingKeydown, true);
  void stopRecording();
  emit('recording', false);
});
</script>

<template>
  <section class="settings-view" aria-label="设置" :aria-busy="busy">
    <form v-if="draft && options" @submit.prevent="save" novalidate>
      <nav class="settings-nav" aria-label="设置分类">
        <button
          v-for="entry in categories"
          :key="entry.id"
          :disabled="busy"
          type="button"
          :aria-current="category === entry.id ? 'page' : undefined"
          @click="category = entry.id"
          >{{ entry.label }}</button
        >
        <button
          v-if="!fluent"
          type="button"
          class="settings-toggle settings-back"
          :disabled="busy"
          @click="leaveSettings()"
          >返回列表</button
        >
      </nav>
      <div class="settings-pane">
        <label class="settings-category"
          >设置分类<select v-model="category" aria-label="设置分类"
            ><option v-for="entry in categories" :key="entry.id" :value="entry.id">{{
              entry.label
            }}</option></select
          ></label
        >
        <div class="settings-content">
          <fieldset :disabled="busy" v-show="category === 'general'" data-category="general">
            <legend>通用</legend>
            <SettingToggle
              v-model="draft.hide_on_blur"
              :fluent="fluent"
              label="主窗口失去焦点时自动隐藏"
            />
            <SettingToggle v-model="draft.autostart" :fluent="fluent" label="开机自启" />
            <SettingToggle
              v-model="draft.pause_on_start"
              :fluent="fluent"
              label="启动时暂停监听（下次启动生效）"
            />
            <SettingToggle
              v-model="draft.select_first_on_show"
              :fluent="fluent"
              label="每次唤出主窗口时选中第一条记录"
            />
            <p class="detail-note">包含紧凑模式，按当前筛选结果选择；关闭时记忆上次选中记录。</p>
            <label class="number-field settings-select"
              ><span>主窗口唤出位置</span
              ><select v-model="draft.main_position" aria-label="主窗口唤出位置"
                ><option value="remember">记忆上次位置</option
                ><option value="mouse">鼠标光标附近</option></select
              ></label
            >
            <p class="detail-note"
              >普通与紧凑模式共用；只在唤出时定位。空间不足时约束在当前屏幕工作区内。</p
            >
          </fieldset>
          <fieldset :disabled="busy" v-show="category === 'storage'" data-category="storage">
            <legend>数据存储</legend>
            <DataDirectorySettings
              :disabled="busy || archiveBusy || cleanupBusy"
              :unsaved="unsavedSettings"
              @busy="directoryBusy = $event"
            />
            <ArchiveSettings
              :disabled="busy || cleanupBusy"
              :unsaved="unsavedSettings"
              @busy="archiveBusy = $event"
            />
            <StagingSettings :disabled="busy || archiveBusy" @busy="cleanupBusy = $event" />
          </fieldset>
          <fieldset :disabled="busy" v-show="category === 'history'" data-category="history">
            <legend>历史记录</legend>
            <p class="detail-note"
              >空间按 1 MB =
              {{ options.bytes_per_mb }}
              字节计。保存后立即清理；置顶、分组及旧版保留记录受保护，最新一条始终保留。</p
            >
            <label
              v-for="field in fields.filter((f) => f.group === 'history')"
              :key="field.key"
              class="number-field"
            >
              <span>{{ field.label }}</span><input
                :value="draft[field.key] / factor(field.key)"
                type="number"
                v-bind="range(field.key)"
                required
                @input="numeric(field.key, $event)"
              />
              <small>范围 {{ range(field.key).min }}–{{ range(field.key).max }}</small>
            </label>
          </fieldset>
          <fieldset :disabled="busy" v-show="category === 'appearance'" data-category="appearance">
            <legend>外观</legend>
            <div
              class="appearance-section"
              :role="fluent ? 'group' : undefined"
              :aria-labelledby="fluent ? 'appearance-theme-heading' : undefined"
            >
              <h3 v-if="fluent" id="appearance-theme-heading">主题</h3>
              <label class="number-field settings-select"
                ><span class="appearance-label"
                  ><UiIcon class="appearance-icon-slot" name="layers" /><span
                    class="appearance-label-copy"
                    >皮肤<small>窗口与控件的外观</small></span
                  ></span
                ><select v-model="draft.skin" aria-label="皮肤"
                  ><option value="clipraft">素笺</option
                  ><option value="fluent">Fluent</option></select
                ></label
              >
              <label class="number-field settings-select"
                ><span class="appearance-label"
                  ><UiIcon class="appearance-icon-slot" name="image" /><span
                    class="appearance-label-copy"
                    >明暗<small>浅色、深色或跟随 Windows</small></span
                  ></span
                ><select v-model="draft.theme" :aria-label="fluent ? '明暗' : '主题'"
                  ><option value="light">浅色</option
                  ><option value="dark">深色</option
                  ><option value="system">跟随系统</option></select
                ></label
              >
              <label
                v-if="fluent && !backgrounds[backgroundLayout].asset"
                class="number-field backdrop-setting"
                ><span class="appearance-label"
                  ><span class="appearance-label-copy"
                    >背景透出强度<small>提高后可看到更多桌面背景</small></span
                  ></span
                ><span class="backdrop-control"
                  ><input
                    v-model.number="draft.backdrop_strength"
                    type="range"
                    min="0"
                    max="100"
                    step="1"
                    aria-label="背景透出"
                    :style="{
                      '--range-progress': `${draft.backdrop_strength}%`,
                    }"
                  /><output>{{ draft.backdrop_strength }}/100</output></span
                ></label
              >
            </div>
            <BackgroundSettings
              :backgrounds="backgrounds"
              :active-layout="backgroundLayout"
              :disabled="busy || backgroundLoading || archiveBusy || cleanupBusy"
              @busy="backgroundBusy = $event"
              @changed="(layout, asset) => emit('backgroundChanged', layout, asset)"
            />
            <div
              class="appearance-section"
              :role="fluent ? 'group' : undefined"
              :aria-labelledby="fluent ? 'appearance-layout-heading' : undefined"
            >
              <h3 v-if="fluent" id="appearance-layout-heading">布局</h3>
              <label class="number-field settings-select"
                ><span class="appearance-label"
                  ><UiIcon class="appearance-icon-slot" name="layers" /><span
                    class="appearance-label-copy"
                    >常规布局<small>历史记录的排列方式</small></span
                  ></span
                ><select v-model="draft.layout" aria-label="常规布局"
                  ><option value="list">单列列表</option
                  ><option value="cards">双列卡片</option></select
                ></label
              >
              <SettingToggle
                v-model="draft.compact"
                class="compact-setting"
                :fluent="fluent"
                label="紧凑模式（隐藏侧栏与预览）"
                fluent-label="紧凑模式"
                description="隐藏分组侧栏和内嵌预览"
                description-id="compact-setting-description"
                icon="compact"
              />
            </div>
            <p class="detail-note">主题与布局独立；退出紧凑模式恢复常规布局。保存后生效。</p>
          </fieldset>
          <fieldset :disabled="busy" v-show="category === 'privacy'" data-category="privacy">
            <legend>隐私</legend>
            <p class="detail-note"
              >忽略列表与敏感识别保存后对后续复制生效，不删除已有历史。保留时长设为 0
              会取消所有敏感记录的倒计时，普通历史清理规则仍然适用；恢复正数不改变已取消的倒计时。</p
            >
            <SettingToggle
              v-model="draft.sensitive_detection"
              :fluent="fluent"
              label="识别敏感内容"
            />
            <SettingToggle
              v-model="draft.sensitive_high_entropy"
              :fluent="fluent"
              label="高熵字符串识别（可能误报，默认关闭）"
              :disabled="busy"
            />
            <label class="number-field"
              ><span>敏感内容保留时长</span
              ><span class="duration-input"
                ><input
                  v-model.number="draft.sensitive_ttl_minutes"
                  type="number"
                  v-bind="range('sensitive_ttl_minutes')"
                  required
                /><span>分钟</span></span
              ><small>0 为不设敏感删除倒计时；范围 0–1440</small></label
            >
            <p id="ignored-apps-heading">忽略程序（每行一个 exe 名）</p>
            <ul class="ignored-apps" aria-labelledby="ignored-apps-heading"
              ><li v-for="(name, index) in draft.ignored_apps" :key="index"
                ><span>{{ name }}</span
                ><button
                  type="button"
                  :aria-label="'删除 ' + name"
                  @click="draft.ignored_apps.splice(index, 1)"
                  >删除</button
                ></li
              ></ul
            >
            <div class="add-app"
              ><input
                v-model="appName"
                aria-label="新增忽略程序"
                placeholder="exe 文件名或路径"
                @keydown.enter.prevent="addApp"
              /><button type="button" @click="addApp">添加</button></div
            >
            <p id="content-ignore-heading">忽略内容规则</p>
            <ul class="ignored-apps" aria-labelledby="content-ignore-heading">
              <li v-for="(rule, index) in draft.content_ignore_rules" :key="index">
                <select v-model="rule.mode" :aria-label="`第 ${index + 1} 条规则类型`">
                  <option value="keyword">包含关键词</option>
                  <option value="regex">正则表达式</option>
                </select>
                <input
                  v-model="rule.value"
                  :maxlength="options.content_ignore_rule_length_max"
                  :aria-label="`第 ${index + 1} 条规则内容`"
                  :placeholder="rule.mode === 'regex' ? '输入正则表达式' : '输入关键词'"
                />
                <small v-if="contentIgnoreRuleError(rule, index)" class="error">{{
                  contentIgnoreRuleError(rule, index)
                }}</small>
                <button
                  type="button"
                  :aria-label="`删除第 ${index + 1} 条忽略规则`"
                  @click="draft.content_ignore_rules.splice(index, 1)"
                  >删除</button
                >
              </li>
            </ul>
            <button type="button" class="add-content-ignore-rule" @click="addContentIgnoreRule"
              >添加忽略内容规则</button
            >
            <p class="detail-note"
              >命中后整条文本不记录，关键词忽略大小写；规则只影响之后的复制。</p
            >
          </fieldset>
          <fieldset :disabled="busy" v-show="category === 'hotkeys'" data-category="hotkeys">
            <legend>快捷键</legend>
            <label class="number-field"
              ><span>主窗口</span
              ><select
                v-model="draft.main_hotkey"
                aria-label="主窗口快捷键"
                @focus="clearRecordIssue('main_hotkey')"
                @change="presetChanged('main_hotkey')"
                :disabled="!!recordingField"
                ><option v-for="key in options.main_hotkeys" :key="key.id" :value="key.id">{{
                  key.label
                }}</option
                ><option
                  v-if="!options.main_hotkeys.some((k) => k.id === draft?.main_hotkey)"
                  :value="draft.main_hotkey"
                  >{{ shortcutLabel(draft.main_hotkey) }}</option
                ></select
              ><span class="hotkey-record"
                ><button
                  type="button"
                  data-hotkey-record="main_hotkey"
                  :data-hotkey-warning="hotkeyWarnings.main_hotkey ? 'main_hotkey' : undefined"
                  :class="{ 'hotkey-problem': !!hotkeyWarnings.main_hotkey }"
                  :title="hotkeyWarnings.main_hotkey?.detail"
                  :aria-invalid="hotkeyWarnings.main_hotkey ? true : undefined"
                  :aria-label="
                    hotkeyWarnings.main_hotkey
                      ? '主窗口：' + hotkeyWarnings.main_hotkey.text + '，点击重新录制'
                      : undefined
                  "
                  aria-live="polite"
                  :disabled="!!recordingField && recordingField !== 'main_hotkey'"
                  @click="record('main_hotkey')"
                  >{{
                    recordingField === 'main_hotkey'
                      ? '取消录制'
                      : (hotkeyWarnings.main_hotkey?.text ?? '录制主窗口快捷键')
                  }}</button
                ></span
              ></label
            >
            <label class="number-field"
              ><span>快速面板</span
              ><select
                v-model="draft.quick_hotkey"
                aria-label="快速面板快捷键"
                @focus="clearRecordIssue('quick_hotkey')"
                @change="presetChanged('quick_hotkey')"
                :disabled="!!recordingField"
                ><option v-for="key in options.quick_hotkeys" :key="key.id" :value="key.id">{{
                  key.label
                }}</option
                ><option
                  v-if="!options.quick_hotkeys.some((k) => k.id === draft?.quick_hotkey)"
                  :value="draft.quick_hotkey"
                  >{{ shortcutLabel(draft.quick_hotkey) }}</option
                ></select
              ><span class="hotkey-record"
                ><button
                  type="button"
                  data-hotkey-record="quick_hotkey"
                  :data-hotkey-warning="hotkeyWarnings.quick_hotkey ? 'quick_hotkey' : undefined"
                  :class="{ 'hotkey-problem': !!hotkeyWarnings.quick_hotkey }"
                  :title="hotkeyWarnings.quick_hotkey?.detail"
                  :aria-invalid="hotkeyWarnings.quick_hotkey ? true : undefined"
                  :aria-label="
                    hotkeyWarnings.quick_hotkey
                      ? '快速面板：' + hotkeyWarnings.quick_hotkey.text + '，点击重新录制'
                      : undefined
                  "
                  aria-live="polite"
                  :disabled="!!recordingField && recordingField !== 'quick_hotkey'"
                  @click="record('quick_hotkey')"
                  >{{
                    recordingField === 'quick_hotkey'
                      ? '取消录制'
                      : (hotkeyWarnings.quick_hotkey?.text ?? '录制快速面板快捷键')
                  }}</button
                ></span
              ></label
            >
            <p class="detail-note"
              >点录制后按下组合键，Esc取消，30秒自动结束；保存后生效，冲突或保存失败时保留旧设置。</p
            >
            <label class="number-field"
              ><span>快速面板可见条数</span
              ><input
                v-model.number="draft.quick_visible_count"
                type="number"
                v-bind="range('quick_visible_count')"
                required
              /><small
                >范围 {{ range('quick_visible_count').min }}–{{
                  range('quick_visible_count').max
                }}</small
              ></label
            >
            <p class="detail-note"
              >条数在下次打开时生效；偶数条数时上方比下方多一张，选中项始终居中。面板激活期间不能修改快捷键。保存失败时保留原设置。</p
            >
            <p class="detail-note" data-panel-keyboard-help
              >主面板（普通、双列、紧凑）：↑/↓ 选择记录，←/→ 切换类型，Ctrl+↑/↓ 切换分组，Ctrl+F
              返回搜索；Enter 粘贴，Shift+Enter 纯文本粘贴，Esc
              清空搜索或隐藏。搜索框有文字时左右键移动光标，按上下键进入列表后可切换类型。</p
            >
          </fieldset>
          <fieldset v-show="category === 'about'" :disabled="busy" data-category="about">
            <legend>关于</legend>
            <AboutSettings :updates="updates">
              <SettingToggle
                v-model="draft.auto_check_updates"
                :fluent="fluent"
                label="启动时自动检查更新"
              />
            </AboutSettings>
          </fieldset>
        </div>
        <div class="settings-actions">
          <div class="settings-feedback">
            <div v-if="error" class="error" role="alert">{{ error }}</div>
            <p
              v-if="validation && (!hotkeyValidation || recordingField || recordOperations)"
              class="error"
              role="alert"
              >{{ validation }}</p
            >
            <div v-if="pending" class="delete-confirm" role="alertdialog" aria-label="确认立即清理">
              <span
                >将立即删除
                {{ pending.count }} 条记录。预估后新增或到期的记录可能使实际数量变化。</span
              >
              <button
                type="button"
                class="confirm-delete"
                :disabled="busy || archiveBusy"
                @click="confirm"
                >确认保存并清理</button
              ><button type="button" :disabled="busy" @click="pending = undefined">取消</button>
            </div>
            <p v-if="status" role="status">{{ status }}</p>
            <span v-if="!error && !validation && !pending && !status" class="detail-note">{{
              unsavedSettings ? '有未保存的修改，Esc 取消' : '设置未修改，Esc 返回列表'
            }}</span>
          </div>
          <div class="settings-buttons">
            <template
              v-for="action in fluent ? ['cancel', 'save'] : ['save', 'cancel']"
              :key="action"
            >
              <button
                v-if="action === 'save'"
                type="submit"
                :disabled="
                  busy ||
                  archiveBusy ||
                  !!validation ||
                  !!pending ||
                  (hotkeysChanged && checkBlocked) ||
                  (fluent && !unsavedSettings)
                "
                >{{ busy ? '正在处理…' : '保存' }}</button><button
                v-else
                type="button"
                :disabled="busy"
                class="settings-cancel"
                @click="cancel()"
                >{{ recordingField || recordOperations ? '取消录制' : '取消' }}</button
              >
            </template>
          </div>
        </div>
      </div>
    </form>
    <div v-else-if="error" class="error" role="alert"
      >{{ error }} <button type="button" @click="load">重试</button></div
    >
    <p v-else-if="busy" role="status">正在加载设置…</p>
  </section>
</template>

<style scoped>
.settings-view {
  flex: 1;
  min-height: 0;
  overflow: auto;
  padding: 2px 8px 12px 2px;
}
.settings-view form {
  max-width: 850px;
  margin: 0 auto;
}
.settings-view fieldset {
  border: 1px solid var(--line);
  background: var(--bg);
  border-radius: 10px;
  margin: 0 0 16px;
  padding: 16px;
}
.settings-view legend {
  font-weight: 600;
  padding: 0 8px;
}
.settings-view input,
.settings-view select {
  font: inherit;
  color: var(--text);
  background: var(--bg);
  border: 1px solid var(--line);
  border-radius: var(--radius-button);
  padding: 7px;
  min-width: 0;
}
.settings-view :deep(button) {
  border: 1px solid var(--line);
  border-radius: var(--radius-button);
  background: var(--select);
  color: var(--accent);
  padding: 6px 12px;
  cursor: pointer;
}
.settings-view :deep(button:disabled) {
  opacity: var(--opacity-form-disabled);
  cursor: default;
}
.check {
  display: flex;
  align-items: center;
  gap: 8px;
  margin: 8px 0;
}
.appearance-section {
  display: contents;
}
.number-field {
  display: flex;
  align-items: center;
  flex-wrap: wrap;
  gap: 10px;
  margin: 12px 0;
  font-size: var(--font-body-size);
}
.number-field > span {
  width: var(--settings-label-width);
}
.number-field > .duration-input {
  width: auto;
  max-width: 100%;
}
.duration-input > span {
  white-space: nowrap;
}
.number-field input,
.number-field select {
  width: var(--settings-input-width);
}
.number-field small {
  color: var(--muted);
}
.ignored-apps {
  list-style: none;
  padding: 0;
  max-width: 600px;
}
.ignored-apps li {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 12px;
  padding: 4px 0;
  overflow-wrap: anywhere;
}
.add-app {
  display: flex;
  gap: 10px;
}
.settings-actions {
  position: sticky;
  bottom: -12px;
  background: var(--surface);
  padding: 12px 0;
  display: flex;
  gap: 12px;
  align-items: center;
  flex-wrap: wrap;
}
.settings-actions .error,
.settings-actions .delete-confirm {
  width: 100%;
  margin: 0;
}
.settings-actions p {
  margin: 0;
}

.settings-view {
  display: flex;
  flex-direction: column;
  overflow: hidden;
  padding: 0 !important;
  min-width: 0;
}
.settings-view form {
  display: grid;
  grid-template-columns: var(--main-sidebar-width) minmax(0, 1fr);
  grid-template-rows: minmax(0, 1fr) auto;
  flex: 1;
  min-height: 0;
  max-width: none;
  width: 100%;
  margin: 0;
  overflow: hidden;
}
.settings-pane {
  display: contents;
}
.settings-nav {
  grid-column: 1;
  grid-row: 1 / -1;
  width: var(--main-sidebar-width);
  flex-shrink: 0;
  padding: 20px 10px;
  background: var(--surface);
  border-right: 1px solid var(--line);
  overflow: auto;
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.settings-nav button {
  text-align: left;
  padding: 10px;
  background: transparent;
  color: var(--muted);
  border-color: transparent;
}
.settings-nav button[aria-current='page'] {
  background: var(--selected-bg);
  color: var(--selected-text);
}
.settings-category {
  display: none;
}
.settings-content {
  grid-column: 2;
  grid-row: 1;
  flex: 1;
  min-width: 0;
  overflow: auto;
  padding: var(--settings-content-padding);
}
.settings-content fieldset {
  margin: 0;
  border: 0 !important;
  padding: 0 0 24px !important;
}
.settings-content legend {
  padding: 0;
  font-size: var(--font-section-size) !important;
  margin-bottom: 16px;
}
.settings-actions {
  grid-column: 2;
  grid-row: 2;
  display: grid;
  grid-template-columns: minmax(0, 1fr) auto;
  position: static;
  flex-shrink: 0;
  padding: 16px 24px;
  gap: 12px;
  border-top: 1px solid var(--line);
  background: var(--bg);
}
.settings-feedback > [role='status'] {
  width: 100%;
  color: var(--accent);
  font-size: var(--font-label-size);
}
.number-field > .hotkey-record {
  width: auto;
  max-width: 100%;
  min-width: 0;
  display: inline-flex;
  align-items: center;
  gap: 10px;
}
.hotkey-record > button {
  min-width: 11em;
  max-width: 100%;
  flex-shrink: 0;
  white-space: nowrap;
}
.settings-view button.hotkey-problem {
  color: var(--error-text);
  background: var(--error-surface);
  border-color: var(--error-text);
  font-weight: 600;
}
.settings-content input,
.settings-content select {
  max-width: 100%;
}
.add-app input {
  flex: 1;
  width: 0;
}
.ignored-apps span {
  min-width: 0;
}
.settings-nav .settings-back {
  margin-top: auto;
}
.settings-feedback {
  min-width: 0;
  font-size: var(--font-label-size);
  overflow-wrap: anywhere;
}
.settings-feedback > * + * {
  margin-top: 8px;
}
.settings-buttons {
  display: flex;
  align-items: center;
  gap: 10px;
  align-self: center;
}
.settings-buttons button {
  white-space: nowrap;
}
.settings-buttons button[type='submit'] {
  background: var(--primary-bg);
  color: var(--primary-text);
}
@media (max-width: 600px) {
  .settings-view form {
    grid-template-columns: minmax(0, 1fr);
    grid-template-rows: auto minmax(0, 1fr) auto;
  }
  .settings-category {
    grid-column: 1;
    grid-row: 1;
  }
  .settings-content {
    grid-column: 1;
    grid-row: 2;
  }
  .settings-actions {
    grid-column: 1;
    grid-row: 3;
  }
  .settings-nav {
    display: none;
  }
  .settings-category {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 12px;
    border-bottom: 1px solid var(--line);
    font-size: var(--font-label-size);
  }
  .settings-category select {
    flex: 1;
  }
  .settings-content {
    padding: 16px;
  }
  .settings-actions {
    padding: 12px 16px;
  }
  .number-field > span {
    width: 100%;
  }
}
</style>

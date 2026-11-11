<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { tightened, type Settings, type SettingOptions } from './settings';
const emit = defineEmits<{ saved: [settings: Settings]; busy: [busy: boolean] }>();
const draft = ref<Settings>(), original = ref<Settings>(), options = ref<SettingOptions>();
const error = ref(''), status = ref(''), busy = ref(false), pending = ref<{ settings: Settings; count: number }>();
const appName = ref('');
const clone = (value: Settings): Settings => JSON.parse(JSON.stringify(value));
const fields = [
  { key: 'max_items', label: '最大条数', group: 'history' },
  { key: 'max_bytes', label: '最大空间（MB）', group: 'history' },
  { key: 'max_days', label: '保留天数（0 表示不限）', group: 'history' },
  { key: 'sensitive_ttl_minutes', label: '敏感内容保留分钟数', group: 'privacy' },
  { key: 'quick_visible_count', label: '快速面板可见条数（预留）', group: 'hotkeys' },
] as const;
type NumericKey = typeof fields[number]['key'];
function factor(key: NumericKey) { return key === 'max_bytes' ? options.value!.bytes_per_mb : 1; }
function range(key: NumericKey) {
  const r = options.value!.ranges[key], unit = factor(key);
  return { min: r.min / unit, max: r.max / unit, step: r.step / unit };
}
function numeric(key: NumericKey, event: Event) { draft.value![key] = (event.target as HTMLInputElement).valueAsNumber * factor(key); }
const validation = computed(() => {
  if (!draft.value || !options.value) return '';
  for (const field of fields) {
    const value = draft.value[field.key], r = options.value.ranges[field.key];
    if (!Number.isSafeInteger(value) || value < r.min || value > r.max) return `${field.label}必须在 ${range(field.key).min} 到 ${range(field.key).max} 之间，且字节数或条数为整数`;
  }
  if (appName.value.trim()) return '请先添加忽略程序，或清空输入框';
  return '';
});
watch(draft, () => { pending.value = undefined; status.value = ''; }, { deep: true, flush: 'sync' });
watch(busy, value => emit('busy', value), { flush: 'sync' });
async function load() {
  error.value = ''; busy.value = true;
  try {
    const [settings, opts] = await Promise.all([invoke<Settings>('get_settings'), invoke<SettingOptions>('get_setting_options')]);
    original.value = clone(settings); draft.value = clone(settings); options.value = opts;
  } catch (e) { error.value = String(e); }
  finally { busy.value = false; }
}
function addApp() {
  const name = appName.value.trim();
  if (!name || !draft.value) return;
  const filename = name.split(/[\\/]/).pop()!.trim();
  if (!/^[^\\/:*?"<>|\x00-\x1f\x7f-\x9f]+\.exe$/i.test(filename) || new TextEncoder().encode(filename).length > 255) { error.value = '请输入有效的 exe 文件名或路径（文件名最多 255 UTF-8 字节）'; return; }
  if (draft.value.ignored_apps.length >= options.value!.ignored_apps_max) { error.value = `忽略程序最多 ${options.value!.ignored_apps_max} 个`; return; }
  if (!draft.value.ignored_apps.some(app => app.toLowerCase() === name.toLowerCase())) draft.value.ignored_apps.push(name);
  appName.value = ''; error.value = '';
}
async function save() {
  if (busy.value || !draft.value || !original.value || validation.value) return;
  error.value = ''; status.value = ''; busy.value = true;
  try {
    const next = clone(draft.value);
    if (tightened(next, original.value)) {
      const count = await invoke<number>('preview_cleanup', { policy: { max_items: next.max_items, max_bytes: next.max_bytes, max_days: next.max_days } });
      pending.value = { settings: next, count };
    } else await submit(next);
  } catch (e) { error.value = String(e); }
  finally { busy.value = false; }
}
async function submit(next: Settings) {
  const saved = await invoke<Settings>('update_settings', { settings: next });
  original.value = clone(saved); draft.value = clone(saved); pending.value = undefined;
  const normalized = JSON.stringify(next.ignored_apps) !== JSON.stringify(saved.ignored_apps);
  status.value = normalized ? '设置已保存；忽略程序已规范化（路径转文件名、去除空白及重复项）' : '设置已保存'; emit('saved', saved);
}
async function confirm() {
  if (!pending.value || busy.value) return;
  const next = pending.value.settings;
  busy.value = true; error.value = '';
  try { await submit(next); }
  catch (e) { error.value = String(e); pending.value = undefined; }
  finally { busy.value = false; }
}
onMounted(load);
</script>

<template>
  <section class="settings-view" aria-label="设置" :aria-busy="busy">
    <div v-if="error" class="error" role="alert">{{ error }} <button v-if="!draft" type="button" @click="load">重试</button></div>
    <form v-if="draft && options" @submit.prevent="save" novalidate>
      <fieldset :disabled="busy">
        <legend>通用</legend>
        <label class="check"><input v-model="draft.autostart" type="checkbox">开机自启（T8 生效）</label>
        <label class="check"><input v-model="draft.pause_on_start" type="checkbox">启动时暂停监听（T8 生效）</label>
      </fieldset>
      <fieldset :disabled="busy">
        <legend>历史</legend>
        <p class="detail-note">空间按 1 MB = {{ options.bytes_per_mb }} 字节计。保存后立即清理；置顶、收藏、分组记录受保护，最新一条始终保留。</p>
        <label v-for="field in fields.filter(f => f.group === 'history')" :key="field.key" class="number-field">
          <span>{{ field.label }}</span><input :value="draft[field.key] / factor(field.key)" type="number" v-bind="range(field.key)" required @input="numeric(field.key, $event)">
          <small>范围 {{ range(field.key).min }}–{{ range(field.key).max }}</small>
        </label>
      </fieldset>
      <fieldset :disabled="busy">
        <legend>隐私</legend>
        <p class="detail-note">忽略列表保存后对后续复制立即生效，不删除已有历史；敏感内容识别及保留时间将在后续版本生效。</p>
        <label class="check"><input v-model="draft.sensitive_detection" type="checkbox">识别敏感内容</label>
      <label class="check"><input v-model="draft.sensitive_high_entropy" type="checkbox" :disabled="busy">高熵字符串识别（可能误报，默认关闭）</label>
        <label class="number-field"><span>敏感内容保留分钟数</span><input v-model.number="draft.sensitive_ttl_minutes" type="number" v-bind="range('sensitive_ttl_minutes')" required><small>范围 {{ range('sensitive_ttl_minutes').min }}–{{ range('sensitive_ttl_minutes').max }}</small></label>
        <p>忽略程序（每行一个 exe 名）</p>
        <ul class="ignored-apps"><li v-for="(name, index) in draft.ignored_apps" :key="index"><span>{{ name }}</span><button type="button" :aria-label="'删除 ' + name" @click="draft.ignored_apps.splice(index, 1)">删除</button></li></ul>
        <div class="add-app"><input v-model="appName" aria-label="新增忽略程序" placeholder="exe 文件名或路径" @keydown.enter.prevent="addApp"><button type="button" @click="addApp">添加</button></div>
      </fieldset>
      <fieldset :disabled="busy">
        <legend>快捷键</legend>
        <label class="number-field"><span>主窗口</span><select v-model="draft.main_hotkey"><option v-for="key in options.main_hotkeys" :key="key.id" :value="key.id">{{ key.label }}</option></select></label>
        <label class="number-field"><span>快速面板</span><select v-model="draft.quick_hotkey"><option v-for="key in options.quick_hotkeys" :key="key.id" :value="key.id">{{ key.label }}</option></select></label>
        <label class="number-field"><span>快速面板可见条数（仅存储）</span><input v-model.number="draft.quick_visible_count" type="number" v-bind="range('quick_visible_count')" required><small>范围 {{ range('quick_visible_count').min }}–{{ range('quick_visible_count').max }}</small></label>
        <p class="detail-note">面板激活期间不能修改快捷键。保存失败时保留原设置。</p>
      </fieldset>
      <div class="settings-actions">
        <p v-if="validation" class="error" role="alert">{{ validation }}</p>
        <div v-if="pending" class="delete-confirm" role="alertdialog" aria-label="确认立即清理">
          <span>将立即删除 {{ pending.count }} 条记录。预估后新增或到期的记录可能使实际数量变化。</span>
          <button type="button" :disabled="busy" @click="confirm">确认保存并清理</button><button type="button" :disabled="busy" @click="pending = undefined">取消</button>
        </div>
        <p v-if="status" role="status">{{ status }}</p>
        <button type="submit" :disabled="busy || !!validation || !!pending">{{ busy ? '正在处理…' : '保存' }}</button><span class="detail-note">Esc 返回列表，未保存的修改将丢弃</span>
      </div>
    </form>
    <p v-else-if="busy" role="status">正在加载设置…</p>
  </section>
</template>

<style scoped>
.settings-view{flex:1;min-height:0;overflow:auto;padding:2px 8px 12px 2px}.settings-view form{max-width:850px;margin:0 auto}.settings-view fieldset{border:1px solid #d8e3e7;background:#fff;border-radius:10px;margin:0 0 16px;padding:16px}.settings-view legend{font-weight:600;padding:0 8px}.settings-view input,.settings-view select{font:inherit;border:1px solid #b8cdd3;border-radius:5px;padding:7px;min-width:0}.settings-view button{border:1px solid #b8cdd3;border-radius:5px;background:#edf5f5;color:#245f69;padding:6px 12px;cursor:pointer}.settings-view button:disabled{opacity:.5;cursor:default}.check{display:flex;align-items:center;gap:8px;margin:8px 0}.number-field{display:flex;align-items:center;flex-wrap:wrap;gap:10px;margin:12px 0;font-size:13px}.number-field>span{width:210px}.number-field input,.number-field select{width:220px}.number-field small{color:#6b7e89}.ignored-apps{list-style:none;padding:0;max-width:600px}.ignored-apps li{display:flex;justify-content:space-between;align-items:center;gap:12px;padding:4px 0;overflow-wrap:anywhere}.add-app{display:flex;gap:10px}.settings-actions{position:sticky;bottom:-12px;background:#f4f7f8;padding:12px 0;display:flex;gap:12px;align-items:center;flex-wrap:wrap}.settings-actions .error,.settings-actions .delete-confirm{width:100%;margin:0}.settings-actions p{margin:0}
</style>

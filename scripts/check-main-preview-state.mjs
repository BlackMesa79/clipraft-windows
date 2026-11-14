// 真实 Vue refs/nextTick + 合成 IPC/动画帧；不打开窗口或操作桌面。
import assert from 'node:assert/strict';
import { readFile, writeFile, mkdir, rm } from 'node:fs/promises';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import ts from 'typescript';
import { nextTick } from 'vue';

const root = process.cwd();
const temporary = path.resolve(root, '.tools', `main-preview-state-${process.pid}-${Date.now()}`);
assert.equal(path.dirname(temporary), path.resolve(root, '.tools'));
const checks = [];
const requests = [];
const frames = new Map();
let frameId = 0, created = false;
const globals = ['__mainPreviewInvoke', 'requestAnimationFrame', 'cancelAnimationFrame'];
const originals = globals.map(key => [key, Object.getOwnPropertyDescriptor(globalThis, key)]);
function invoke(command, args) {
  return new Promise((resolve, reject) => requests.push({ command, args, resolve, reject }));
}
function latest(command) {
  const request = requests.findLast(request => request.command === command);
  assert(request, `缺少请求 ${command}`);
  return request;
}
function result(text) { return { text, files: [], truncated: false }; }
async function flushGeometry() {
  await nextTick();
  const callbacks = [...frames.values()];
  frames.clear();
  callbacks.forEach(callback => callback(0));
}
async function test(label, run) { await run(); checks.push(label); }
try {
  await mkdir(temporary);
  created = true;
  globalThis.__mainPreviewInvoke = invoke;
  globalThis.requestAnimationFrame = callback => { frames.set(++frameId, callback); return frameId; };
  globalThis.cancelAnimationFrame = id => frames.delete(id);
  const config = JSON.parse(await readFile(path.join(root, 'tsconfig.json'), 'utf8'));
  const converted = ts.convertCompilerOptionsFromJson(config.compilerOptions, root);
  assert.equal(converted.errors.length, 0);
  await writeFile(path.join(temporary, 'ipc.mjs'), 'export const invoke = (...args) => globalThis.__mainPreviewInvoke(...args);');
  for (const name of ['itemDetail', 'compactPreview']) {
    const source = await readFile(path.join(root, 'src', name + '.ts'), 'utf8');
    const compiled = ts.transpileModule(source, { compilerOptions: converted.options }).outputText;
    assert(compiled.includes("'@tauri-apps/api/core'"));
    await writeFile(path.join(temporary, name + '.mjs'), compiled.replace("'@tauri-apps/api/core'", "'./ipc.mjs'"));
  }
  const { createItemDetail } = await import(pathToFileURL(path.join(temporary, 'itemDetail.mjs')));
  const { createCompactPreview } = await import(pathToFileURL(path.join(temporary, 'compactPreview.mjs')));
  let active = { id: 'a', sensitive: true, kind: 'text' }, compact = false, resets = 0;
  const detail = createItemDetail({ active: () => active, compact: () => compact, resetScroll: () => { resets++; } });
  await test('旧 reveal 失败不覆盖新记录的 loading/error，最新详情正常提交', async () => {
    const load = detail.loadDetail(); latest('item_detail').resolve(result('masked')); await load;
    const reveal = detail.revealDetail(), old = latest('reveal_item');
    active = { id: 'b', sensitive: false, kind: 'text' };
    const next = detail.loadDetail(), current = latest('item_detail');
    old.reject(Error('stale reveal')); await reveal;
    assert.equal(detail.detailLoading.value, true); assert.equal(detail.detailError.value, '');
    current.resolve(result('current')); await next;
    assert.equal(detail.detail.value.text, 'current'); assert.equal(detail.revealed.value, false);
  });
  await test('隐藏后的迟到原文成功不恢复正文或 revealed', async () => {
    active = { id: 'a', sensitive: true, kind: 'text' };
    const reveal = detail.revealDetail(), old = latest('reveal_item');
    detail.conceal(); old.resolve(result('SYNTHETIC_RAW')); await reveal;
    assert.equal(detail.detail.value, null); assert.equal(detail.revealed.value, false); assert.equal(detail.detailLoading.value, false);
  });
  await test('旧普通详情失败不覆盖新记录的成功结果', async () => {
    const oldLoad = detail.loadDetail(), old = latest('item_detail');
    active = { id: 'c', sensitive: false, kind: 'text' };
    const load = detail.loadDetail(); latest('item_detail').resolve(result('latest')); await load;
    old.reject(Error('stale detail')); await oldLoad;
    assert.equal(detail.detail.value.text, 'latest'); assert.equal(detail.detailError.value, '');
    detail.invalidateDetail(); assert.equal(detail.detail.value.text, 'latest');
  });
  await test('图片不读文本，紧凑模式只隐藏且不重置详情滚动', async () => {
    const count = requests.length, before = resets;
    active = { id: 'image', sensitive: false, kind: 'image' }; await detail.loadDetail();
    assert.equal(resets, before + 1); assert.equal(detail.detailLoading.value, false);
    compact = true; await detail.loadDetail();
    assert.equal(resets, before + 1); assert.equal(requests.length, count); compact = false;
  });
  await test('两个详情实例的失效代号相互独立', async () => {
    active = { id: 'independent', sensitive: false, kind: 'text' };
    const other = createItemDetail({ active: () => active, compact: () => false, resetScroll() {} });
    const load = other.loadDetail(); detail.conceal(); latest('item_detail').resolve(result('other')); await load;
    assert.equal(other.detail.value.text, 'other'); assert.equal(detail.detail.value, null);
  });
  compact = true;
  let settingsOpen = false;
  const errors = [];
  const preview = createCompactPreview({ active: () => active, compact: () => compact, settingsOpen: () => settingsOpen,
    listRoot: () => ({ getBoundingClientRect: () => ({ top: 30, bottom: 90 }) }),
    recordElement: () => ({ getBoundingClientRect: () => ({ top: 10, bottom: 110 }) }), reportError: error => errors.push(error) });
  await test('相同内容与几何去重，锚点裁剪到可见列表区域', async () => {
    const before = requests.length;
    preview.syncFloatingPreview(); preview.syncFloatingPreview(); await flushGeometry();
    const calls = requests.slice(before);
    assert.deepEqual(calls.map(request => request.command), ['sync_compact_preview', 'position_compact_preview']);
    assert.deepEqual(latest('position_compact_preview').args.anchor, { id: active.id, top: 30, bottom: 90 });
    calls.forEach(request => request.resolve()); preview.syncFloatingPreview(); await flushGeometry();
    assert.equal(requests.length, before + 2);
  });
  await test('旧浮窗失败不报告，当前失败清去重标记并允许重试', async () => {
    active = { ...active, id: 'old' }; preview.syncFloatingPreview(); const old = latest('sync_compact_preview');
    active = { ...active, id: 'new' }; preview.syncFloatingPreview(); const current = latest('sync_compact_preview');
    old.reject(Error('stale')); await nextTick(); assert.deepEqual(errors, []);
    current.reject(Error('current')); await nextTick(); assert.equal(errors.length, 1);
    preview.syncFloatingPreview(); assert.notEqual(latest('sync_compact_preview'), current);
    latest('sync_compact_preview').resolve(); await flushGeometry(); latest('position_compact_preview').resolve();
  });
  await test('几何发送失败后，相同几何可重新发送', async () => {
    active = { ...active, id: 'geometry-retry' }; preview.schedulePreviewGeometry(); await flushGeometry();
    const failed = latest('position_compact_preview'); failed.reject(Error('geometry')); await nextTick();
    preview.schedulePreviewGeometry(); await flushGeometry(); assert.notEqual(latest('position_compact_preview'), failed);
    latest('position_compact_preview').resolve();
  });
  await test('暂停发送 null，隐藏失败不污染提示，取消既有动画帧', async () => {
    preview.suspendFloatingPreview(); const hidden = latest('sync_compact_preview'); assert.equal(hidden.args.id, null);
    hidden.reject(Error('hidden')); await nextTick(); assert.equal(errors.length, 1);
    await flushGeometry();
    assert.equal(latest('position_compact_preview').args.anchor, null); latest('position_compact_preview').resolve();
    settingsOpen = true; preview.floatingActive.value = true; preview.syncFloatingPreview(); await flushGeometry();
    assert.equal(latest('position_compact_preview').args.anchor, null);
    preview.schedulePreviewGeometry(); assert.equal(frames.size, 1);
    preview.cancelPreviewGeometry(); assert.equal(frames.size, 0);
  });
  console.log(JSON.stringify({ passed: checks.length, failed: 0, checks }, null, 2));
} finally {
  for (const [key, descriptor] of originals) {
    if (descriptor) Object.defineProperty(globalThis, key, descriptor); else delete globalThis[key];
  }
  // 只清理本次创建且父目录已验证为项目 .tools 的临时模块。
  assert.equal(path.dirname(temporary), path.resolve(root, '.tools'));
  if (created) await rm(temporary, { recursive: true, force: true });
}

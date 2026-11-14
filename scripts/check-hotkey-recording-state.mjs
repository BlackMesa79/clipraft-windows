// 真实 Vue refs/watch + 合成 IPC/定时器；不安装钩子、不注入键盘或操作桌面。
import assert from 'node:assert/strict';
import { readFile, writeFile, mkdir, rm } from 'node:fs/promises';
import path from 'node:path';
import { pathToFileURL } from 'node:url';
import ts from 'typescript';
import { ref, watch, nextTick } from 'vue';

const root = process.cwd();
const temporary = path.resolve(
  root,
  '.tools',
  `hotkey-recording-state-${process.pid}-${Date.now()}`,
);
assert.equal(path.dirname(temporary), path.resolve(root, '.tools'));
const requests = [],
  checks = [],
  timers = new Map();
let created = false,
  timerId = 0;
const originals = ['__hotkeyRecordingInvoke', 'setTimeout', 'clearTimeout'].map((key) => [
  key,
  Object.getOwnPropertyDescriptor(globalThis, key),
]);
function latest(active) {
  const request = requests.findLast((request) => request.args.active === active);
  assert(request, `缺少 active=${active} 请求`);
  return request;
}
async function test(label, run) {
  await run();
  checks.push(label);
}
async function settle() {
  await nextTick();
  await nextTick();
}
try {
  await mkdir(temporary);
  created = true;
  globalThis.__hotkeyRecordingInvoke = (command, args) => {
    assert.equal(command, 'set_hotkey_recording');
    return new Promise((resolve, reject) => requests.push({ args, resolve, reject }));
  };
  globalThis.setTimeout = (run, delay) => {
    timers.set(++timerId, { run, delay });
    return timerId;
  };
  globalThis.clearTimeout = (id) => timers.delete(id);
  const config = JSON.parse(await readFile(path.join(root, 'tsconfig.json'), 'utf8'));
  const converted = ts.convertCompilerOptionsFromJson(config.compilerOptions, root);
  assert.equal(converted.errors.length, 0);
  await writeFile(
    path.join(temporary, 'ipc.mjs'),
    'export const invoke = (...args) => globalThis.__hotkeyRecordingInvoke(...args);',
  );
  for (const name of ['settings', 'hotkeyRecording']) {
    const source = await readFile(path.join(root, 'src', name + '.ts'), 'utf8');
    const compiled = ts.transpileModule(source, { compilerOptions: converted.options }).outputText;
    await writeFile(
      path.join(temporary, name + '.mjs'),
      compiled
        .replace("'@tauri-apps/api/core'", "'./ipc.mjs'")
        .replace("'./settings'", "'./settings.mjs'"),
    );
  }
  const { createHotkeyRecording } = await import(
    pathToFileURL(path.join(temporary, 'hotkeyRecording.mjs'))
  );
  function fixture() {
    const draft = ref({ main_hotkey: 'alt_v', quick_hotkey: 'ctrl_shift_v' });
    const checks = [];
    return {
      draft,
      checks,
      controller: createHotkeyRecording({ draft, scheduleHotkeyCheck: () => checks.push('check') }),
    };
  }
  async function start(controller, session = 1) {
    const pending = controller.record('main_hotkey');
    latest(true).resolve(session);
    await pending;
  }
  async function stop(controller) {
    const pending = controller.stopRecording();
    latest(false).resolve();
    await pending;
  }
  await test('提前到达的结果仅按启动回复 session 消费，先停止再写草稿', async () => {
    const { controller, draft } = fixture();
    const pending = controller.record('main_hotkey');
    await controller.receiveRecording({ session: 99, hotkey: 'custom:1:67' });
    await controller.receiveRecording({ session: 1, hotkey: 'custom:4:67' });
    assert.equal(draft.value.main_hotkey, 'alt_v');
    latest(true).resolve(1);
    await pending;
    assert.equal(latest(false).args.active, false);
    assert.equal(draft.value.main_hotkey, 'alt_v');
    latest(false).resolve();
    await settle();
    assert.equal(draft.value.main_hotkey, 'custom:4:67');
    assert.equal(controller.recordOperations.value, 0);
  });
  await test('提前到达的 null 是取消结果，不当成未收到结果', async () => {
    const { controller, draft } = fixture();
    const before = requests.length;
    const pending = controller.record('main_hotkey');
    await controller.receiveRecording({ session: 2, hotkey: null });
    latest(true).resolve(2);
    await pending;
    latest(false).resolve();
    await settle();
    assert.equal(requests.length, before + 2);
    assert.equal(draft.value.main_hotkey, 'alt_v');
    assert.equal(controller.recordingField.value, null);
  });
  await test('错误 session 不停止当前会话，当前结果相同仍补做检测', async () => {
    const { controller, checks } = fixture();
    await start(controller, 3);
    const before = requests.length;
    await controller.receiveRecording({ session: 2, hotkey: 'custom:4:67' });
    assert.equal(requests.length, before);
    assert.equal(controller.recordingField.value, 'main_hotkey');
    const finish = controller.receiveRecording({ session: 3, hotkey: 'alt_v' });
    latest(false).resolve();
    await finish;
    assert.deepEqual(checks, ['check']);
  });
  await test('停止在途时预设修订，迟到合法或非法结果均不覆盖', async () => {
    for (const hotkey of ['custom:4:67', 'custom:1:67']) {
      const { controller, draft } = fixture();
      await start(controller);
      const finish = controller.receiveRecording({ session: 1, hotkey });
      draft.value.main_hotkey = 'ctrl_alt_v';
      controller.clearRecordIssue('main_hotkey');
      latest(false).resolve();
      await finish;
      assert.equal(draft.value.main_hotkey, 'ctrl_alt_v');
      assert.equal(controller.recordIssues.value.main_hotkey, undefined);
    }
  });
  await test('开始和停止通信失败仅归属本行，不写入草稿或误报冲突', async () => {
    const { controller, draft } = fixture();
    const failed = controller.record('main_hotkey');
    latest(true).reject(Error('start failure'));
    await failed;
    assert.equal(controller.recordIssues.value.main_hotkey.conflict, false);
    assert.match(controller.recordIssues.value.main_hotkey.detail, /start failure/);
    await start(controller);
    const finish = controller.receiveRecording({ session: 1, hotkey: 'custom:4:67' });
    latest(false).reject(Error('stop failure'));
    await finish;
    assert.equal(controller.recordIssues.value.main_hotkey.conflict, false);
    assert.equal(draft.value.main_hotkey, 'alt_v');
  });
  await test('当前非法结果归为本行冲突，重新选择后清除问题', async () => {
    const { controller, draft } = fixture();
    await start(controller);
    const finish = controller.receiveRecording({ session: 1, hotkey: 'custom:1:67' });
    latest(false).resolve();
    await finish;
    assert.equal(controller.recordIssues.value.main_hotkey.conflict, true);
    assert.match(controller.recordIssues.value.main_hotkey.detail, /保留/);
    assert.equal(controller.recordIssues.value.quick_hotkey, undefined);
    assert.equal(draft.value.main_hotkey, 'alt_v');
    controller.clearRecordIssue('main_hotkey');
    assert.equal(controller.recordIssues.value.main_hotkey, undefined);
  });
  await test('卸载时未完成的启动追加停止，迟到失败不写入问题或复活录制', async () => {
    const { controller } = fixture();
    const pending = controller.record('main_hotkey');
    controller.markDisposed();
    const stopping = controller.stopRecording();
    latest(false).resolve();
    await stopping;
    latest(true).resolve(4);
    await settle();
    latest(false).reject(Error('late cleanup failure'));
    await pending;
    assert.equal(controller.isDisposed(), true);
    assert.equal(controller.recordingField.value, null);
    assert.equal(controller.recordOperations.value, 0);
    assert.deepEqual(controller.recordIssues.value, {});
    const count = requests.length;
    await controller.record('quick_hotkey');
    assert.equal(requests.length, count);
  });
  await test('epoch 已变化的启动结果只补停止，不覆盖后续状态', async () => {
    const { controller, draft } = fixture();
    const pending = controller.record('main_hotkey');
    await stop(controller);
    latest(true).resolve(5);
    await settle();
    latest(false).resolve();
    await pending;
    assert.equal(controller.recordingField.value, null);
    assert.equal(controller.recordOperations.value, 0);
    assert.equal(draft.value.main_hotkey, 'alt_v');
  });
  await test('三十秒超时按字段修订号报告并停止，修订后不残留超时问题', async () => {
    for (const revised of [false, true]) {
      const { controller } = fixture();
      await start(controller);
      const timer = [...timers.values()].at(-1);
      assert.equal(timer.delay, 30_000);
      if (revised) controller.clearRecordIssue('main_hotkey');
      timer.run();
      latest(false).resolve();
      await settle();
      assert.equal(controller.recordingField.value, null);
      assert.equal(
        controller.recordIssues.value.main_hotkey?.detail,
        revised ? undefined : '录制已超时，请重试',
      );
    }
  });
  await test('捕获重复/输入法按键与停止在途输入，Esc 只停止录制', async () => {
    const { controller } = fixture();
    await start(controller);
    const before = requests.length;
    let prevented = 0,
      stopped = 0;
    const event = {
      key: 'c',
      code: 'KeyC',
      altKey: true,
      ctrlKey: false,
      shiftKey: false,
      metaKey: false,
      preventDefault() {
        prevented++;
      },
      stopImmediatePropagation() {
        stopped++;
      },
    };
    controller.recordingKeydown({ ...event, repeat: true });
    controller.recordingKeydown({ ...event, isComposing: true });
    controller.recordingKeydown({ ...event, keyCode: 229 });
    assert.equal(requests.length, before);
    controller.recordingKeydown({ ...event, key: 'Escape' });
    controller.recordingKeydown(event);
    assert.equal(requests.length, before + 1);
    assert.equal(prevented, 5);
    assert.equal(stopped, 5);
    latest(false).resolve();
    await settle();
    controller.recordingKeydown(event);
    assert.equal(prevented, 5);
  });
  await test('同步 watcher 观察到的 field/operations 写入顺序保持，实例之间不共享 disposed', async () => {
    const { controller } = fixture();
    const trace = [];
    const unwatch = watch(
      () => [controller.recordingField.value, controller.recordOperations.value],
      (value) => trace.push(value),
      { flush: 'sync' },
    );
    await start(controller);
    await stop(controller);
    unwatch();
    assert.deepEqual(trace, [
      ['main_hotkey', 0],
      ['main_hotkey', 1],
      ['main_hotkey', 0],
      [null, 0],
      [null, 1],
      [null, 0],
    ]);
    controller.markDisposed();
    const other = fixture().controller;
    await start(other, 6);
    assert.equal(other.isDisposed(), false);
    await stop(other);
  });
  console.log(JSON.stringify({ passed: checks.length, failed: 0, checks }, null, 2));
} finally {
  for (const [key, descriptor] of originals) {
    if (descriptor) Object.defineProperty(globalThis, key, descriptor);
    else delete globalThis[key];
  }
  assert.equal(path.dirname(temporary), path.resolve(root, '.tools'));
  if (created) await rm(temporary, { recursive: true, force: true });
}

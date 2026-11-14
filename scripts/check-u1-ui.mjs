// 离屏渲染 + 合成 IPC 数据；不操作用户桌面，不代表原生粘贴验收。
import { createServer } from 'node:http';
import { spawn } from 'node:child_process';
import { readFile, writeFile, mkdir } from 'node:fs/promises';
import path from 'node:path';
import assert from 'node:assert/strict';
import { settingsMock } from './settings-ui-mock.mjs';
const root = process.cwd();
const out = path.join(root, '.tools/u1-ui');
const profile = path.join(out, 'profile-' + Date.now());
await mkdir(profile, { recursive: true });
const server = createServer(async (req, res) => {
  try {
    const url = new URL(req.url, 'http://127.0.0.1');
    const file = path.join(root, 'dist', url.pathname === '/' ? 'index.html' : url.pathname);
    const content = await readFile(file);
    res.writeHead(200, {
      'Content-Type': file.endsWith('.js')
        ? 'text/javascript'
        : file.endsWith('.css')
          ? 'text/css'
          : 'text/html',
    });
    res.end(content);
  } catch {
    res.writeHead(404);
    res.end();
  }
});
await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
const origin = `http://127.0.0.1:${server.address().port}`;
const browser = spawn(
  'C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe',
  [
    '--headless=new',
    '--disable-gpu',
    '--no-first-run',
    '--no-default-browser-check',
    '--remote-debugging-port=0',
    `--user-data-dir=${profile}`,
    'about:blank',
  ],
  { windowsHide: true, stdio: 'ignore' },
);
const pause = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
async function until(check, message) {
  for (let n = 0; n < 150; n++) {
    if (await check()) return;
    await pause(40);
  }
  throw Error(message);
}
let ws;
try {
  let port;
  await until(async () => {
    try {
      port = (await readFile(path.join(profile, 'DevToolsActivePort'), 'utf8')).split('\n')[0];
      return !!port;
    } catch {
      return false;
    }
  }, '浏览器未启动');
  const targets = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
  ws = new WebSocket(targets.find((target) => target.type === 'page').webSocketDebuggerUrl);
  await new Promise((resolve, reject) => {
    ws.onopen = resolve;
    ws.onerror = reject;
  });
  let seq = 0;
  const pending = new Map();
  ws.onmessage = (event) => {
    const message = JSON.parse(event.data);
    if (message.id) {
      const pair = pending.get(message.id);
      pending.delete(message.id);
      if (message.error) pair?.reject(message.error);
      else pair?.resolve(message.result);
    }
  };
  const send = (method, params = {}) =>
    new Promise((resolve, reject) => {
      const id = ++seq;
      pending.set(id, { resolve, reject });
      ws.send(JSON.stringify({ id, method, params }));
    });
  const evaluate = async (expression) => {
    const result = await send('Runtime.evaluate', {
      expression,
      returnByValue: true,
      awaitPromise: true,
    });
    if (result.exceptionDetails) throw Error(JSON.stringify(result.exceptionDetails));
    return result.result.value;
  };
  const mock = `(() => {
    const callbacks=new Map(),events=new Map();let id=0;
    window.calls=[];window.ready=false;window.failResize=false;
    window.rows=[
      {id:'safe',kind:'text',summary:'合成普通文本',formats:['CF_UNICODETEXT'],files:[]},
      {id:'secret',kind:'text',summary:'sk-S••••••••ef',sensitive:true,formats:['CF_UNICODETEXT'],files:[],thumbnail:null},
      {id:'file',kind:'files',summary:'文件',formats:['CF_HDROP'],files:['C:/synthetic/report.txt'],file_status:'missing'}
    ];
    window.emitEvent=(event,payload)=>callbacks.get(events.get(event))?.({event,id:1,payload});
    window.__TAURI_EVENT_PLUGIN_INTERNALS__={unregisterListener(){}};
    window.__TAURI_INTERNALS__={
      transformCallback(fn){callbacks.set(++id,fn);return id;},convertFileSrc:hash=>'/mock/'+hash,
      async invoke(cmd,args={}) {
        window.calls.push({cmd,args});
        if(cmd==='plugin:event|listen'){events.set(args.event,args.handler);return id;}
        if(cmd==='resize_quick'){if(window.failResize){window.failResize=false;window.resizeFailures=(window.resizeFailures??0)+1;throw Error('合成 resize 失败');}window.resizeSuccess=(window.resizeSuccess??0)+1;return;}
        if(cmd==='panel_state'){if(location.search.includes('fail=panel'))throw Error('合成 panel_state 失败');return {items:window.rows,selected:1,visible:true,error:null};}
        if(cmd==='frontend_ready'){window.ready=true;return;}
        if(cmd==='list_items')return window.rows;
        if(cmd==='list_groups')return [];
        if(cmd==='last_error')return null;
        if(cmd==='item_detail')return {text:'合成详情',files:[],truncated:false};
        return 1;
      }
    };
  })();`;
  await send('Page.enable');
  await send('Runtime.enable');
  await send('Page.addScriptToEvaluateOnNewDocument', {
    source:
      mock +
      settingsMock +
      `
    const invokeWithSettings=window.__TAURI_INTERNALS__.invoke;
    window.__TAURI_INTERNALS__.invoke=async(cmd,args)=>{
      if(cmd==='get_settings'&&location.search.includes('fail=settings'))throw Error('合成 settings 失败');
      return invokeWithSettings(cmd,args);
    };`,
  });
  const checks = [];
  const check = (name) => checks.push(name);
  const navigate = async (query = '') => {
    await send('Page.navigate', { url: origin + '/' + query });
    await until(() => evaluate('window.ready===true'), '未 ready');
  };
  const emit = async (payload) => {
    await evaluate(`window.emitEvent('panel-state',${JSON.stringify(payload)})`);
    await pause(70);
  };
  const panel = (visible, error = null, selected = 1, items = null) => ({
    visible,
    error,
    selected,
    ...(items ? { items } : {}),
  });
  const state = async (visible, error = null, selected = 1, items = null) =>
    emit({ ...panel(visible, error, selected), items: items ?? (await evaluate('window.rows')) });
  const resizeCalls = () =>
    evaluate("window.calls.filter(c=>c.cmd==='resize_quick').map(c=>c.args)");
  const capture = async (name) => {
    const shot = await send('Page.captureScreenshot', { format: 'jpeg', quality: 65 });
    const bytes = Buffer.from(shot.data, 'base64');
    assert(bytes.length <= 300 * 1024, '预览超过 300KB');
    await writeFile(path.join(out, name + '.jpg'), bytes);
  };
  await send('Emulation.setDeviceMetricsOverride', {
    width: 1000,
    height: 700,
    deviceScaleFactor: 1,
    mobile: false,
  });
  // U1-AC1：入口及主窗口原有选择器/事件保持。
  await navigate();
  assert(
    await evaluate(
      "!!document.querySelector('.search-tools')&&!!document.querySelector('.detail')&&!document.querySelector('.quick')",
    ),
  );
  assert.equal((await resizeCalls()).length, 0);
  check('AC1 主窗口入口、列表/详情及无 quick resize');
  await capture('main');
  await evaluate("window.emitEvent('operation-error','合成主窗口操作失败')");
  await until(
    () => evaluate("document.querySelector('[role=alert]')?.textContent==='合成主窗口操作失败'"),
    '主窗口错误未显示',
  );
  check('AC1 operation-error 保持可见');
  await evaluate("window.emitEvent('startup-error','合成启动提示')");
  await pause(40);
  assert(await evaluate("document.body.textContent.includes('合成启动提示')"));
  check('AC1 startup-error 保持可见');
  await evaluate(
    "window.emitEvent('panel-state',{items:[],selected:0,visible:false,error:'不应影响主窗口'})",
  );
  await pause(40);
  assert.equal(await evaluate("document.querySelectorAll('.item').length"), 3);
  check('AC1 主窗口忽略 panel-state');
  await navigate('?panel=other');
  assert.equal(await evaluate("!!document.querySelector('.quick')"), false);
  check('AC1 非 quick 参数仍使用主窗口');
  // U1-AC2：quick 初始化、事件隔离及后端选中状态。
  await send('Emulation.setDeviceMetricsOverride', {
    width: 460,
    height: 530,
    deviceScaleFactor: 1,
    mobile: false,
  });
  await navigate('?panel=quick');
  await pause(80);
  assert.equal(
    await evaluate("document.querySelector('.item.selected .summary').textContent"),
    'sk-S••••••••ef',
  );
  assert.deepEqual(
    await evaluate("window.calls.filter(c=>c.cmd==='plugin:event|listen').map(c=>c.args.event)"),
    ['panel-state', 'settings-changed'],
  );
  assert.equal(
    await evaluate(
      "window.calls.filter(c=>['list_items','list_groups','item_detail','last_error','startup_error'].includes(c.cmd)).length",
    ),
    0,
  );
  assert(
    await evaluate(
      "window.calls.findIndex(c=>c.cmd==='panel_state')<window.calls.findIndex(c=>c.cmd==='frontend_ready')",
    ),
  );
  check('AC2 quick 先订阅再取快照再 ready，不读取主窗口数据');
  assert(
    await evaluate(
      "!document.querySelector('.detail,.settings-toggle,.search-tools,.sensitive-actions')&&getComputedStyle(document.querySelector('.quick')).pointerEvents==='none'",
    ),
  );
  assert(await evaluate("[...document.querySelectorAll('.item')].every(el=>el.tabIndex===-1)"));
  check('AC2 quick 无主窗口控件且不参与鼠标/Tab 交互');
  assert(
    await evaluate(
      "document.querySelector('.sensitive-badge')&&document.querySelectorAll('img').length===0",
    ),
  );
  assert.equal(
    await evaluate(
      "window.calls.filter(c=>['reveal_item','thumbnail_loaded','preview_loaded'].includes(c.cmd)).length",
    ),
    0,
  );
  check('AC3 后端安全视图敏感标记、无原文入口及缩略图请求');
  await capture('quick');
  await state(true, null, 2);
  assert.equal(
    await evaluate("document.querySelector('.item.selected .summary').textContent"),
    'report.txt',
  );
  assert(await evaluate("document.body.textContent.includes('源文件已不存在')"));
  check('AC2 panel-state 更新选择、文件名及失效标记');
  await evaluate(
    "window.emitEvent('settings-changed',{...window.settings,quick_hotkey:'alt_shift_v'})",
  );
  await pause(40);
  assert(await evaluate("document.querySelector('header p').textContent.includes('Alt + Shift')"));
  check('AC2 settings-changed 更新 hold_label');
  await evaluate(
    "window.emitEvent('operation-error','不应显示');window.emitEvent('history-changed');window.emitEvent('open-settings');window.dispatchEvent(new KeyboardEvent('keydown',{key:'Enter'}))",
  );
  await pause(50);
  assert.equal(await evaluate("document.querySelectorAll('.item').length"), 3);
  assert.equal(
    await evaluate(
      "window.calls.filter(c=>['paste_item','hide_main','list_items'].includes(c.cmd)).length",
    ),
    0,
  );
  check('AC2 quick 忽略主窗口事件与 Enter，不发粘贴 IPC');
  // U1-AC4：实际布局驱动 notice 高度，重复消息去重，失败后可重试。
  assert((await resizeCalls()).some((c) => c.height === 530 && c.notice === false));
  const before = (await resizeCalls()).length;
  await state(true);
  await state(true);
  assert.equal((await resizeCalls()).length, before);
  check('AC4 普通高度 530，重复状态不重复 resize');
  await state(true, '合成可见面板错误');
  assert.equal(await evaluate("!!document.querySelector('.notice-only')"), false);
  assert.equal(await evaluate("document.querySelectorAll('.item').length"), 3);
  check('AC4 visible=true 错误保留列表与 footer');
  await state(false, '合成粘贴失败：目标暂不可用');
  assert(
    await evaluate(
      "!!document.querySelector('.notice-only')&&!document.querySelector('.list,.empty,footer,.count')&&document.querySelector('h1').textContent==='操作失败'",
    ),
  );
  let last = (await resizeCalls()).at(-1);
  assert.equal(last.notice, true);
  assert.equal(
    last.height,
    await evaluate("Math.ceil(document.querySelector('main').getBoundingClientRect().height)"),
  );
  await capture('notice');
  check('AC4 hidden+error 切换 notice，按实际渲染高度 resize');
  await state(false, '合成长错误说明，需自动换行。'.repeat(30));
  last = (await resizeCalls()).at(-1);
  assert(last.height <= 530);
  assert.equal(
    last.height,
    await evaluate("Math.ceil(document.querySelector('main').getBoundingClientRect().height)"),
  );
  check('AC4 长错误换行并限制 530 高度');
  await state(true);
  assert.deepEqual((await resizeCalls()).at(-1), { height: 530, notice: false });
  check('AC4 notice 后重新打开恢复普通高度/列表');
  const retryStart = (await resizeCalls()).length;
  const successes = await evaluate('window.resizeSuccess');
  await evaluate('window.failResize=true');
  await state(false, '合成重试提示');
  await state(false, '合成重试提示2');
  assert.equal(await evaluate('window.resizeFailures'), 1);
  assert((await resizeCalls()).length >= retryStart + 2);
  assert((await evaluate('window.resizeSuccess')) > successes);
  check('AC4 resize 拒绝后 observer 或状态更新可重试');
  await state(false, null, 0, []);
  assert(
    await evaluate("!!document.querySelector('.empty')&&!document.querySelector('.notice-only')"),
  );
  await state(true, null, 0, [
    { id: 'only', kind: 'text', summary: '唯一记录', formats: [], files: [] },
  ]);
  assert.equal(await evaluate("document.querySelectorAll('.item.selected').length"), 1);
  check('AC2 空快照及单条选中展示');
  // U1-AC5：初始化失败的错误仍能进入 notice，不错误报告 ready。
  for (const fail of ['panel', 'settings']) {
    await send('Page.navigate', { url: origin + '/?panel=quick&fail=' + fail });
    await until(
      () =>
        evaluate(
          "document.querySelector('.notice-only .error')?.textContent.includes('初始化失败')",
        ),
      '初始化错误未展示',
    );
    assert.equal(await evaluate('window.ready'), false);
    assert((await resizeCalls()).some((c) => c.notice === true));
    check('AC5 ' + fail + ' 初始化失败展示 notice 且不 ready');
  }
  await writeFile(
    path.join(out, 'results.json'),
    JSON.stringify(
      {
        passed: checks.length,
        failed: 0,
        checks,
        scope: 'Edge 无头实际渲染 + 合成 IPC；不证明原生输入法、焦点、透明窗口和粘贴',
      },
      null,
      2,
    ),
  );
  console.log(JSON.stringify({ passed: checks.length, failed: 0, checks }, null, 2));
  await send('Browser.close').catch(() => {});
} finally {
  ws?.close();
  browser.kill();
  await new Promise((resolve) => server.close(resolve));
}

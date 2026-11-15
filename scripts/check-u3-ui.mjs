// 离屏渲染 + 合成 IPC 数据；不操作用户桌面，不代表原生粘贴验收。
import { createServer } from 'node:http';
import { spawn } from 'node:child_process';
import { readFile, writeFile, mkdir } from 'node:fs/promises';
import path from 'node:path';
import assert from 'node:assert/strict';
import { settingsMock } from './settings-ui-mock.mjs';
const root = process.cwd();
const out = path.join(root, '.tools/u3-ui');
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
    window.calls=[];window.ready=false;window.imageRequests=[];
    window.emitEvent=(event,payload)=>callbacks.get(events.get(event))?.({event,id:1,payload});
    window.__TAURI_EVENT_PLUGIN_INTERNALS__={unregisterListener(){}};
    window.__TAURI_INTERNALS__={
      transformCallback(fn){callbacks.set(++id,fn);return id;},
      convertFileSrc(hash){window.imageRequests.push(hash);return '/mock/'+hash;},
      async invoke(cmd,args={}) {
        window.calls.push({cmd,args});
        if(cmd==='plugin:event|listen'){events.set(args.event,args.handler);return id;}
        if(cmd==='panel_state')return new Promise(resolve=>{window.resolveInitial=resolve;});
        if(cmd==='frontend_ready'){window.ready=true;return;}
        return 1;
      }
    };
  })();`;
  await send('Page.enable');
  await send('Runtime.enable');
  await send('Page.addScriptToEvaluateOnNewDocument', { source: mock + settingsMock });
  await send('Emulation.setDeviceMetricsOverride', {
    width: 460,
    height: 530,
    deviceScaleFactor: 1,
    mobile: false,
  });
  await send('Page.navigate', { url: origin + '/?panel=quick' });
  await until(() => evaluate('typeof window.resolveInitial==="function"'), '初始快照未请求');
  const checks = [];
  const check = (name) => checks.push(name);
  let serial = 1;
  const row = (id, extra = {}) => ({
    id,
    kind: 'text',
    summary: '合成记录 ' + id,
    formats: ['CF_UNICODETEXT'],
    files: [],
    ...extra,
  });
  const payload = (overrides = {}) => ({
    serial: serial++,
    session: 1,
    position: 220,
    total: 301,
    loading: false,
    visible: true,
    error: null,
    selected: 4,
    items: Array.from({ length: 9 }, (_, index) => row('id-' + (216 + index))),
    ...overrides,
  });
  const emit = async (panel) => {
    await evaluate(`window.emitEvent('panel-state',${JSON.stringify(panel)})`);
    await pause(40);
  };
  const count = () =>
    evaluate(
      "[...document.querySelectorAll('.count-current,.count-total')].map(el=>el.textContent.trim()).join('/')",
    );
  const selected = () => evaluate("document.querySelector('.item.selected')?.dataset.id");
  const capture = async (name) => {
    const shot = await send('Page.captureScreenshot', { format: 'jpeg', quality: 65 });
    const bytes = Buffer.from(shot.data, 'base64');
    assert(bytes.length <= 300 * 1024);
    await writeFile(path.join(out, name + '.jpg'), bytes);
  };
  const latest = payload();
  await emit(latest);
  await evaluate(`window.resolveInitial(${JSON.stringify(payload({ serial: 0, position: 1 }))})`);
  await until(() => evaluate('window.ready===true'), '前端未 ready');
  assert.equal((await count()).replace(/\s/g, ''), '221/301');
  assert.equal(await selected(), 'id-220');
  check('较新的事件先到、初始 IPC 快照迟到时保持最新选择');
  assert.equal(await evaluate("document.querySelectorAll('.item').length"), 9);
  assert.deepEqual(
    await evaluate("[...document.querySelectorAll('.item .index')].map(el=>el.textContent)"),
    Array.from({ length: 9 }, (_, index) => String(217 + index)),
  );
  check('深页全局位置/总数与局部选中、高亮及行号一致，正文缓存有界');
  const geometry = await evaluate(`(() => {
    const item=document.querySelector('.item.selected').getBoundingClientRect();
    const list=document.querySelector('.list').getBoundingClientRect();
    return {top:item.top,bottom:item.bottom,listTop:list.top,listBottom:list.bottom};
  })()`);
  assert(geometry.top >= geometry.listTop && geometry.bottom <= geometry.listBottom);
  check('实际 460×530 渲染中选中行完整可见');
  await capture('deep-ready');
  await emit(payload({ position: 221, items: [], selected: 0, loading: true }));
  assert.equal(await evaluate("document.querySelectorAll('.item').length"), 0);
  assert(
    await evaluate(
      "!!document.querySelector('[aria-busy=true]')&&document.body.textContent.includes('正在加载')",
    ),
  );
  assert(!(await evaluate("document.body.textContent.includes('从一次复制开始')")));
  check('慢取数时立即清除上一条正文，显示加载状态和最新全局位置');
  await capture('loading');
  await emit(latest);
  assert.equal(await evaluate("document.querySelectorAll('.item').length"), 0);
  check('加载期间旧成功事件不能重新显示上一条正文');
  await emit(payload({ position: 221, items: [], selected: 0, error: '合成邻域查询失败' }));
  assert(
    await evaluate(
      "document.body.textContent.includes('所选记录暂时无法显示')&&document.body.textContent.includes('继续选择可重试')",
    ),
  );
  assert(!(await evaluate("document.body.textContent.includes('从一次复制开始')")));
  check('取数失败区别于空历史，保留位置、取消与重试说明');
  await capture('failed');
  const sensitive = row('secret', {
    sensitive: true,
    summary: '••••••',
    kind: 'image',
    thumbnail: 'must-not-request',
  });
  await emit(payload({ items: [sensitive], position: 230, selected: 0 }));
  assert.equal(await selected(), 'secret');
  assert.equal(await evaluate('window.imageRequests.length'), 0);
  assert.equal(await evaluate("document.querySelectorAll('img').length"), 0);
  assert(await evaluate("!!document.querySelector('.sensitive-badge')"));
  check('敏感安全视图显示打码标记，即使带旧缩略图字段也不请求图片');
  await emit(payload({ items: [], position: 230, selected: 0, loading: true }));
  assert(!(await evaluate("document.body.textContent.includes('••••••')")));
  check('历史安全失效状态立即清除现有摘要与图片节点');
  const closed = payload({ items: [], visible: false, total: 0, position: 0, selected: 0 });
  await emit(closed);
  const reopened = payload({
    session: 2,
    position: 1,
    total: 2,
    selected: 1,
    items: [row('new-0'), row('new-1')],
  });
  await emit(reopened);
  await emit(payload({ serial: closed.serial, session: 1 }));
  assert.equal(await selected(), 'new-1');
  assert.equal((await count()).replace(/\s/g, ''), '2/2');
  check('关闭重开后旧会话事件不能覆盖新会话');
  await emit(
    payload({
      position: 0,
      total: 301,
      selected: 1,
      items: [row('end'), row('first'), row('second')],
    }),
  );
  assert.deepEqual(
    await evaluate("[...document.querySelectorAll('.item .index')].map(el=>el.textContent)"),
    ['301', '1', '2'],
  );
  check('首尾邻域行号双向循环，局部下标不冒充全局位置');
  await emit(
    payload({ items: [], selected: 0, visible: false, error: '记录已删除或到期，请重新选择' }),
  );
  assert(
    await evaluate(
      "!!document.querySelector('.notice-only')&&document.querySelector('[role=alert]').textContent.includes('已删除或到期')",
    ),
  );
  check('提交时记录失效仍通过原有 notice 明确反馈');
  assert.equal(
    await evaluate(
      "window.calls.filter(c=>['list_items','item_detail','paste_item','reveal_item'].includes(c.cmd)).length",
    ),
    0,
  );
  check('前端只展示后端状态，不自行分页取原文或按局部下标粘贴');
  const result = {
    passed: checks.length,
    failed: 0,
    checks,
    scope: 'Edge 无头实际渲染 + 合成 IPC，不代表原生键鼠/焦点/外部粘贴验收',
  };
  await writeFile(path.join(out, 'results.json'), JSON.stringify(result, null, 2));
  console.log(JSON.stringify(result, null, 2));
  await send('Browser.close').catch(() => {});
} finally {
  ws?.close();
  browser.kill();
  await new Promise((resolve) => server.close(resolve));
}

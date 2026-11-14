// 离屏渲染 + 合成 IPC 数据；不操作用户桌面，不代表原生粘贴验收。
import { createServer } from 'node:http';
import { spawn } from 'node:child_process';
import { readFile, writeFile, mkdir } from 'node:fs/promises';
import path from 'node:path';
import assert from 'node:assert/strict';
const root = process.cwd();
const out = path.join(root, '.test-data/m1-ui');
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
    const callbacks = new Map(), events = new Map(); let callbackId = 0;
    window.rows = Array.from({length:250},(_,i)=>({id:'item-'+i,kind:'text',summary:'合成记录 '+i,formats:['CF_UNICODETEXT'],files:[],thumbnail:null,placeholder:null}));
    window.calls = []; window.ready = false; window.failNext = false; window.holdNext = false;
    window.changed = () => callbacks.get(events.get('history-changed'))?.({event:'history-changed',id:1,payload:null});
    window.__TAURI_EVENT_PLUGIN_INTERNALS__ = {unregisterListener(){}};
    window.__TAURI_INTERNALS__ = {
      transformCallback(fn){const id=++callbackId; callbacks.set(id,fn); return id;},
      convertFileSrc: hash => '/mock/'+hash,
      async invoke(cmd,args={}) {
        if(cmd==='plugin:event|listen'){events.set(args.event,args.handler);return 1;}
        if(cmd==='list_items'){
          window.calls.push(args);
          if(window.failNext){window.failNext=false;throw Error('合成分页失败');}
          const result=window.rows.slice(args.offset,args.offset+args.limit).map(item=>({...item}));
          if(window.holdNext){window.holdNext=false;return new Promise(resolve=>window.releasePage=()=>resolve(result));}
          return result;
        }
        if(cmd==='panel_state')return {items:window.rows.slice(0,200),selected:1,visible:true,error:null,serial:0,session:1,position:1,total:window.rows.length,loading:false};
        if(cmd==='last_error')return null;
        if(cmd==='frontend_ready'){window.ready=true;return;}
        if(cmd==='item_detail') {
          const result={text:'详情 '+args.id,truncated:false,files:[]};
          if(window.holdDetail){window.holdDetail=false;return new Promise(resolve=>window.releaseDetail=()=>resolve(result));}
          return result;
        }
        return 1;
      }
    };
  })();`;
  await send('Page.enable');
  await send('Runtime.enable');
  await send('Page.addScriptToEvaluateOnNewDocument', { source: mock });
  await send('Emulation.setDeviceMetricsOverride', {
    width: 1000,
    height: 700,
    deviceScaleFactor: 1,
    mobile: false,
  });
  const checks = [];
  const ready = () => until(() => evaluate('window.ready === true'), '前端未就绪');
  const count = (n) =>
    until(
      () => evaluate(`document.querySelectorAll('.item').length === ${n}`),
      '条目数量不匹配 ' + n,
    );
  const scroll = () =>
    evaluate(
      `(() => {const list=document.querySelector('.list');list.scrollTop=list.scrollHeight;list.dispatchEvent(new Event('scroll'));})()`,
    );
  const text = () => evaluate(`document.querySelector('.text-detail')?.textContent`);
  await send('Page.navigate', { url: origin });
  await ready();
  await count(100);
  assert.deepEqual(await evaluate('window.calls[0]'), {
    filter: { query: '', kind: null },
    offset: 0,
    limit: 100,
  });
  assert(await evaluate(`document.querySelector('.list').getBoundingClientRect().height > 200`));
  checks.push('首屏 100 条并实际完成布局');
  await evaluate(`document.querySelectorAll('.item')[98].click()`);
  await until(async () => (await text()) === '详情 item-98', '初始详情缺失');
  await scroll();
  await count(200);
  assert.equal(
    await evaluate(`document.querySelector('.item.selected .summary').textContent`),
    '合成记录 98',
  );
  assert.equal(await text(), '详情 item-98');
  await scroll();
  await count(250);
  assert(
    await evaluate(`document.querySelector('.list [role=status]').textContent.includes('全部')`),
  );
  checks.push('滚动分页越过 200 条、最后不足一页、追加保持选择与详情');
  await evaluate(`document.querySelectorAll('.item')[225].click()`);
  await until(async () => (await text()) === '详情 item-225', '深页详情缺失');
  await evaluate(
    `window.rows.unshift({...window.rows[0],id:'new',summary:'新复制'});window.changed()`,
  );
  await count(251);
  assert.equal(
    await evaluate(`document.querySelector('.item.selected .summary').textContent`),
    '合成记录 225',
  );
  assert.equal(await text(), '详情 item-225');
  checks.push('历史变化重建已加载范围并保留深页选中项');
  await evaluate(`window.holdDetail=true;document.querySelectorAll('.item')[3].click()`);
  await until(() => evaluate('typeof window.releaseDetail === "function"'), '延迟详情未触发');
  await evaluate(`document.querySelectorAll('.item')[4].click()`);
  await until(async () => (await text()) === '详情 item-3', '新详情缺失');
  await evaluate('window.releaseDetail()');
  await pause(80);
  assert.equal(await text(), '详情 item-3');
  checks.push('旧详情响应不覆盖新选择');
  // 新页面创建独立的 Vue 状态，检查错误重试以及刷新期间的过期追加响应。
  await send('Page.navigate', { url: origin });
  await ready();
  await count(100);
  await evaluate('window.failNext=true');
  await scroll();
  await until(
    () => evaluate(`document.body.textContent.includes('合成分页失败')`),
    '分页错误未显示',
  );
  assert.equal(await evaluate(`document.querySelectorAll('.item').length`), 100);
  await evaluate(
    `Array.from(document.querySelectorAll('button')).find(b=>b.textContent==='重试').click()`,
  );
  await count(200);
  checks.push('分页错误保留已有条目并支持重试');
  await evaluate('window.holdNext=true');
  await scroll();
  await until(() => evaluate('typeof window.releasePage === "function"'), '延迟分页未触发');
  await evaluate(
    `window.rows.unshift({...window.rows[0],id:'new',summary:'新复制'});window.changed()`,
  );
  await until(
    () => evaluate(`document.querySelector('.item .summary')?.textContent === '新复制'`),
    '刷新未应用',
  );
  await evaluate('window.releasePage()');
  await pause(100);
  assert.equal(await evaluate(`document.querySelector('.item .summary').textContent`), '新复制');
  const summaries = await evaluate(
    `Array.from(document.querySelectorAll('.item .summary')).map(n=>n.textContent)`,
  );
  assert.equal(new Set(summaries).size, summaries.length);
  checks.push('刷新丢弃旧分页响应，无重复条目');
  await send('Emulation.setDeviceMetricsOverride', {
    width: 680,
    height: 620,
    deviceScaleFactor: 1,
    mobile: false,
  });
  assert(await evaluate(`document.documentElement.scrollWidth <= 680`));
  checks.push('窄窗口分页布局无横向溢出');
  await send('Page.navigate', { url: origin + '/?panel=quick' });
  await ready();
  await count(200);
  assert.equal(await evaluate('window.calls.length'), 0);
  assert(
    await evaluate(
      `document.querySelector('.quick')!==null && document.querySelector('.detail')===null`,
    ),
  );
  checks.push('快速面板使用缓存，不发分页请求');
  await writeFile(
    path.join(out, 'result.json'),
    JSON.stringify(
      {
        passed: true,
        checks,
        scope: '离屏 Edge + 合成 IPC 数据；未验证原生焦点、输入法、真实粘贴和跨应用图片复制',
      },
      null,
      2,
    ),
  );
  console.log(JSON.stringify({ passed: true, checks }, null, 2));
  await send('Browser.close').catch(() => {});
} finally {
  ws?.close();
  server.close();
  if (browser.exitCode === null) browser.kill();
}

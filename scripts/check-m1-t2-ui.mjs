// 离屏渲染 + 合成 IPC 数据；不操作用户桌面，不代表原生粘贴验收。
import { createServer } from 'node:http';
import { spawn } from 'node:child_process';
import { readFile, writeFile, mkdir } from 'node:fs/promises';
import path from 'node:path';
import assert from 'node:assert/strict';
import { settingsMock } from './settings-ui-mock.mjs';
const root = process.cwd();
const out = path.join(root, '.test-data/m1-t2-ui');
const profile = path.join(out, 'profile-' + Date.now());
await mkdir(profile, { recursive: true });
const server = createServer(async (req, res) => {
  try {
    const url = new URL(req.url, 'http://127.0.0.1');
    const file = path.join(root, 'dist', url.pathname === '/' ? 'index.html' : url.pathname);
    const content = await readFile(file);
    res.writeHead(200, { 'Content-Type': file.endsWith('.js') ? 'text/javascript' : file.endsWith('.css') ? 'text/css' : 'text/html' });
    res.end(content);
  } catch { res.writeHead(404); res.end(); }
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const origin = `http://127.0.0.1:${server.address().port}`;
const browser = spawn('C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe', [
  '--headless=new', '--disable-gpu', '--no-first-run', '--no-default-browser-check', '--remote-debugging-port=0',
  `--user-data-dir=${profile}`, 'about:blank',
], { windowsHide: true, stdio: 'ignore' });
const pause = ms => new Promise(resolve => setTimeout(resolve, ms));
async function until(check, message) {
  for (let n = 0; n < 150; n++) { if (await check()) return; await pause(40); }
  throw Error(message);
}
let ws;
try {
  let port;
  await until(async () => { try { port = (await readFile(path.join(profile, 'DevToolsActivePort'), 'utf8')).split('\n')[0]; return !!port; } catch { return false; } }, '浏览器未启动');
  const targets = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
  ws = new WebSocket(targets.find(target => target.type === 'page').webSocketDebuggerUrl);
  await new Promise((resolve, reject) => { ws.onopen = resolve; ws.onerror = reject; });
  let seq = 0;
  const pending = new Map();
  ws.onmessage = event => {
    const message = JSON.parse(event.data);
    if (message.id) {
      const pair = pending.get(message.id); pending.delete(message.id);
      if (message.error) pair?.reject(message.error); else pair?.resolve(message.result);
    }
  };
  const send = (method, params = {}) => new Promise((resolve, reject) => {
    const id = ++seq; pending.set(id, { resolve, reject }); ws.send(JSON.stringify({ id, method, params }));
  });
  const evaluate = async expression => {
    const result = await send('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true });
    if (result.exceptionDetails) throw Error(JSON.stringify(result.exceptionDetails));
    return result.result.value;
  };
  const mock = `(() => {
    const callbacks = new Map(), events = new Map(); let callbackId = 0;
    window.rows = Array.from({length:250},(_,i)=>({id:'item-'+i,kind:'text',summary:'合成记录 '+i,formats:['CF_UNICODETEXT'],files:[],thumbnail:null,placeholder:null}));
    window.calls = []; window.deletes = []; window.pastes = []; window.hidden = 0; window.ready = false; window.failNext = false; window.holdNext = false;
    window.changed = () => callbacks.get(events.get('history-changed'))?.({event:'history-changed',id:1,payload:null});
    window.emitEvent=(event,payload)=>callbacks.get(events.get(event))?.({event,id:1,payload});
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
        if(cmd==='panel_state')return {items:window.rows.slice(0,200),selected:1,visible:true,error:null};
        if(cmd==='delete_items') {
          window.deletes.push(args.ids);
          if(window.failDelete){window.failDelete=false;throw Error('合成删除失败');}
          if(window.holdDelete){window.holdDelete=false;await new Promise(resolve=>window.releaseDelete=resolve);}
          const before=window.rows.length;
          window.rows=window.rows.filter(row=>!args.ids.includes(row.id));
          window.changed();
          return {deleted_items:before-window.rows.length,reclaimed_blobs:0,failed_blobs:[],maintenance_errors:[]};
        }
        if(cmd==='paste_item'){window.pastes.push(args);return;}
        if(cmd==='hide_main'){window.hidden++;return;}
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
  await send('Page.enable'); await send('Runtime.enable');
  await send('Page.addScriptToEvaluateOnNewDocument', { source: mock + settingsMock });
  await send('Emulation.setDeviceMetricsOverride', { width: 1000, height: 700, deviceScaleFactor: 1, mobile: false });
  const checks = [];
  const ready = () => until(() => evaluate('window.ready === true'), '前端未就绪');
  const count = n => until(() => evaluate(`document.querySelectorAll('.item').length === ${n}`), '条目数量不匹配 ' + n);
  const scroll = () => evaluate(`(() => {const list=document.querySelector('.list');list.scrollTop=list.scrollHeight;list.dispatchEvent(new Event('scroll'));})()`);
  const text = () => evaluate(`document.querySelector('.text-detail')?.textContent`);
  await send('Page.navigate', { url: origin }); await ready(); await count(100);
  assert.deepEqual(await evaluate('window.calls[0]'), { filter: {}, offset: 0, limit: 100 });
  assert(await evaluate(`document.querySelector('.list').getBoundingClientRect().height > 200`));
  checks.push('首屏 100 条并实际完成布局');
  await evaluate(`document.querySelectorAll('.item')[98].click()`);
  await until(async () => (await text()) === '详情 item-98', '初始详情缺失');
  await scroll(); await count(200);
  assert.equal(await evaluate(`document.querySelector('.item.selected .summary').textContent`), '合成记录 98');
  assert.equal(await text(), '详情 item-98');
  await scroll(); await count(250);
  assert(await evaluate(`document.querySelector('.list [role=status]').textContent.includes('全部')`));
  checks.push('滚动分页越过 200 条、最后不足一页、追加保持选择与详情');
  await evaluate(`document.querySelectorAll('.item')[225].click()`);
  await until(async () => (await text()) === '详情 item-225', '深页详情缺失');
  await evaluate(`window.rows.unshift({...window.rows[0],id:'new',summary:'新复制'});window.changed()`);
  await count(251);
  assert.equal(await evaluate(`document.querySelector('.item.selected .summary').textContent`), '合成记录 225');
  assert.equal(await text(), '详情 item-225');
  checks.push('历史变化重建已加载范围并保留深页选中项');
  await evaluate(`window.holdDetail=true;document.querySelectorAll('.item')[3].click()`);
  await until(() => evaluate('typeof window.releaseDetail === "function"'), '延迟详情未触发');
  await evaluate(`document.querySelectorAll('.item')[4].click()`);
  await until(async () => (await text()) === '详情 item-3', '新详情缺失');
  await evaluate('window.releaseDetail()'); await pause(80);
  assert.equal(await text(), '详情 item-3'); checks.push('旧详情响应不覆盖新选择');
  // 新页面创建独立的 Vue 状态，检查错误重试以及刷新期间的过期追加响应。
  await send('Page.navigate', { url: origin }); await ready(); await count(100);
  await evaluate('window.failNext=true'); await scroll();
  await until(() => evaluate(`document.body.textContent.includes('合成分页失败')`), '分页错误未显示');
  assert.equal(await evaluate(`document.querySelectorAll('.item').length`), 100);
  await evaluate(`Array.from(document.querySelectorAll('button')).find(b=>b.textContent==='重试').click()`); await count(200);
  checks.push('分页错误保留已有条目并支持重试');
  await evaluate('window.holdNext=true'); await scroll();
  await until(() => evaluate('typeof window.releasePage === "function"'), '延迟分页未触发');
  await evaluate(`window.rows.unshift({...window.rows[0],id:'new',summary:'新复制'});window.changed()`);
  await until(() => evaluate(`document.querySelector('.item .summary')?.textContent === '新复制'`), '刷新未应用');
  await evaluate('window.releasePage()'); await pause(100);
  assert.equal(await evaluate(`document.querySelector('.item .summary').textContent`), '新复制');
  const summaries = await evaluate(`Array.from(document.querySelectorAll('.item .summary')).map(n=>n.textContent)`);
  assert.equal(new Set(summaries).size, summaries.length);
  checks.push('刷新丢弃旧分页响应，无重复条目');

  // T2：合成 IPC 在事件与命令响应的真实异步顺序下更新历史。
  const press = (key, extras = {}) => evaluate(`window.dispatchEvent(new KeyboardEvent('keydown', {key:${JSON.stringify(key)},bubbles:true,cancelable:true,...${JSON.stringify(extras)}}))`);
  const click = (index, extras = {}) => evaluate(`document.querySelectorAll('.item')[${index}].dispatchEvent(new MouseEvent('click',{bubbles:true,...${JSON.stringify(extras)}}))`);
  const chosen = () => evaluate(`Array.from(document.querySelectorAll('.item.selected .summary')).map(n=>n.textContent)`);
  const fresh = async () => { await send('Page.navigate', {url:origin}); await ready(); await count(100); };
  const settled = () => until(() => evaluate(`document.querySelector('.list')?.getAttribute('aria-busy')==='false'`), '列表没有结束刷新');
  await fresh();
  await click(2); await click(4,{ctrlKey:true});
  assert.deepEqual(await chosen(),['合成记录 2','合成记录 4']);
  await click(2,{ctrlKey:true}); assert.deepEqual(await chosen(),['合成记录 4']);
  await click(5); await click(8,{shiftKey:true});
  assert.deepEqual(await chosen(),['合成记录 5','合成记录 6','合成记录 7','合成记录 8']);
  checks.push('单击单选、Ctrl 切换和 Shift 范围选择');
  await press('a',{ctrlKey:true}); assert.equal((await chosen()).length,100);
  await press('Delete');
  assert(await evaluate(`document.querySelector('.delete-confirm').textContent.includes('将删除 100 条')`));
  assert.equal(await evaluate('window.deletes.length'),0);
  await press('Escape'); assert.equal(await evaluate('window.hidden'),0);
  assert.equal(await evaluate(`document.querySelector('.delete-confirm')`),null);
  await press('Escape'); assert.equal(await evaluate('window.hidden'),1);
  checks.push('Ctrl+A 仅选择已加载 100 条、批量确认、Esc 取消与无确认时隐藏');
  await click(10); await press('Delete'); await settled();
  assert.equal(await evaluate('window.deletes.length'),1);
  assert.equal(await evaluate(`document.querySelector('.delete-confirm')`),null);
  assert.deepEqual(await chosen(),['合成记录 11']);
  assert(!await evaluate(`Array.from(document.querySelectorAll('.summary')).some(n=>n.textContent==='合成记录 10')`));
  await scroll(); await count(200); await scroll(); await count(249);
  const expected = await evaluate('window.rows.map(row=>row.summary)');
  assert.deepEqual(await evaluate(`Array.from(document.querySelectorAll('.item .summary')).map(n=>n.textContent)`),expected);
  checks.push('单条 Delete 无确认、选中后继、后续分页完整且无重复');
  await click(40); await click(43,{shiftKey:true}); await press('Delete');
  assert(await evaluate(`document.querySelector('.delete-confirm').textContent.includes('将删除 4 条')`));
  // 保存一次小尺寸、压缩的真实离屏渲染，供本轮视觉检查。
  const confirmationImage = await send('Page.captureScreenshot',{format:'jpeg',quality:65});
  await writeFile(path.join(out,'confirmation.jpg'),Buffer.from(confirmationImage.data,'base64'));
  await evaluate(`document.querySelector('.delete-confirm button').click()`); await count(245); await settled();
  assert.deepEqual(await chosen(),['合成记录 45']);
  assert.equal(await evaluate('window.deletes.at(-1).length'),4);
  checks.push('批量确认删除并保持原位置后继');
  await evaluate(`document.querySelectorAll('.item')[15].dispatchEvent(new MouseEvent('contextmenu',{bubbles:true,cancelable:true,clientX:995,clientY:695}))`);
  await until(() => evaluate(`document.querySelector('[role=menu]')!==null`), '菜单未出现');
  assert.deepEqual(await chosen(),['合成记录 16']);
  assert(await evaluate(`(()=>{const b=document.querySelector('[role=menu]').getBoundingClientRect();return b.right<=1000&&b.bottom<=700})()`));
  await evaluate(`document.querySelectorAll('[role=menuitem]')[1].click()`);
  assert.deepEqual(await evaluate('window.pastes.at(-1)'),{id:'item-16',plain:false});
  await evaluate(`document.querySelectorAll('.item')[15].dispatchEvent(new MouseEvent('contextmenu',{bubbles:true,cancelable:true,clientX:30,clientY:60}))`);
  await evaluate(`document.querySelector('[role=menuitem]').click()`); await count(244); await settled();
  assert.deepEqual(await chosen(),['合成记录 17']);
  checks.push('HTML 右键菜单选择目标、边界定位、粘贴与删除');
  await click(10); await click(11,{ctrlKey:true});
  await evaluate(`document.querySelectorAll('.item')[10].dispatchEvent(new MouseEvent('contextmenu',{bubbles:true,cancelable:true,clientX:30,clientY:60}))`);
  assert.equal((await chosen()).length,2);
  await evaluate(`document.querySelector('[role=menuitem]').click()`);
  await evaluate(`document.querySelectorAll('.delete-confirm button')[1].click()`);
  assert.equal(await evaluate('window.deletes.length'),3);
  checks.push('右键已选记录保留多选、取消按钮不删除');
  await fresh(); await click(6); await evaluate('window.failDelete=true'); await press('Delete');
  await until(() => evaluate(`document.body.textContent.includes('合成删除失败')`), '删除错误未显示');
  assert.deepEqual(await chosen(),['合成记录 6']); assert.equal(await evaluate('window.rows.length'),250);
  checks.push('删除失败保留历史与选择');
  await evaluate('window.holdNext=true'); await scroll();
  await until(() => evaluate('typeof window.releasePage === "function"'), '延迟追加未开始');
  await press('Delete'); await settled(); await evaluate('window.releasePage()'); await pause(80);
  assert.deepEqual(await chosen(),['合成记录 7']);
  assert(!await evaluate(`Array.from(document.querySelectorAll('.item .summary')).some(n=>n.textContent==='合成记录 6')`));
  checks.push('删除期间丢弃旧分页响应');
  await fresh(); await press('a',{ctrlKey:true}); await press('Delete');
  await evaluate('window.holdDelete=true'); await evaluate(`document.querySelector('.delete-confirm button').click()`);
  await until(() => evaluate('typeof window.releaseDelete === "function"'), '延迟删除未开始');
  await press('Delete'); await press('Enter');
  assert.equal(await evaluate('window.deletes.length'),1); assert.equal(await evaluate('window.pastes.length'),0);
  await evaluate('window.releaseDelete()'); await settled();
  assert.deepEqual(await chosen(),['合成记录 100']);
  await scroll(); await count(150);
  assert.deepEqual(await evaluate(`Array.from(document.querySelectorAll('.item .summary')).map(n=>n.textContent)`),await evaluate('window.rows.map(row=>row.summary)'));
  checks.push('删除互斥、删除全部已加载后从正确 offset 续页');
  await press('a',{ctrlKey:true}); await press('Delete'); await evaluate(`document.querySelector('.delete-confirm button').click()`);
  await until(() => evaluate('window.rows.length===0 && document.querySelectorAll(".item").length===0'), '最后全部删除未清空');
  assert(await evaluate(`document.querySelector('.empty')!==null`));
  checks.push('删除全部历史显示空态');

  await send('Emulation.setDeviceMetricsOverride', { width: 680, height: 620, deviceScaleFactor: 1, mobile: false });
  assert(await evaluate(`document.documentElement.scrollWidth <= 680`));
  checks.push('窄窗口分页布局无横向溢出');
  await send('Page.navigate', { url: origin + '/?panel=quick' }); await ready(); await count(200);
  assert.equal(await evaluate('window.calls.length'), 0);
  assert(await evaluate(`document.querySelector('.quick')!==null && document.querySelector('.detail')===null`));
  await press('Delete'); await press('a',{ctrlKey:true});
  assert.equal(await evaluate('window.deletes.length'),0);
  assert.equal(await evaluate(`document.querySelector('.delete-confirm')`),null);
  checks.push('快速面板使用缓存，不发分页请求且不提供删除');
  await writeFile(path.join(out, 'result.json'), JSON.stringify({ passed: true, checks, scope: '离屏 Edge + 合成 IPC 数据；未验证原生焦点、输入法、真实粘贴和跨应用图片复制' }, null, 2));
  console.log(JSON.stringify({ passed: true, checks }, null, 2));
  await Promise.race([send('Browser.close').catch(() => {}), pause(1000)]);
} finally {
  ws?.close(); server.close(); if (browser.exitCode === null) browser.kill();
}

// 离屏渲染 + 合成 IPC 数据；不操作用户桌面，不代表原生粘贴验收。
import { createServer } from 'node:http';
import { spawn } from 'node:child_process';
import { readFile, writeFile, mkdir } from 'node:fs/promises';
import path from 'node:path';
import assert from 'node:assert/strict';
import { settingsMock } from './settings-ui-mock.mjs';
const root = process.cwd();
const out = path.join(root, '.test-data/m1-t4-ui');
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
    window.rows = Array.from({length:250},(_,i)=>({id:'item-'+i,kind:'text',summary:(i<220?'你好 Hello nihao nh ':'Other ') + i,formats:['CF_UNICODETEXT'],files:[],thumbnail:null,placeholder:null}));
    window.calls = []; window.deletes = []; window.pastes = []; window.hidden = 0; window.ready = false; window.failNext = false; window.holdNext = false;
    window.changed = () => callbacks.get(events.get('history-changed'))?.({event:'history-changed',id:1,payload:null});
    window.emitEvent=(event,payload)=>callbacks.get(events.get(event))?.({event,id:1,payload});
    window.__TAURI_EVENT_PLUGIN_INTERNALS__ = {unregisterListener(){}};
    window.__TAURI_INTERNALS__ = {
      transformCallback(fn){const id=++callbackId; callbacks.set(id,fn); return id;},
      convertFileSrc: hash => '/mock/'+hash,
      async invoke(cmd,args={}) {
        if(cmd==='plugin:event|listen'){events.set(args.event,args.handler);return 1;}
        if(cmd==='list_groups')return [];
        if(cmd==='list_items'){
          window.calls.push(args);
          if(window.failNext){window.failNext=false;throw Error('合成分页失败');}
          const terms=(args.filter?.query??'').trim().toLowerCase().split(/\\s+/).filter(Boolean);
          const filtered=window.rows.filter(item=>terms.every(term=>item.summary.toLowerCase().includes(term)) && (!args.filter?.kind || (args.filter.kind==='text' ? ['text','rich'].includes(item.kind) : args.filter.kind==='link' ? item.summary.startsWith('https://') : item.kind===args.filter.kind)));
          const result=filtered.slice(args.offset,args.offset+args.limit).map(item=>({...item}));
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
  const checks=[];
  const ready=()=>until(()=>evaluate('window.ready===true'),'前端未就绪');
  const count=n=>until(()=>evaluate(`document.querySelectorAll('.item').length===${n}`),'条目数错误 '+n);
  const chosen=()=>evaluate("Array.from(document.querySelectorAll('.item.selected .summary')).map(n=>n.textContent)");
  const settled=()=>until(()=>evaluate("document.querySelector('.list')?.getAttribute('aria-busy')==='false'"),'刷新未完成');
  const input=async value=>{await evaluate(`(()=>{const el=document.querySelector('input[type=search]');el.focus();el.value=${JSON.stringify(value)};el.dispatchEvent(new Event('input',{bubbles:true}));})()`);};
  const press=(key,extras={},target="document.activeElement")=>evaluate(`${target}.dispatchEvent(new KeyboardEvent('keydown',{key:${JSON.stringify(key)},bubbles:true,cancelable:true,...${JSON.stringify(extras)}}))`);
  const click=(i,extras={})=>evaluate(`document.querySelectorAll('.item')[${i}].dispatchEvent(new MouseEvent('click',{bubbles:true,...${JSON.stringify(extras)}}))`);
  const scroll=()=>evaluate("(()=>{const el=document.querySelector('.list');el.scrollTop=el.scrollHeight;el.dispatchEvent(new Event('scroll'));})()");
  const fresh=async()=>{await send('Page.navigate',{url:origin});await ready();await count(100);};
  await fresh();
  assert(await evaluate("document.activeElement===document.querySelector('input[type=search]')"));
  await evaluate("document.querySelector('.settings-toggle').focus();window.emitEvent('main-shown',null)");
  await until(()=>evaluate("document.activeElement===document.querySelector('input[type=search]')"),'显示事件未聚焦');
  checks.push('初始打开与 main-shown 事件在 nextTick 后聚焦搜索框');
  await input('H');await input('He');await input('Hello');await pause(70);
  assert.equal(await evaluate('window.calls.length'),1);
  await count(100); await settled();assert.equal(await evaluate('window.calls.length'),2);
  assert.deepEqual(await evaluate('window.calls.at(-1)'),{filter:{query:'Hello',kind:null},offset:0,limit:100});
  assert.deepEqual(await chosen(),['你好 Hello nihao nh 0']);
  checks.push('150ms 防抖，只发送最终查询并选中第一页首条');
  await scroll();await count(200);await scroll();await count(220);
  assert(await evaluate("window.calls.every(c=>!c.filter.query||c.filter.query==='Hello')"));
  assert(await evaluate("document.querySelector('.list [role=status]').textContent.includes('全部匹配记录')"));
  checks.push('搜索结果跨页 220 条，无重复或遗漏');
  await press('ArrowDown');assert.deepEqual(await chosen(),['你好 Hello nihao nh 1']);
  await press('ArrowUp');assert.deepEqual(await chosen(),['你好 Hello nihao nh 0']);
  await press('Enter');await press('Enter',{shiftKey:true});
  assert.deepEqual(await evaluate('window.pastes'),[{id:'item-0',plain:false},{id:'item-0',plain:true}]);
  assert.equal(await press('Delete'),true);assert.equal(await evaluate('window.deletes.length'),0);
  assert.equal(await press('a',{ctrlKey:true}),true);assert.equal((await chosen()).length,1);
  checks.push('搜索框方向键/Enter/Shift+Enter 操作列表，非空 Delete 与 Ctrl+A 保留文本编辑');
  await evaluate("document.querySelector('input').dispatchEvent(new CompositionEvent('compositionstart',{bubbles:true}))");
  for(const key of ['ArrowDown','ArrowUp','Enter','Delete','Escape'])await press(key,{isComposing:true});
  await input('你好');await pause(200);assert.equal(await evaluate('window.calls.at(-1).filter.query'),'Hello');
  await press('Enter');assert.equal(await evaluate('window.pastes.length'),2);
  await evaluate("document.querySelector('input').dispatchEvent(new CompositionEvent('compositionend',{bubbles:true}))");
  await count(100);assert.equal(await evaluate('window.calls.at(-1).filter.query'),'你好');
  checks.push('组合输入期间不提交查询、不执行快捷键，结束后查询中文');
  await press('Escape');await count(100);assert.equal(await evaluate('window.hidden'),0);assert.equal(await evaluate("document.querySelector('input').value"),'');
  await press('Escape');assert.equal(await evaluate('window.hidden'),1);
  await press('Delete');await settled();assert.equal(await evaluate('window.deletes.length'),1);
  checks.push('Esc 先清空再隐藏，空搜索框 Delete 删除记录');
  await input('不存在');await until(()=>evaluate("document.body.textContent.includes('没有匹配的记录')"),'没有匹配空态缺失');
  await input('nihao');await count(100);assert.equal(await evaluate('window.calls.at(-1).offset'),0);
  checks.push('无结果空态与拼音查询请求');
  await evaluate("window.rows.push({id:'rich',kind:'rich',summary:'nihao rich',formats:[],files:[],thumbnail:null,placeholder:null},{id:'link',kind:'text',summary:'https://example.com/nihao',formats:[],files:[],thumbnail:null,placeholder:null})");
  await evaluate("Array.from(document.querySelectorAll('.kind-filters button')).find(b=>b.textContent==='文本').click()");await count(100);
  assert.equal(await evaluate('window.calls.at(-1).filter.kind'),'text');
  await evaluate("Array.from(document.querySelectorAll('.kind-filters button')).find(b=>b.textContent==='链接').click()");await count(1);assert.deepEqual(await chosen(),['https://example.com/nihao']);
  await evaluate("Array.from(document.querySelectorAll('.kind-filters button')).find(b=>b.textContent==='图片').click()");await count(0);
  await evaluate("Array.from(document.querySelectorAll('.kind-filters button')).find(b=>b.textContent==='文件').click()");await pause(100);
  assert.equal(await evaluate('window.calls.at(-1).filter.kind'),'files');
  await evaluate("Array.from(document.querySelectorAll('.kind-filters button')).find(b=>b.textContent==='全部').click()");await count(100);
  checks.push('五种分段筛选与搜索组合，每次重置分页和选择');
  await click(0);await click(1,{ctrlKey:true});await press('Delete',{},'window');
  assert(await evaluate("!!document.querySelector('.delete-confirm')"));await press('Escape',{},'window');
  assert.equal(await evaluate("document.querySelector('input').value"),'nihao');assert.equal(await evaluate('window.hidden'),1);
  await press('Delete',{},'window');await evaluate("document.querySelector('.delete-confirm button').click()");await settled();
  assert.equal(await evaluate('window.deletes.at(-1).length'),2);assert.equal(await evaluate('window.calls.at(-1).filter.query'),'nihao');
  checks.push('搜索结果多选删除按当前过滤重建；确认条 Esc 优先于搜索');
  await evaluate("document.querySelector('.item').dispatchEvent(new MouseEvent('contextmenu',{bubbles:true,cancelable:true,clientX:20,clientY:160}))");
  await until(()=>evaluate("!!document.querySelector('[role=menu]')"),'右键菜单缺失');await press('Escape',{},'window');
  assert.equal(await evaluate("document.querySelector('[role=menu]')"),null);assert.equal(await evaluate("document.querySelector('input').value"),'nihao');
  assert.equal(await evaluate('window.hidden'),1);
  checks.push('右键菜单 Esc 优先关闭菜单，不清空搜索或隐藏窗口');
  await evaluate("document.querySelector('.settings-toggle').click()");await press('Escape',{},'window');
  assert(await evaluate("!!document.querySelector('input[type=search]')"));assert.equal(await evaluate("document.querySelector('input').value"),'nihao');
  checks.push('设置页 Esc 返回列表，保留搜索并恢复输入焦点');
  await fresh();await evaluate('window.holdNext=true');await scroll();
  await until(()=>evaluate('typeof window.releasePage===\"function\"'),'延迟分页未开始');
  await input('Other');await count(30);await evaluate('window.releasePage()');await pause(100);await count(30);
  assert.deepEqual(await chosen(),['Other 220']);checks.push('筛选变更后丢弃过期分页响应');
  await fresh();await click(99);await evaluate('window.holdNext=true');
  // 不等待处理器完成：模拟方向键等待分页时用户立即输入新查询。
  await press('ArrowDown',{},'window');await until(()=>evaluate('typeof window.releasePage===\"function\"'),'方向键分页未开始');
  await input('Other');await count(30);await evaluate('window.releasePage()');await pause(100);
  assert.deepEqual(await chosen(),['Other 220']);checks.push('等待旧分页的方向键不会移动新搜索的首条选择');
  await fresh();await evaluate('window.failNext=true');await scroll();
  await until(()=>evaluate("!!document.querySelector('.error button')"),'分页重试按钮未出现');
  await evaluate("document.querySelector('.error button').focus()");
  assert.equal(await press('Enter',{},'document.activeElement'),true);
  assert.equal(await press(' ',{},'document.activeElement'),true);
  assert.equal(await evaluate('window.pastes.length'),0);
  // CDP 按键仅发给隔离的 headless 页面，验证浏览器默认激活确实执行重试。
  await send('Input.dispatchKeyEvent',{type:'keyDown',key:'Enter',code:'Enter',windowsVirtualKeyCode:13,text:'\r',unmodifiedText:'\r'});
  await send('Input.dispatchKeyEvent',{type:'keyUp',key:'Enter',code:'Enter',windowsVirtualKeyCode:13});
  await count(200);assert.equal(await evaluate('window.pastes.length'),0);
  checks.push('分页失败且有选中项时，重试按钮 Enter/Space 不被拦截，原生 Enter 激活重试且不粘贴');
  for (const selector of ['.settings-toggle','.kind-filters button','.detail-header button','.detail-actions .paste-primary']) {
    await evaluate(`document.querySelector(${JSON.stringify(selector)}).focus()`);
    for (const key of ['Enter',' ']) assert.equal(await press(key,{},'document.activeElement'),true,selector);
  }
  for (const tag of ['select','a','textarea']) {
    await evaluate(`(()=>{const el=document.createElement(${JSON.stringify(tag)});el.id='native-control';el.tabIndex=0;document.body.append(el);el.focus()})()`);
    for (const key of ['Enter',' ']) assert.equal(await press(key,{},'document.activeElement'),true,tag);
    await evaluate("document.querySelector('#native-control').remove()");
  }
  assert.equal(await evaluate('window.pastes.length'),0);
  await evaluate("document.querySelector('.item').focus()");
  const focusedId=await evaluate('document.activeElement.dataset.id');
  assert.equal(await press(' ',{},'document.activeElement'),true);
  assert.equal(await press('Enter',{},'document.activeElement'),false);
  assert.equal(await evaluate('window.pastes.length'),1);
  assert.equal(await evaluate('window.pastes.at(-1).id'),focusedId);
  checks.push('记录按钮Space保留原生行为，Enter粘贴所聚焦记录且不触发重复原生点击');
  await evaluate("document.querySelector('.settings-toggle').click()");
  await until(()=>evaluate("!!document.querySelector('.settings-view input')"),'设置输入框未出现');
  await evaluate("document.querySelector('.settings-view input').focus()");
  for (const key of ['Enter',' ']) assert.equal(await press(key,{},'document.activeElement'),true);
  assert.equal(await evaluate('window.pastes.length'),1);
  await press('Escape',{},'window');
  await evaluate("document.activeElement.blur()");
  assert.equal(await press('Enter',{},'window'),false);
  assert.equal(await evaluate('window.pastes.length'),2);
  await evaluate("const list=document.querySelector('.list');list.tabIndex=0;list.focus()");
  assert.equal(await press('Enter',{},'document.activeElement'),false);
  assert.equal(await evaluate('window.pastes.length'),3);
  checks.push('按钮/select/a/textarea/设置输入框保留 Enter/Space；列表容器和无焦点 Enter 仍粘贴');
  const shot=await send('Page.captureScreenshot',{format:'jpeg',quality:65});await writeFile(path.join(out,'search.jpg'),Buffer.from(shot.data,'base64'));
  await send('Emulation.setDeviceMetricsOverride',{width:680,height:620,deviceScaleFactor:1,mobile:false});
  assert(await evaluate('document.documentElement.scrollWidth<=680'));checks.push('离屏实际渲染与窄窗口无横向溢出');
  await send('Page.navigate',{url:origin+'/?panel=quick'});await ready();await count(200);
  assert.equal(await evaluate("document.querySelector('input[type=search]')"),null);assert.equal(await evaluate('window.calls.length'),0);
  checks.push('快速面板只读原缓存，不显示搜索或筛选');
  await writeFile(path.join(out,'results.json'),JSON.stringify({passed:checks.length,checks},null,2));console.log(JSON.stringify({passed:checks.length,checks},null,2));
} finally {
  ws?.close();browser.kill();await new Promise(resolve=>server.close(resolve));
}

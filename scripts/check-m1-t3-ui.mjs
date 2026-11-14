// 离屏渲染 + 合成 IPC 数据；不操作用户桌面，不代表原生粘贴验收。
import { createServer } from 'node:http';
import { spawn } from 'node:child_process';
import { readFile, writeFile, mkdir } from 'node:fs/promises';
import path from 'node:path';
import assert from 'node:assert/strict';
import { settingsMock } from './settings-ui-mock.mjs';
const root = process.cwd();
const out = path.join(root, '.test-data/m1-t3-ui');
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
  await send('Page.navigate', {url:origin}); await ready(); await count(100);
  const press = key => evaluate(`window.dispatchEvent(new KeyboardEvent('keydown',{key:${JSON.stringify(key)},bubbles:true,cancelable:true}))`);
  const open = async () => { await evaluate(`document.querySelector('.settings-toggle').click()`); await until(()=>evaluate(`!!document.querySelector('.settings-view form')`),'设置页未加载'); };
  const input = (index,value) => evaluate(`(()=>{const n=document.querySelectorAll('.settings-view input[type=number]')[${index}];n.value=${JSON.stringify(String(value))};n.dispatchEvent(new Event('input',{bubbles:true}));})()`);
  const save = () => evaluate(`document.querySelector('.settings-view button[type=submit]').click()`);
  await open();
  assert.equal(await evaluate(`document.querySelectorAll('.settings-view fieldset').length`),5);
  assert.equal(await evaluate(`[...document.querySelectorAll('.settings-view select')].find(n=>n.querySelector('option[value=alt_v]')).options.length`),4);
  assert.equal(await evaluate(`[...document.querySelectorAll('.settings-view select')].find(n=>n.querySelector('option[value=ctrl_shift_v]')).options.length`),3);
  assert.equal(await evaluate(`document.querySelectorAll('.settings-view input[type=number]')[1].value`),'1024');
  checks.push('五组设置（含外观）、后端候选及 MB 单位正确显示');
  await press('Delete'); await press('Enter');
  assert.equal(await evaluate('window.deletes.length+window.pastes.length'),0);
  await press('Escape'); await count(100);
  assert.equal(await evaluate('window.hidden'),0); await open();
  checks.push('设置页键盘隔离，Esc 只返回列表');
  await input(0,9);
  assert(await evaluate(`document.querySelector('.settings-actions .error').textContent.includes('10')`));
  assert(await evaluate(`document.querySelector('button[type=submit]').disabled`));
  await input(0,''); assert(await evaluate(`document.querySelector('button[type=submit]').disabled`));
  await input(0,1000);
  checks.push('数字范围及空值校验，非法值阻止提交');
  await evaluate(`(()=>{const n=document.querySelector('.add-app input');n.value='测试程序.exe';n.dispatchEvent(new Event('input',{bubbles:true}));document.querySelector('.add-app button').click()})()`);
  assert.equal(await evaluate(`document.querySelectorAll('.ignored-apps li').length`),5);
  await evaluate(`document.querySelector('.ignored-apps button').click()`);
  assert.equal(await evaluate(`document.querySelectorAll('.ignored-apps li').length`),4);
  checks.push('忽略程序增加及删除');
  await input(0,50); await save();
  await until(()=>evaluate(`!!document.querySelector('.settings-actions .delete-confirm')`),'确认条缺失');
  assert.equal(await evaluate('window.settingUpdates.length'),0);
  assert(await evaluate(`document.querySelector('.delete-confirm').textContent.includes('将立即删除 27 条')`));
  await input(0,40); assert.equal(await evaluate(`!!document.querySelector('.delete-confirm')`),false);
  await save(); await until(()=>evaluate(`!!document.querySelector('.delete-confirm')`),'确认条缺失');
  await evaluate(`document.querySelectorAll('.delete-confirm button')[1].click()`);
  assert.equal(await evaluate('window.settingUpdates.length'),0);
  checks.push('收紧前只预估不写入，编辑及取消使确认失效');
  await save(); await until(()=>evaluate(`!!document.querySelector('.delete-confirm')`),'确认条缺失');
  await evaluate(`document.querySelector('.settings-view').scrollTop=document.querySelector('.settings-view').scrollHeight`);
  await pause(80);
  const image=await send('Page.captureScreenshot',{format:'jpeg',quality:65});
  await writeFile(path.join(out,'settings-confirmation.jpg'),Buffer.from(image.data,'base64'));
  await evaluate(`document.querySelector('.delete-confirm button').click()`);
  await until(()=>evaluate(`document.querySelector('.settings-actions [role=status]')?.textContent==='设置已保存'`),'保存未完成');
  assert.equal(await evaluate('window.settings.max_items'),40);
  checks.push('确认后提交且显示保存结果，确认条已离屏渲染');
  await input(0,1000); await save();
  await until(()=>evaluate('window.settings.max_items===1000'),'放宽未保存');
  const previews=await evaluate('window.cleanupPreviews.length');
  await input(2,0); await save(); await until(()=>evaluate('window.settings.max_days===0'),'不限未保存');
  assert.equal(await evaluate('window.cleanupPreviews.length'),previews);
  await input(2,30); await evaluate('window.failPreview=true'); await save();
  await until(()=>evaluate(`document.querySelector('.settings-view .error')?.textContent.includes('合成预估失败')`),'预估错误缺失');
  assert.equal(await evaluate('window.settings.max_days'),0);
  await evaluate('window.failPreview=false'); await input(2,0);
  checks.push('放宽及改为不限不确认，预估失败不写入');
  await evaluate(`(()=>{const n=[...document.querySelectorAll('.settings-view select')].find(n=>n.querySelector('option[value=alt_v]'));n.value='ctrl_alt_v';n.dispatchEvent(new Event('change',{bubbles:true}));window.failSettings=true})()`);
  await save(); await until(()=>evaluate(`document.querySelector('.settings-view .error')?.textContent.includes('快捷键已被其他程序占用')`),'占用错误缺失');
  assert.equal(await evaluate('window.settings.main_hotkey'),'alt_v');
  await evaluate('window.failSettings=false;window.holdSettings=true'); await save();
  await until(()=>evaluate(`typeof window.releaseSettings==='function'`),'等待设置未开始');
  await press('Escape'); assert(await evaluate(`!!document.querySelector('.settings-view')`));
  await evaluate('window.releaseSettings()'); await until(()=>evaluate('window.settings.main_hotkey==="ctrl_alt_v"'),'切换未保存');
  await until(()=>evaluate(`document.querySelector('header p').textContent.includes('Ctrl + Alt + V')`),'提示未更新');
  await press('Escape'); await open();
  assert.equal(await evaluate(`[...document.querySelectorAll('.settings-view select')].find(n=>n.querySelector('option[value=alt_v]')).value`),'ctrl_alt_v');
  checks.push('占用错误保留设置；保存期间防止重复操作；保存后提示同步及重新加载');
  await send('Emulation.setDeviceMetricsOverride',{width:680,height:620,deviceScaleFactor:1,mobile:false});
  assert(await evaluate('document.documentElement.scrollWidth<=680'));
  checks.push('窄窗口设置页无横向溢出');
  const canonicalApps = [...await evaluate('window.settings.ignored_apps'), 'PathApp.EXE'];
  await evaluate(`window.holdSettings=false;window.normalizedIgnoredApps=${JSON.stringify(canonicalApps)}`);
  for (const name of [' C:\\Apps\\PathApp.EXE ', 'D:/Apps/pathapp.exe']) {
    await evaluate(`(()=>{const n=document.querySelector('.add-app input');n.value=${JSON.stringify(name)};n.dispatchEvent(new Event('input',{bubbles:true}));document.querySelector('.add-app button').click()})()`);
  }
  assert.equal(await evaluate(`document.querySelectorAll('.ignored-apps li').length`), canonicalApps.length + 1);
  await save();
  await until(()=>evaluate(`document.querySelector('.settings-actions [role=status]')?.textContent.includes('忽略程序已规范化')`),'规范化提示缺失');
  assert.deepEqual(await evaluate(`Array.from(document.querySelectorAll('.ignored-apps li>span'),n=>n.textContent)`), canonicalApps);
  assert.equal(await evaluate('window.settingUpdates.at(-1).ignored_apps.at(-1)'), 'D:/Apps/pathapp.exe');
  checks.push('接受 exe 路径，保存后回显后端规范化列表并提示转换');
  await send('Page.navigate',{url:origin+'/?panel=quick'}); await ready();
  assert.equal(await evaluate(`document.querySelector('.settings-toggle')`),null);
  assert(await evaluate(`document.querySelector('header p').textContent.includes('Ctrl + Shift')`));
  checks.push('快速面板无设置入口，使用后端快捷键提示');
  await evaluate(`window.emitEvent('settings-changed',{...window.settings,quick_hotkey:'alt_shift_v'})`);
  await until(()=>evaluate(`document.querySelector('header p').textContent.includes('保持 Alt + Shift')`),'快速面板提示未同步');
  checks.push('后端设置变更事件即时更新快速面板提示');
  await send('Page.navigate',{url:origin});await ready();
  await evaluate(`window.emitEvent('startup-error','快捷键已被其他程序占用，请在设置中更换快捷键');window.emitEvent('operation-error','')`);
  assert(await evaluate(`document.querySelector('[role=alert]').textContent.includes('快捷键已被其他程序占用')`));
  await evaluate(`window.emitEvent('startup-error','')`);
  assert(!await evaluate(`document.body.textContent.includes('快捷键已被其他程序占用')`));
  checks.push('启动热键错误独立保留，普通提示清空不覆盖，修复事件可清除');

  await writeFile(path.join(out,'result.json'),JSON.stringify({passed:true,checks,scope:'离屏 Edge + 合成 IPC；原生快捷键与输入法仍需人工验收'},null,2));
  console.log(JSON.stringify({passed:true,checks},null,2));
  await Promise.race([send('Browser.close').catch(()=>{}), pause(1000)]);
} finally { ws?.close();server.close();if(browser.exitCode===null)browser.kill(); }

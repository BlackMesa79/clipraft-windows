// 离屏渲染 + 合成 IPC 数据；不操作用户桌面，不代表原生粘贴验收。
import { createServer } from 'node:http';
import { spawn } from 'node:child_process';
import { readFile, writeFile, mkdir } from 'node:fs/promises';
import path from 'node:path';
import assert from 'node:assert/strict';
import { settingsMock } from './settings-ui-mock.mjs';
const root = process.cwd();
const out = path.join(root, '.test-data/m1-t6-ui');
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
    window.groups = [{id:1,name:'工作',sort:0},{id:2,name:'私人',sort:1}];window.mutations=[];
    window.rows = Array.from({length:250},(_,i)=>({pinned:false,favorite:i<220,group_id:i<120?1:null,id:'item-'+i,kind:'text',summary:(i<220?'你好 Hello nihao nh ':'Other ') + i,formats:['CF_UNICODETEXT'],files:[],thumbnail:null,placeholder:null}));
    window.rows[0].source_exe='notepad.exe';
    window.calls = []; window.deletes = []; window.pastes = []; window.hidden = 0; window.ready = false; window.failNext = false; window.holdNext = false;
    window.changed = () => callbacks.get(events.get('history-changed'))?.({event:'history-changed',id:1,payload:null});
    window.emitEvent=(event,payload)=>callbacks.get(events.get(event))?.({event,id:1,payload});
    window.__TAURI_EVENT_PLUGIN_INTERNALS__ = {unregisterListener(){}};
    window.__TAURI_INTERNALS__ = {
      transformCallback(fn){const id=++callbackId; callbacks.set(id,fn); return id;},
      convertFileSrc: hash => '/mock/'+hash,
      async invoke(cmd,args={}) {
        if(cmd==='plugin:event|listen'){events.set(args.event,args.handler);return 1;}
        if(cmd==='list_groups')return window.groups.map(g=>({...g,count:window.rows.filter(i=>i.group_id===g.id).length}));
        if(['set_pinned','set_favorite','move_to_group','create_group','rename_group','delete_group'].includes(cmd)) {
          window.mutations.push({cmd,args});
          if(window.failMutation){window.failMutation=false;throw Error('合成修改失败');}
          if(window.holdMutation){window.holdMutation=false;await new Promise(resolve=>window.releaseMutation=resolve);}
          let result;
          if(cmd==='create_group'||cmd==='rename_group'){
            const name=args.name.trim();
            if(!name||[...name].length>40)throw Error('组名去掉首尾空白后必须为 1 到 40 个字符');
            if(window.groups.some(g=>g.id!==args.id&&g.name.toLowerCase()===name.toLowerCase()))throw Error('已存在同名分组');
            if(cmd==='create_group'){result={id:Math.max(0,...window.groups.map(g=>g.id))+1,name,sort:window.groups.length};window.groups.push(result);}
            else window.groups.find(g=>g.id===args.id).name=name;
          } else if(cmd==='delete_group') {
            window.rows.forEach(i=>{if(i.group_id===args.id)i.group_id=null});window.groups=window.groups.filter(g=>g.id!==args.id);
          } else {
            let count=0;const field=cmd==='set_pinned'?'pinned':cmd==='set_favorite'?'favorite':'group_id';const value=cmd==='move_to_group'?args.groupId:args.value;
            window.rows.forEach(i=>{if(args.ids.includes(i.id)&&i[field]!==value){i[field]=value;count++}});result=count;
          }
          window.changed();return result;
        }
        if(cmd==='list_items'){
          window.calls.push(args);
          if(window.failNext){window.failNext=false;throw Error('合成分页失败');}
          const terms=(args.filter?.query??'').trim().toLowerCase().split(/\\s+/).filter(Boolean);
          const filtered=window.rows.filter(item=>(!args.filter?.favorite_only || item.favorite) && (args.filter?.group_id===undefined || (args.filter.group_id===-1 ? item.group_id===null : item.group_id===args.filter.group_id)) && terms.every(term=>item.summary.toLowerCase().includes(term)) && (!args.filter?.kind || (args.filter.kind==='text' ? ['text','rich'].includes(item.kind) : args.filter.kind==='link' ? item.summary.startsWith('https://') : item.kind===args.filter.kind)));
          filtered.sort((a,b)=>Number(b.pinned)-Number(a.pinned));
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
  const settled=()=>until(()=>evaluate("document.querySelector('.list')?.getAttribute('aria-busy')==='false' || (!!document.querySelector('.empty') && !document.querySelector('.empty').textContent.includes('正在搜索'))"),'刷新未完成');
  const click=(i,extras={})=>evaluate(`document.querySelectorAll('.item')[${i}].dispatchEvent(new MouseEvent('click',{bubbles:true,...${JSON.stringify(extras)}}))`);
  const fresh=async()=>{await send('Page.navigate',{url:origin});await ready();await count(100);};
  await fresh();
  await evaluate(`window.rows[0].source_exe='notepad.exe';window.rows[0].source_title='合成标题';window.rows[1].source_exe=null;window.rows[2].source_exe='browser.exe';window.changed()`);
  await settled();
  await until(()=>evaluate("document.querySelector('.item .source-name')?.textContent==='来源：notepad.exe'"),'列表来源未更新');
  assert.equal(await evaluate("document.querySelector('.detail .source-name').textContent"),'来源：notepad.exe');
  checks.push('主窗口列表与详情统一显示完整 exe 文件名');
  assert.equal(await evaluate("document.querySelectorAll('.item')[1].querySelector('.source-name')"),null);
  await click(1); await until(()=>evaluate("document.querySelector('.detail .source-name')===null"),'无来源详情仍有标记');
  checks.push('NULL 来源在列表与详情均不显示');
  await click(2);await until(()=>evaluate("document.querySelector('.detail .source-name')?.textContent==='来源：browser.exe'"),'详情来源未随选择切换');
  checks.push('切换选中记录后详情来源同步');
  await click(3);assert.equal(await evaluate("document.querySelector('.detail .source-name')"),null);
  checks.push('兼容没有来源字段的旧记录');
  await evaluate(`window.rows[0].source_exe='x'.repeat(240)+'.exe';window.changed()`); await settled();await click(0);
  await send('Emulation.setDeviceMetricsOverride',{width:680,height:620,deviceScaleFactor:1,mobile:false});
  await pause(100);
  assert(await evaluate("document.documentElement.scrollWidth<=680 && [...document.querySelectorAll('.workspace,.list,.detail,.item')].every(el=>el.scrollWidth<=el.clientWidth+1)"));
  assert(await evaluate("[...document.querySelectorAll('.source-name')].every(el=>el.getBoundingClientRect().right<=680)"));
  checks.push('680px 窄窗口长来源名省略，页面/列表/详情无横向溢出');
  const image=await send('Page.captureScreenshot',{format:'jpeg',quality:65});
  await writeFile(path.join(out,'narrow-preview.jpg'),Buffer.from(image.data,'base64'));
  await evaluate(`window.rows[0].source_exe='合成程序.EXE';window.changed()`);await settled();
  assert.equal(await evaluate("document.querySelector('.item .source-name').textContent"),'来源：合成程序.EXE');
  assert.equal(await evaluate("document.querySelector('.detail .source-name').textContent"),'来源：合成程序.EXE');
  checks.push('来源名保留 Unicode 与原始大小写');
  const hostileSource='<img src=x onerror=alert(1)>.exe';
  await evaluate(`window.rows[0].source_exe=${JSON.stringify(hostileSource)};window.changed()`);await settled();
  assert.equal(await evaluate("document.querySelector('.item .source-name').textContent"),`来源：${hostileSource}`);
  assert.equal(await evaluate("document.querySelector('.item .source-name img')"),null);
  checks.push('来源名中的 HTML 特殊字符按文本转义');
  await send('Page.navigate',{url:origin+'/?panel=quick'});await ready();await count(200);
  assert.equal(await evaluate("document.querySelector('.source-name')"),null);
  checks.push('快速面板保持现有紧凑布局');
  await writeFile(path.join(out,'results.json'),JSON.stringify({passed:checks.length,checks},null,2));console.log(JSON.stringify({passed:checks.length,checks},null,2));
} finally {
  ws?.close();browser.kill();await new Promise(resolve=>server.close(resolve));
}

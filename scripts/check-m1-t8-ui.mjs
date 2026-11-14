// 离屏渲染 + 合成 IPC 数据；不操作用户桌面，不代表原生粘贴验收。
import { createServer } from 'node:http';
import { spawn } from 'node:child_process';
import { readFile, writeFile, mkdir } from 'node:fs/promises';
import path from 'node:path';
import assert from 'node:assert/strict';
import { settingsMock } from './settings-ui-mock.mjs';
const root = process.cwd();
const out = path.join(root, '.test-data/m1-t8-ui');
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
    const nativeInterval=window.setInterval;window.setInterval=(fn,ms,...args)=>{if(ms===60000)window.minuteTick=fn;if(ms===30000)window.fileTick=fn;return nativeInterval(fn,ms,...args);};
    window.groups = [{id:1,name:'工作',sort:0},{id:2,name:'私人',sort:1}];window.mutations=[];
    window.rows = Array.from({length:250},(_,i)=>({pinned:false,favorite:i<220,group_id:i<120?1:null,id:'item-'+i,kind:'text',summary:(i<220?'你好 Hello nihao nh ':'Other ') + i,formats:['CF_UNICODETEXT'],files:[],thumbnail:null,placeholder:null}));
    window.syntheticToken='sk-SyntheticOnly0123456789abcdef'; window.masked='普通前文 sk-S••••••••ef 普通后文'; window.rows[0].sensitive=true;window.rows[0].expires_at=Date.now()+9*60000;window.rows[0].summary=window.masked; window.reveals=[];
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
        if(['set_pinned','set_favorite','move_to_group','set_sensitive','create_group','rename_group','delete_group'].includes(cmd)) {
          window.mutations.push({cmd,args});
          if(window.failMutation){window.failMutation=false;throw Error('合成修改失败');}
          if(window.holdMutation){window.holdMutation=false;await new Promise(resolve=>window.releaseMutation=resolve);}
          let result;
          if(cmd==='set_sensitive'){window.rows.forEach(i=>{if(args.ids.includes(i.id)){i.sensitive=args.value;i.expires_at=args.value?Date.now()+600000:null;i.summary=args.value?window.masked:'普通前文 '+window.syntheticToken+' 普通后文';}});window.changed();return args.ids.length;}
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
        if(cmd==='panel_state')return {items:window.rows.slice(0,200),selected:1,visible:true,error:null,serial:0,session:1,position:1,total:window.rows.length,loading:false};
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
        if(cmd==='reveal_item'){window.reveals.push(args.id);const result={text:'普通前文 '+window.syntheticToken+' 普通后文',truncated:false,files:[]};if(window.holdReveal){window.holdReveal=false;return new Promise(resolve=>window.releaseReveal=()=>resolve(result));}return result;}
        if(cmd==='item_detail') {
          const item=window.rows.find(i=>i.id===args.id);const result={text:item?.sensitive?window.masked:'详情 '+args.id,truncated:false,files:[]};
          if(window.holdDetail){window.holdDetail=false;return new Promise(resolve=>window.releaseDetail=()=>resolve(result));}
          return result;
        }
        return 1;
      }
    };
  })();`;
  await send('Page.enable');
  await send('Runtime.enable');
  await send('Page.addScriptToEvaluateOnNewDocument', { source: mock + settingsMock });
  await send('Emulation.setDeviceMetricsOverride', {
    width: 1000,
    height: 700,
    deviceScaleFactor: 1,
    mobile: false,
  });
  const checks = [];
  const ready = () => until(() => evaluate('window.ready===true'), '前端未就绪');
  const count = (n) =>
    until(() => evaluate(`document.querySelectorAll('.item').length===${n}`), '条目数错误 ' + n);
  const settled = () =>
    until(
      () =>
        evaluate(
          "document.querySelector('.list')?.getAttribute('aria-busy')==='false' || (!!document.querySelector('.empty') && !document.querySelector('.empty').textContent.includes('正在搜索'))",
        ),
      '刷新未完成',
    );
  const fresh = async () => {
    await send('Page.navigate', { url: origin });
    await ready();
    await count(100);
  };
  const button = (label, selector = 'button') =>
    evaluate(
      `(()=>{const el=Array.from(document.querySelectorAll(${JSON.stringify(selector)})).find(b=>b.textContent.trim()===${JSON.stringify(label)});if(!el)throw Error('按钮未找到 '+${JSON.stringify(label)});el.click();})()`,
    );
  await fresh();
  await evaluate(`window.rows=[
    {id:'missing',kind:'files',summary:'文件',files:['C:/合成目录/已删除.txt'],file_status:'missing'},
    {id:'unknown',kind:'files',summary:'文件',files:['C:/合成目录/权限.txt'],file_status:'unknown'},
    {id:'available',kind:'files',summary:'文件',files:['C:/合成目录/存在.txt'],file_status:'available'},
    {id:'pending',kind:'files',summary:'文件',files:['C:/合成目录/等待.txt'],file_status:null},
    {id:'secret',kind:'files',summary:'已打码',files:[],file_status:'missing',sensitive:true},
    {id:'text',kind:'text',summary:'普通文本',files:[],file_status:'missing'}
  ].map(i=>({pinned:false,favorite:false,group_id:null,formats:['CF_HDROP'],thumbnail:null,placeholder:null,...i}));window.changed()`);
  await count(6);
  await settled();
  const status = (i) =>
    evaluate(
      `document.querySelectorAll('.item')[${i}].querySelector('.file-status')?.textContent??null`,
    );
  assert.equal(await status(0), '源文件已不存在');
  checks.push('缺失文件显示明确标记');
  assert.equal(await status(1), '无法检查源文件');
  checks.push('未知状态独立显示，不误报缺失');
  assert.equal(await status(2), null);
  assert.equal(await status(3), null);
  checks.push('可用及尚未检查状态不显示失效标记');
  assert.equal(await status(4), null);
  assert.equal(await status(5), null);
  checks.push('敏感文件与普通文本不显示文件失效标记');
  assert.match(
    await evaluate("document.querySelector('.item .summary').textContent"),
    /已删除.txt/,
  );
  checks.push('中文文件名正确渲染');
  await evaluate(
    "window.rows[0].file_status='available';window.emitEvent('file-status-changed',null)",
  );
  await settled();
  await until(
    () => evaluate("!document.querySelectorAll('.item')[0].querySelector('.file-status')"),
    '缓存更新未移除标记',
  );
  checks.push('后台刷新可移除已恢复文件的缺失标记');
  const beforeTick = await evaluate('window.calls.length');
  await evaluate('window.fileTick()');
  await until(() => evaluate(`window.calls.length>${beforeTick}`), '30秒文件刷新未发起');
  await settled();
  checks.push('30秒定时回调刷新文件列表');
  await evaluate("window.emitEvent('open-settings',null)");
  await until(
    () => evaluate("!!document.querySelector('.settings-view form')"),
    '托盘设置事件未打开设置',
  );
  checks.push('托盘 open-settings 事件打开设置');
  await evaluate("window.emitEvent('open-settings',null)");
  await pause(30);
  assert(await evaluate("!!document.querySelector('.settings-view form')"));
  checks.push('重复托盘设置事件保持打开');
  const checkbox = (label) =>
    `[...document.querySelectorAll('.settings-view label')].find(l=>l.textContent.includes(${JSON.stringify(label)})).querySelector('input')`;
  assert.equal(await evaluate(checkbox('开机自启') + '.checked'), false);
  assert.equal(await evaluate(checkbox('启动时暂停监听') + '.checked'), false);
  await evaluate(checkbox('开机自启') + '.click()');
  await evaluate(checkbox('启动时暂停监听') + '.click()');
  await evaluate('window.failSettings=true');
  await button('保存');
  await until(
    () => evaluate("!!document.querySelector('.settings-view .error')"),
    '保存失败没有错误提示',
  );
  assert.equal(await evaluate('window.settings.autostart'), false);
  assert.equal(await evaluate('window.settings.pause_on_start'), false);
  checks.push('设置失败显示错误且 mock 持久值保持原值（不代表真实注册表回滚）');
  await evaluate('window.failSettings=false');
  await button('保存');
  await until(
    () => evaluate('window.settings.autostart && window.settings.pause_on_start'),
    '设置重试未保存',
  );
  checks.push('重试提交两个通用布尔设置');
  await button('返回列表');
  await evaluate("window.emitEvent('open-settings',null)");
  await until(() => evaluate("!!document.querySelector('.settings-view form')"), '设置未重开');
  assert.equal(await evaluate(checkbox('开机自启') + '.checked'), true);
  assert.equal(await evaluate(checkbox('启动时暂停监听') + '.checked'), true);
  checks.push('重新打开设置回显保存结果');
  await button('返回列表');
  await evaluate("window.rows[0].file_status='missing';window.changed()");
  await settled();
  await send('Emulation.setDeviceMetricsOverride', {
    width: 680,
    height: 620,
    deviceScaleFactor: 1,
    mobile: false,
  });
  await pause(80);
  assert(
    await evaluate(
      "document.documentElement.scrollWidth<=680 && [...document.querySelectorAll('.file-status')].every(el=>{const r=el.getBoundingClientRect(),p=el.closest('.item').getBoundingClientRect();return r.top>=p.top && r.bottom<=p.bottom && r.width>0})",
    ),
  );
  const preview = await send('Page.captureScreenshot', { format: 'jpeg', quality: 65 });
  await writeFile(path.join(out, 'narrow-preview.jpg'), Buffer.from(preview.data, 'base64'));
  checks.push('680px 真实离屏渲染：文件标记在行内且无横向溢出');
  await send('Page.navigate', { url: origin + '/?panel=quick' });
  await ready();
  await count(200);
  await evaluate("window.emitEvent('open-settings',null)");
  await pause(30);
  assert.equal(await evaluate("document.querySelector('.settings-view')"), null);
  checks.push('快速面板忽略托盘设置事件');
  await writeFile(
    path.join(out, 'results.json'),
    JSON.stringify({ passed: checks.length, checks }, null, 2),
  );
  console.log(JSON.stringify({ passed: checks.length, checks }, null, 2));
} finally {
  ws?.close();
  browser.kill();
  await new Promise((resolve) => server.close(resolve));
}

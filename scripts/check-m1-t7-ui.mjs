// 离屏渲染 + 合成 IPC 数据；不操作用户桌面，不代表原生粘贴验收。
import { createServer } from 'node:http';
import { spawn } from 'node:child_process';
import { readFile, writeFile, mkdir } from 'node:fs/promises';
import path from 'node:path';
import assert from 'node:assert/strict';
import { settingsMock } from './settings-ui-mock.mjs';
const root = process.cwd();
const out = path.join(root, '.test-data/m1-t7-ui');
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
    window.clockNow=1800000000000;Date.now=()=>window.clockNow;
    const nativeInterval=window.setInterval;window.setInterval=(fn,ms,...args)=>{if(ms===60000)window.minuteTick=fn;return nativeInterval(fn,ms,...args);};
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
  const click = (i, extras = {}) =>
    evaluate(
      `document.querySelectorAll('.item')[${i}].dispatchEvent(new MouseEvent('click',{bubbles:true,...${JSON.stringify(extras)}}))`,
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
  const context = async (i) => {
    await evaluate(
      `document.querySelectorAll('.item')[${i}].dispatchEvent(new MouseEvent('contextmenu',{bubbles:true,clientX:980,clientY:680}))`,
    );
    await pause(30);
  };
  const mutated = async (n) => {
    await until(() => evaluate(`window.mutations.length===${n}`), '修改未发出 ' + n);
    await settled();
    await pause(40);
  };
  await fresh();
  const noPlain = () => evaluate('!document.body.textContent.includes(window.syntheticToken)');
  const detailReady = () =>
    until(() => evaluate("!!document.querySelector('.text-detail')"), '详情未就绪');
  await detailReady();
  assert(await noPlain());
  assert.match(
    await evaluate("document.querySelector('.sensitive-badge').textContent"),
    /敏感.*9 分钟后删除/,
  );
  assert.deepEqual(await evaluate('window.reveals'), []);
  checks.push('列表及详情默认脱敏，显示敏感与 9 分钟剩余时间，不主动请求原文');
  await evaluate(
    'window.originalNow=Date.now;Date.now=()=>window.originalNow()+60000;window.minuteTick()',
  );
  await until(
    () =>
      evaluate("document.querySelector('.sensitive-badge').textContent.includes('8 分钟后删除')"),
    '分钟刷新未生效',
  );
  await evaluate('Date.now=window.originalNow;window.minuteTick()');
  checks.push('分钟定时回调更新剩余时间');
  await button('显示原文');
  await until(
    () =>
      evaluate(
        "document.querySelector('.text-detail')?.textContent.includes(window.syntheticToken)",
      ),
    '原文未显示',
  );
  assert.deepEqual(await evaluate('window.reveals'), ['item-0']);
  await click(1);
  await detailReady();
  assert(await noPlain());
  await click(0);
  await detailReady();
  assert(await noPlain());
  checks.push('点击显示才请求原文，切换记录后恢复脱敏');
  await button('显示原文');
  await until(
    () =>
      evaluate(
        "!!document.querySelector('.sensitive-actions button') && document.querySelector('.sensitive-actions button').textContent==='隐藏原文'",
      ),
    '显示未完成',
  );
  await evaluate("window.emitEvent('main-hidden',null)");
  await pause(30);
  assert(await noPlain());
  await evaluate("window.emitEvent('main-shown',null)");
  await detailReady();
  assert(await noPlain());
  checks.push('关闭再显示窗口后原文不保留');
  await evaluate('window.holdReveal=true');
  await button('显示原文');
  await until(() => evaluate("typeof window.releaseReveal==='function'"), '原文请求未挂起');
  await click(1);
  await detailReady();
  await evaluate('window.releaseReveal()');
  await pause(50);
  assert(await noPlain());
  checks.push('切换记录后丢弃迟到的原文响应');
  await click(0);
  await detailReady();
  await button('显示原文');
  await until(
    () => evaluate('document.body.textContent.includes(window.syntheticToken)'),
    '显示未完成',
  );
  await send('Page.reload');
  await ready();
  await count(100);
  await detailReady();
  assert(await noPlain());
  checks.push('页面重新加载恢复脱敏');
  // 仅向离屏页面派发 DOM 事件，不改变用户的真实窗口焦点。
  const showPlain = async () => {
    await button('显示原文');
    await until(
      () => evaluate('document.body.textContent.includes(window.syntheticToken)'),
      '原文未显示',
    );
  };
  await showPlain();
  await button('刷新状态');
  await detailReady();
  assert(await noPlain());
  checks.push('详情刷新状态恢复脱敏');
  await showPlain();
  await evaluate('window.changed()');
  await settled();
  await detailReady();
  assert(await noPlain());
  checks.push('同一选中项收到历史刷新后恢复脱敏');
  await showPlain();
  await evaluate("window.dispatchEvent(new Event('blur'))");
  await pause(30);
  assert(await noPlain());
  await evaluate("window.dispatchEvent(new Event('focus'))");
  await detailReady();
  assert(await noPlain());
  checks.push('离屏失焦及重新聚焦恢复脱敏');
  await showPlain();
  await evaluate(
    "Object.defineProperty(document,'hidden',{configurable:true,value:true});document.dispatchEvent(new Event('visibilitychange'))",
  );
  await pause(30);
  assert(await noPlain());
  await evaluate(
    "Object.defineProperty(document,'hidden',{configurable:true,value:false});document.dispatchEvent(new Event('visibilitychange'))",
  );
  await detailReady();
  assert(await noPlain());
  checks.push('页面隐藏及恢复可见时恢复脱敏');
  await evaluate('window.holdReveal=true');
  await button('显示原文');
  await until(() => evaluate("typeof window.releaseReveal==='function'"), '原文请求未挂起');
  await evaluate("window.emitEvent('main-hidden',null);window.releaseReveal()");
  await pause(50);
  assert(await noPlain());
  await evaluate("window.emitEvent('main-shown',null)");
  await detailReady();
  assert(await noPlain());
  checks.push('隐藏窗口后丢弃迟到的原文响应');
  await showPlain();
  await evaluate("document.querySelector('button[aria-label=设置]').click()");
  await until(() => evaluate("!!document.querySelector('.settings-view form')"), '设置未加载');
  assert(await noPlain());
  await button('返回列表');
  await until(() => evaluate("!document.querySelector('.settings-view')"), '设置未关闭');
  assert(await noPlain());
  assert.equal(
    await evaluate("document.querySelector('.sensitive-actions button').textContent"),
    '显示原文',
  );
  await button('刷新状态');
  await detailReady();
  assert(await noPlain());
  checks.push('打开设置清除原文，返回后刷新提供脱敏详情');
  await context(0);
  await button('取消敏感标记');
  await mutated(1);
  assert.equal(await evaluate('window.mutations[0].cmd'), 'set_sensitive');
  assert.equal(await evaluate('window.mutations[0].args.value'), false);
  assert.equal(
    await evaluate("document.querySelectorAll('.item')[0].querySelector('.sensitive-badge')"),
    null,
  );
  await context(0);
  await button('标记为敏感');
  await mutated(2);
  assert(await noPlain());
  checks.push('右键标记与取消标记使用 set_sensitive，列表状态刷新');
  await click(0);
  await click(1, { ctrlKey: true });
  await context(0);
  await button('标记为敏感');
  await mutated(3);
  assert.deepEqual(await evaluate('window.mutations[2].args.ids'), ['item-0', 'item-1']);
  assert.equal(await evaluate('window.mutations[2].args.value'), true);
  await context(0);
  await button('取消敏感标记');
  await mutated(4);
  assert.equal(await evaluate('window.mutations[3].args.value'), false);
  checks.push('多选可批量标记及取消标记');
  await evaluate(
    'window.rows[0].sensitive=true;window.rows[0].summary=window.masked;window.rows[0].expires_at=null;window.rows[1].summary="普通记录";window.changed()',
  );
  await settled();
  await click(0);
  await detailReady();
  assert.equal(await evaluate("document.querySelector('.sensitive-badge').textContent"), '敏感');
  checks.push('无删除倒计时的记录仅显示敏感标记');
  await evaluate(
    'window.clockNow+=45000;window.rows[0].expires_at=Date.now()+600000;window.changed()',
  );
  await settled();
  assert.equal(
    await evaluate("document.querySelector('.sensitive-badge').textContent"),
    '敏感 · 10 分钟后删除',
  );
  checks.push('列表已存在45秒后新增10分钟倒计时，立即显示10而非11分钟');
  for (const [advance, expected] of [
    [59999, '10 分钟后删除'],
    [1, '9 分钟后删除'],
  ]) {
    await evaluate(`window.clockNow+=${advance};window.minuteTick()`);
    await pause(20);
    assert.equal(
      await evaluate("document.querySelector('.sensitive-badge').textContent"),
      '敏感 · ' + expected,
    );
  }
  checks.push('按剩余时长向上取整，整分钟边界不提前少算一分钟');
  await evaluate(
    'window.clockNow+=30000;window.rows[0].expires_at=Date.now()+600000;window.changed()',
  );
  await settled();
  assert.equal(
    await evaluate("document.querySelector('.sensitive-badge').textContent"),
    '敏感 · 10 分钟后删除',
  );
  checks.push('同一记录重新设置到期时间立即校准显示时钟');
  await evaluate('window.rows[0].expires_at=Date.now()+60000;window.changed()');
  await settled();
  for (const [advance, expected] of [
    [0, '1 分钟后删除'],
    [59999, '1 分钟后删除'],
    [1, '即将删除'],
  ]) {
    await evaluate(`window.clockNow+=${advance};window.minuteTick()`);
    await pause(20);
    assert.equal(
      await evaluate("document.querySelector('.sensitive-badge').textContent"),
      '敏感 · ' + expected,
    );
  }
  checks.push('1分钟、剩余1毫秒和到期瞬间显示准确，不显示0分钟');

  await evaluate('window.rows[0].expires_at=Date.now()-60000;window.changed()');
  await settled();
  // 通过受控时钟重新挂载，验证每分钟更新使用的同一计算路径。
  await send('Page.reload');
  await ready();
  await count(100);
  await evaluate('window.rows[0].expires_at=Date.now()-60000;window.changed()');
  await settled();
  assert.match(
    await evaluate("document.querySelector('.sensitive-badge').textContent"),
    /即将删除/,
  );
  checks.push('过期记录显示即将删除，等待后端删除事件');
  await send('Emulation.setDeviceMetricsOverride', {
    width: 680,
    height: 620,
    deviceScaleFactor: 1,
    mobile: false,
  });
  await pause(100);
  assert(
    await evaluate(
      "document.documentElement.scrollWidth<=680 && [...document.querySelectorAll('.workspace,.list,.detail,.item')].every(el=>el.scrollWidth<=el.clientWidth+1)",
    ),
  );
  const image = await send('Page.captureScreenshot', { format: 'jpeg', quality: 65 });
  await writeFile(path.join(out, 'narrow-preview.jpg'), Buffer.from(image.data, 'base64'));
  checks.push('680px 窄窗口无横向溢出，已保存离屏渲染预览');
  await evaluate("document.querySelector('button[aria-label=设置]').click()");
  await until(() => evaluate("!!document.querySelector('.settings-view form')"), '设置未加载');
  const entropy =
    "[...document.querySelectorAll('label')].find(l=>l.textContent.includes('高熵字符串识别')).querySelector('input')";
  assert.equal(await evaluate(entropy + '.checked'), false);
  await evaluate(entropy + '.click()');
  await button('保存');
  await until(() => evaluate('window.settingUpdates.length===1'), '设置未保存');
  assert.equal(await evaluate('window.settingUpdates[0].sensitive_high_entropy'), true);
  checks.push('高熵开关默认关闭且通过强类型设置载荷保存');
  await send('Page.navigate', { url: origin + '/?panel=quick' });
  await ready();
  await count(200);
  await evaluate(
    "window.clockNow+=45000;window.rows[0].expires_at=Date.now()+600000;window.emitEvent('panel-state',{items:window.rows.slice(0,200),selected:1,visible:true,error:null,serial:1,session:1,position:1,total:window.rows.length,loading:false})",
  );
  await pause(30);
  assert.equal(
    await evaluate("document.querySelector('.sensitive-badge').textContent"),
    '敏感 · 10 分钟后删除',
  );
  checks.push('快速面板迟到的到期状态也立即显示准确分钟数');
  assert(await noPlain());
  assert(await evaluate("!!document.querySelector('.sensitive-badge')"));
  assert.equal(await evaluate("document.querySelector('.sensitive-actions')"), null);
  assert.deepEqual(await evaluate('window.reveals'), []);
  checks.push('快速面板接收脱敏摘要并显示标记，没有显示原文入口');
  assert(
    await evaluate(
      "[...document.querySelectorAll('.item')].filter(el=>el.querySelector('.sensitive-badge')).every(el=>{const row=el.getBoundingClientRect(),content=el.querySelector('.content').getBoundingClientRect();return content.top>=row.top && content.bottom<=row.bottom;})",
    ),
  );
  const quickImage = await send('Page.captureScreenshot', { format: 'jpeg', quality: 65 });
  await writeFile(path.join(out, 'quick-preview.jpg'), Buffer.from(quickImage.data, 'base64'));
  checks.push('快速面板敏感标记在紧凑行内完整显示，无内容越界');
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

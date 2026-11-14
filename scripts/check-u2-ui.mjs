// 离屏渲染 + 合成 IPC 数据；不操作用户桌面，不代表原生粘贴验收。
import { createServer } from 'node:http';
import { spawn } from 'node:child_process';
import { readFile, writeFile, mkdir } from 'node:fs/promises';
import path from 'node:path';
import assert from 'node:assert/strict';
import { settingsMock } from './settings-ui-mock.mjs';
const root = process.cwd();
const out = path.join(root, '.tools/u2-ui');
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
    const id = ++seq;
    const timer=setTimeout(()=>{pending.delete(id);reject(Error(method+' 等待超过15秒'));},15_000);
    pending.set(id,{resolve:value=>{clearTimeout(timer);resolve(value);},reject:error=>{clearTimeout(timer);reject(error);}});
    ws.send(JSON.stringify({ id, method, params }));
  });
  const evaluate = async expression => {
    const result = await send('Runtime.evaluate', { expression, returnByValue: true, awaitPromise: true });
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
    window.rows=Array.from({length:60},(_,i)=>({id:'row-'+i,kind:i===1?'files':i===2?'image':'text',summary:i===3?'合成敏感••••':'合成记录 '+i+' 用于外观和布局验收',sensitive:i===3,favorite:i%2===0,group_id:i%3===0?7:null,formats:['CF_UNICODETEXT'],files:i===1?['C:/synthetic/report.txt']:[],file_status:i===1?'missing':undefined,source_exe:'synthetic.exe',thumbnail:null}));
    window.emitEvent=(event,payload)=>callbacks.get(events.get(event))?.({event,id:1,payload});
    window.__TAURI_EVENT_PLUGIN_INTERNALS__={unregisterListener(){}};
    window.__TAURI_INTERNALS__={
      metadata:{currentWindow:{label:'main'}},
      transformCallback(fn){callbacks.set(++id,fn);return id;},convertFileSrc:hash=>hash==='synthetic-thumb'?'data:image/svg+xml,'+encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" width="160" height="80"><rect width="160" height="80" fill="#277d77"/></svg>'):'/mock/'+hash,
      async invoke(cmd,args={}) {
        window.calls.push({cmd,args});
        if(cmd==='plugin:event|listen'){events.set(args.event,args.handler);return id;}
        if(cmd==='resize_quick'){if(window.failResize){window.failResize=false;window.resizeFailures=(window.resizeFailures??0)+1;throw Error('合成 resize 失败');}window.resizeSuccess=(window.resizeSuccess??0)+1;return;}
        if(cmd==='sync_compact_preview'){if(window.echoPreviewFocus&&args.id){window.previewFocusEchoes=(window.previewFocusEchoes??0)+1;if(window.previewFocusEchoes>10)throw Error('预览焦点反馈未收敛');window.dispatchEvent(new Event('focus'));}return;}
        if(cmd==='compact_preview_geometry')return null;
        if(cmd==='compact_preview_state'){if(location.search.includes('holdpreview'))return new Promise(r=>window.releasePreviewSnapshot=r);return null;}
        if(cmd==='panel_state'){if(location.search.includes('fail=panel'))throw Error('合成 panel_state 失败');return {items:window.rows,selected:1,visible:true,error:null};}
        if(cmd==='frontend_ready'){window.ready=true;return;}
        if(cmd==='set_hotkey_recording'){if(args.active&&window.holdRecording)await new Promise(resolve=>window.releaseRecording=resolve);return 1;}
        if(cmd==='list_items'){
          if(window.failList)throw Error('合成列表失败');
          const f=args.filter??{};
          return window.rows.filter(r=>(!f.kind||r.kind===f.kind)&&(!f.query||r.summary.includes(f.query))&&(!f.favorite_only||r.favorite)&&(f.group_id==null||(f.group_id===-1?r.group_id==null:r.group_id===f.group_id))).slice(args.offset,args.offset+args.limit);
        }
        if(cmd==='reveal_item'){if(window.holdReveal)return new Promise(r=>window.releaseReveal=()=>r({text:'SYNTHETIC_SECRET_RAW',files:[],truncated:false}));return {text:'SYNTHETIC_SECRET_RAW',files:[],truncated:false};}
        if(cmd==='list_groups')return [{id:7,name:'工作',count:35},{id:8,name:'长分组名称用于检查窄窗口截断',count:2}];
        if(cmd==='last_error')return null;
        if(cmd==='item_detail')return {text:'合成详情',files:[],truncated:false};
        return 1;
      }
    };
  })();`;
  await send('Page.enable');await send('Runtime.enable');
  await send('Page.addScriptToEvaluateOnNewDocument',{source:mock+settingsMock+`
    const invokeWithSettings=window.__TAURI_INTERNALS__.invoke;
    window.__TAURI_INTERNALS__.invoke=async(cmd,args)=>{
      if(cmd==='get_settings'&&location.search.includes('fail=settings'))throw Error('合成 settings 失败');
      return invokeWithSettings(cmd,args);
    };`});
  const checks=[], failures=[];
  const test=async(name,fn)=>{if(process.env.U2_TEST_FILTER&&!name.includes(process.env.U2_TEST_FILTER))return;try{await fn();checks.push(name);}catch(e){failures.push({name,error:e.stack??String(e)});}};
  const click=selector=>evaluate(`document.querySelector(${JSON.stringify(selector)}).click()`);
  const change=(selector,value)=>evaluate(`(()=>{const n=document.querySelector(${JSON.stringify(selector)});n.value=${JSON.stringify(value)};n.dispatchEvent(new Event('change',{bubbles:true}));})()`);
  const appearance=async(patch)=>{await evaluate(`window.settings={...window.settings,...${JSON.stringify(patch)}};window.emitEvent('settings-changed',window.settings)`);await pause(100);};
  const nav=async(query='')=>{await send('Page.navigate',{url:origin+'/'+query});await until(()=>evaluate('window.ready===true'),'未 ready');await pause(100);};
  const metrics=async(width=1000)=>send('Emulation.setDeviceMetricsOverride',{width,height:700,deviceScaleFactor:1,mobile:false});
  const capture=async(name)=>{const shot=await send('Page.captureScreenshot',{format:'jpeg',quality:65});const bytes=Buffer.from(shot.data,'base64');assert(bytes.length<=300*1024);await writeFile(path.join(out,name+'.jpg'),bytes);};
  const selected=()=>evaluate("document.querySelector('.item[aria-selected=true]')?.dataset.id");
  const anchor=()=>evaluate(`(()=>{const l=document.querySelector('.list'),b=l.getBoundingClientRect();const r=[...l.querySelectorAll('.item')].find(r=>r.getBoundingClientRect().bottom>b.top+1);return {id:r?.dataset.id,offset:r?.getBoundingClientRect().top-b.top,scroll:l.scrollTop,height:r?.getBoundingClientRect().height};})()`);
  await metrics();await nav();
  await test('AC1 默认浅色单列非紧凑与失焦关闭',async()=>{
    assert.equal(await evaluate('document.documentElement.dataset.theme'),'light');
    assert.equal(await evaluate("document.querySelector('main').classList.contains('compact')"),false);
    assert.equal(await evaluate('window.settings.hide_on_blur'),false);
  });
  await click('[data-id="row-12"]');
  for(const theme of ['light','dark'])for(const mode of ['list','cards','compact']){
    await metrics(mode==='compact'?390:1000);
    await appearance({theme,layout:mode==='cards'?'cards':'list',compact:mode==='compact'});
    await test('AC2 '+theme+'/'+mode+' 布局、尺寸、选择',async()=>{
      assert.equal(await selected(),'row-12');
      assert.equal(await evaluate('document.documentElement.dataset.theme'),theme);
      if(mode==='compact'){
        assert.equal(await evaluate("!!document.querySelector('.detail')"),false);
        assert.equal(await evaluate("getComputedStyle(document.querySelector('.group-sidebar')).display"),'none');
        assert.equal(await evaluate("document.querySelector('.item').getBoundingClientRect().height"),54);
        assert.equal(await evaluate("document.querySelector('.type-icon').getBoundingClientRect().width"),32);
        assert.equal(await evaluate("document.querySelector('select[aria-label=分组筛选]').options.length"),4);
      }else{
        assert(await evaluate("!!document.querySelector('.detail')"));
        assert.equal(await evaluate("getComputedStyle(document.querySelector('.list')).display"),mode==='cards'?'grid':'flex');
      }
      assert(await evaluate('document.documentElement.scrollWidth<=innerWidth'));
    });
    await capture(theme+'-'+mode);
  }
  await test('AC3 分组×类型交集且切换不清空',async()=>{
    await change('select[aria-label=分组筛选]','7');await pause(70);
    await evaluate("[...document.querySelectorAll('.kind-filters button')].find(b=>b.textContent==='文本').click()");await pause(70);
    assert.equal(await evaluate("document.querySelector('select[aria-label=分组筛选]').value"),'7');
    assert.equal(await evaluate("document.querySelector('.favorite-filter')"),null);
    assert.deepEqual(await evaluate("window.calls.filter(c=>c.cmd==='list_items').at(-1).args.filter"),{query:'',kind:'text',group_id:7});
    assert.equal(await evaluate("document.querySelectorAll('.item').length"),20);
    await appearance({theme:'light',layout:'cards',compact:false});
    assert.equal(await evaluate("window.calls.filter(c=>c.cmd==='list_items').at(-1).args.filter.group_id"),7);
    assert.equal(await evaluate("document.querySelector('.favorite-filter')"),null);
    await appearance({compact:true});await change('select[aria-label=分组筛选]','8');await pause(70);
    assert.equal(await evaluate("document.querySelectorAll('.item').length"),0);
    assert(await evaluate("document.body.textContent.includes('没有匹配的记录')"));
    await capture('empty');
  });
  await nav();await click('[data-id="row-12"]');
  await evaluate("document.querySelector('.list').scrollTop=700");await pause(80);
  for(const [name,patch] of [['主题',{theme:'dark'}],['卡片',{layout:'cards'}],['紧凑',{compact:true}],['退出紧凑',{compact:false}]]){
    await test('AC4 '+name+' 保留选择及滚动锚点',async()=>{const before=await anchor();await appearance(patch);const after=await anchor();assert.equal(await selected(),'row-12');const kept=await evaluate(`(()=>{const l=document.querySelector('.list'),r=document.querySelector('[data-id="${before.id}"]');return {offset:r.getBoundingClientRect().top-l.getBoundingClientRect().top,height:r.getBoundingClientRect().height};})()`);assert(Math.abs(kept.offset-Math.max(before.offset,2-kept.height))<=2,JSON.stringify({before,after,kept}));});
  }
  await test('AC5 页首紧凑切换保存且退出恢复 cards',async()=>{
    await appearance({layout:'cards',compact:false});await click('.compact-toggle');await pause(80);
    assert.equal(await evaluate('window.settingUpdates.at(-1).compact'),true);
    await click('.compact-toggle');await pause(80);
    assert.equal(await evaluate('window.settings.layout'),'cards');assert.equal(await evaluate('window.settings.compact'),false);
  });
  await test('AC5 保存失败保留外观并显示错误',async()=>{
    await evaluate('window.failSettings=true');await click('.compact-toggle');await pause(80);
    assert.equal(await evaluate('window.settings.compact'),false);assert(await evaluate("document.querySelector('[role=alert]')?.textContent.includes('快捷键')"));await evaluate('window.failSettings=false');
  });
  await test('AC6 设置页编辑未生效、保存后生效及失焦配置往返',async()=>{
    await click('.settings-toggle');await until(()=>evaluate("!!document.querySelector('.settings-view form')"),'设置未打开');
    await change('select[aria-label=主题]','light');await change('select[aria-label=常规布局]','list');
    await evaluate("[...document.querySelectorAll('.check')].find(l=>l.textContent.includes('失去焦点')).querySelector('input').click()");
    assert.equal(await evaluate('document.documentElement.dataset.theme'),'dark');
    await click('.settings-view button[type=submit]');await pause(100);
    assert.equal(await evaluate('document.documentElement.dataset.theme'),'light');assert.equal(await evaluate('window.settings.hide_on_blur'),true);
    await capture('settings');await click('.settings-toggle');
    await click('.settings-toggle');await until(()=>evaluate("!!document.querySelector('.settings-view form')"),'设置重开失败');
    assert.equal(await evaluate("[...document.querySelectorAll('.check')].find(l=>l.textContent.includes('失去焦点')).querySelector('input').checked"),true);
    await click('.settings-toggle');
  });
  await nav();
  const reveal=async()=>{await click('[data-id="row-3"]');await pause(50);await evaluate("[...document.querySelectorAll('.sensitive-actions button')].find(b=>b.textContent.includes('显示原文')).click()");await pause(50);};
  for(const [name,action] of [['主题',()=>appearance({theme:'dark'})],['紧凑',()=>appearance({compact:true})],['隐藏事件',()=>evaluate("window.emitEvent('main-hidden')")],['失焦',()=>evaluate("window.dispatchEvent(new Event('blur'))")]]){
    await test('AC7 敏感原文在'+name+'后清理',async()=>{await nav();await reveal();assert(await evaluate("document.body.textContent.includes('SYNTHETIC_SECRET_RAW')"));await action();await pause(60);assert.equal(await evaluate("document.body.textContent.includes('SYNTHETIC_SECRET_RAW')"),false);});
  }
  await test('AC7 隐藏后迟到 reveal 响应被丢弃',async()=>{await nav();await evaluate('window.holdReveal=true');await reveal();await evaluate("window.emitEvent('main-hidden');window.releaseReveal()");await pause(60);assert.equal(await evaluate("document.body.textContent.includes('SYNTHETIC_SECRET_RAW')"),false);});
  await test('AC8 窄屏长错误不产生页面横向溢出',async()=>{await metrics(390);await appearance({compact:true,theme:'dark'});await evaluate("window.emitEvent('operation-error','合成很长的错误信息'.repeat(40))");await pause(60);await capture('narrow-error');assert(await evaluate('document.documentElement.scrollWidth<=innerWidth'));});
  await test('AC9 quick 只应用主题、不请求主窗口尺寸或历史',async()=>{await metrics(460);await nav('?panel=quick');await appearance({theme:'dark',layout:'cards',compact:true});assert.equal(await evaluate('document.documentElement.dataset.theme'),'dark');assert.equal(await evaluate("window.calls.filter(c=>['sync_main_layout','list_items','item_detail'].includes(c.cmd)).length"),0);assert(await evaluate("!!document.querySelector('.quick')&&!document.querySelector('.compact-toggle')"));await capture('quick-dark');});
  await test('AC10 外观根容器无旧样式外边距和间距',async()=>{await metrics();await nav();assert.equal(await evaluate("getComputedStyle(document.querySelector('.main-window')).paddingTop"),'0px');assert.equal(await evaluate("getComputedStyle(document.querySelector('.main-window')).rowGap"),'0px');});
  await test('AC10 卡片图片降级信息和摘要均在自身边界内',async()=>{await appearance({layout:'cards'});await capture('cards-image-fallback');const box=await evaluate(`(()=>{const r=document.querySelector('[data-id="row-2"]'),s=r.querySelector('.content');return {row:r.getBoundingClientRect().toJSON(),summary:s.getBoundingClientRect().toJSON(),display:getComputedStyle(r).display,rows:getComputedStyle(r).gridTemplateRows,assets:[...document.querySelectorAll('link[rel=stylesheet]')].map(l=>l.href)};})()`);assert(box.summary.bottom<=box.row.bottom,JSON.stringify(box));});
  await test('AC4 设置页往返保留滚动锚点及选择',async()=>{await nav();await click('[data-id="row-12"]');await evaluate("document.querySelector('.list').scrollTop=700");await pause(60);const before=await anchor();await click('.settings-toggle');await until(()=>evaluate("!!document.querySelector('.settings-view form')"),'设置页未打开');await click('.settings-toggle');await pause(80);assert.equal(await selected(),'row-12');const after=await anchor();assert.equal(after.id,before.id,JSON.stringify({before,after}));});
  await test('AC10 390px 紧凑容器不继承旧响应式边距',async()=>{await metrics(390);await appearance({compact:true});assert.equal(await evaluate("getComputedStyle(document.querySelector('.main-window')).paddingTop"),'0px');});
  await test('AC4 从设置修改布局返回保持选择、筛选和滚动锚点',async()=>{await metrics();await nav();await evaluate("document.querySelector('.kind-filters button:nth-child(2)').click()");await pause(80);await click('[data-id="row-12"]');await evaluate("document.querySelector('.list').scrollTop=450");await pause(60);const before=await anchor();await click('.settings-toggle');await until(()=>evaluate("!!document.querySelector('.settings-view form')"),'设置未打开');await change('select[aria-label=常规布局]','cards');await click('.settings-view button[type=submit]');await pause(100);await click('.settings-toggle');await pause(100);assert.equal(await selected(),'row-12');assert.equal(await evaluate("document.querySelector('.kind-filters button:nth-child(2)').getAttribute('aria-pressed')"),'true');const after=await anchor();const restored=await evaluate(`(()=>{const l=document.querySelector('.list'),r=document.querySelector('[data-id="${before.id}"]');return {id:r?.dataset.id,offset:r?.getBoundingClientRect().top-l.getBoundingClientRect().top};})()`);assert.equal(restored.id,before.id);assert(Math.abs(restored.offset-before.offset)<=2,JSON.stringify({before,after,restored}));});
  await test('AC11 真正加载的图片缩略图保持 contain 及紧凑尺寸',async()=>{await metrics(390);await nav();await evaluate("window.rows[4]={...window.rows[4],kind:'image',thumbnail:'synthetic-thumb'};window.emitEvent('history-changed')");await pause(100);await appearance({compact:true});await until(()=>evaluate("document.querySelector('[data-id=\"row-4\"] img')?.naturalWidth===160"),'合成图片未解码');assert.equal(await evaluate("getComputedStyle(document.querySelector('[data-id=\"row-4\"] img')).objectFit"),'contain');assert.equal(await evaluate("document.querySelector('[data-id=\"row-4\"] .preview').getBoundingClientRect().height"),38);await capture('compact-image-loaded');});
  await test('AC8 原生尺寸 IPC 失败明确提示且设置仍持久化',async()=>{await metrics();await nav();await evaluate("const prev=window.__TAURI_INTERNALS__.invoke;window.__TAURI_INTERNALS__.invoke=(cmd,args)=>cmd==='sync_main_layout'?Promise.reject('合成原生调整失败'):prev(cmd,args)");await click('.compact-toggle');await pause(100);assert.equal(await evaluate('window.settings.compact'),true);assert(await evaluate("document.querySelector('[role=alert]')?.textContent.includes('窗口尺寸调整失败')"));});

  await test('R01 自绘窗口操作 IPC 与拖动区',async()=>{
    await metrics();await nav();
    assert(await evaluate("document.querySelector('header').hasAttribute('data-tauri-drag-region')"));
    assert.equal(await evaluate("document.querySelectorAll('.window-controls button[data-tauri-drag-region]').length"),0);
    for(const label of ['最小化','最大化或还原','隐藏到托盘'])await click(`button[aria-label="${label}"]`);
    assert.deepEqual(await evaluate("window.calls.filter(c=>c.cmd.startsWith('plugin:window|')).map(c=>c.cmd)"),['plugin:window|minimize','plugin:window|toggle_maximize','plugin:window|close']);
  });
  await test('R02 收藏入口、标记、Ctrl+D 均移除且旧记录可见',async()=>{
    await nav();
    assert.equal(await evaluate("document.body.textContent.includes('收藏')"),false);
    assert.equal(await evaluate("document.querySelectorAll('.item').length"),60);
    await evaluate("window.dispatchEvent(new KeyboardEvent('keydown',{key:'d',ctrlKey:true,bubbles:true}))");
    assert.equal(await evaluate("window.calls.filter(c=>c.cmd==='set_favorite').length"),0);
    await evaluate("document.querySelector('.item').dispatchEvent(new MouseEvent('contextmenu',{bubbles:true,clientX:200,clientY:160}))");
    assert.equal(await evaluate("document.querySelector('.context-menu').textContent.includes('收藏')"),false);
  });
  for(const theme of ['light','dark']){
    await test('R04 '+theme+' 移出分组红底、说明及键盘可达',async()=>{
      await metrics();await nav();await appearance({theme});
      await evaluate("document.querySelector('.item').dispatchEvent(new MouseEvent('contextmenu',{bubbles:true,clientX:200,clientY:160}))");
      await click('[aria-haspopup=menu]');
      const style=await evaluate("(()=>{const n=document.querySelector('.remove-group'),s=getComputedStyle(n);return {title:n.title,background:s.backgroundColor,color:s.color,separator:!!n.previousElementSibling.matches('[role=separator]'),focused:document.activeElement.closest('.group-submenu')!==null};})()");
      assert.equal(style.title,'保留记录，仅移出当前分组');assert(style.separator&&style.focused);assert.notEqual(style.background,'rgba(0, 0, 0, 0)');await capture('revision-'+theme+'-menu');
      await evaluate("document.querySelector('.remove-group').focus();document.activeElement.dispatchEvent(new KeyboardEvent('keydown',{key:'Enter',bubbles:true,cancelable:true}))");await pause(80);
      assert.deepEqual(await evaluate("window.calls.findLast(c=>c.cmd==='move_to_group').args"),{ids:['row-0'],groupId:null});
    });
    await test('R05 '+theme+' 混合内容卡片等宽等高且状态完整',async()=>{
      await metrics();await nav();await appearance({theme,layout:'cards'});
      await evaluate("window.rows[0].summary='长摘要'.repeat(200);window.rows[0].pinned=true;window.rows[0].sensitive=true;window.rows[0].expires_at=Date.now()+120000;window.emitEvent('history-changed')");await pause(100);
      const boxes=await evaluate("[...document.querySelectorAll('.item')].slice(0,6).map(n=>{const r=n.getBoundingClientRect();return {w:r.width,h:r.height,inside:[...n.querySelectorAll('.summary,.content,.sensitive-badge,.file-status,.item-badges,.source-name')].every(c=>{const b=c.getBoundingClientRect();return b.bottom<=r.bottom&&b.right<=r.right}),summary:getComputedStyle(n.querySelector('.summary')).webkitLineClamp}})");
      assert(boxes.every(b=>b.h===boxes[0].h&&Math.abs(b.w-boxes[0].w)<1&&b.inside&&b.summary==='2'),JSON.stringify(boxes));
      await capture('revision-'+theme+'-cards');
    });
    for(const width of [1000,390])await test('R06 '+theme+' '+width+' 分类草稿、取消与独立底栏',async()=>{
      await metrics(width);await nav();await appearance({theme,compact:width===390});await click('.settings-toggle');await until(()=>evaluate("!!document.querySelector('.settings-view form')"),'设置未打开');
      const category=async id=>width===390?change('select[aria-label=设置分类]',id):click(`.settings-nav button:nth-child(${['general','appearance','history','privacy','hotkeys'].indexOf(id)+1})`);
      await category('appearance');await change('[aria-label=主题]',theme==='light'?'dark':'light');
      await category('privacy');assert.equal(await evaluate("[...document.querySelectorAll('fieldset')].filter(n=>getComputedStyle(n).display!=='none').map(n=>n.dataset.category).join()"),'privacy');await evaluate("document.querySelector('.add-app input').value='unsaved.exe';document.querySelector('.add-app input').dispatchEvent(new Event('input',{bubbles:true}))");
      await category('appearance');assert.equal(await evaluate("document.querySelector('[aria-label=主题]').value"),theme==='light'?'dark':'light');
      await category('privacy');assert.equal(await evaluate("document.querySelector('.add-app input').value"),'unsaved.exe');
      await evaluate("document.querySelector('.settings-content').scrollTop=99999");
      const bounds=await evaluate("(()=>{const c=document.querySelector('.settings-content').getBoundingClientRect(),f=document.querySelector('.settings-actions').getBoundingClientRect(),n=document.querySelector('.settings-cancel').getBoundingClientRect();return {content:c.bottom,footer:f.top,bottom:f.bottom,cancel:n.bottom,overflow:document.documentElement.scrollWidth>innerWidth};})()");
      assert(bounds.content<=bounds.footer+1&&bounds.bottom<=700&&bounds.cancel<=700&&!bounds.overflow,JSON.stringify(bounds));
      await capture('revision-'+theme+'-settings-'+width);
      await click('.settings-cancel');await pause(60);assert.equal(await evaluate("document.querySelector('.settings-view')"),null);
      await click('.settings-toggle');await until(()=>evaluate("!!document.querySelector('.settings-view form')"),'设置未重开');
      assert.equal(await evaluate("document.querySelector('[aria-label=主题]').value"),theme);assert.equal(await evaluate("document.querySelector('.add-app input').value"),'');
      assert.equal(await evaluate('window.settingUpdates.length'),0);
    });
  }
  await test('R06 长内容独立滚动、跨分类统一保存及取消恢复新保存值',async()=>{
    await metrics();await nav();await click('.settings-toggle');await until(()=>evaluate("!!document.querySelector('.settings-view form')"),'设置未打开');
    await click('.settings-nav button:nth-child(4)');
    await evaluate("window.settings.ignored_apps=Array.from({length:80},(_,i)=>'synthetic'+i+'.exe')");
    await click('.settings-cancel');await click('.settings-toggle');await until(()=>evaluate("!!document.querySelector('.settings-view form')"),'设置未重开');
    await click('.settings-nav button:nth-child(4)');
    const top=await evaluate("document.querySelector('.settings-actions').getBoundingClientRect().top");
    await evaluate("const n=document.querySelector('.settings-content');n.scrollTop=n.scrollHeight");
    assert(await evaluate("document.querySelector('.settings-content').scrollTop>500"));
    assert.equal(await evaluate("document.querySelector('.settings-actions').getBoundingClientRect().top"),top);
    await capture('revision-settings-long');
    await click('.settings-nav button:nth-child(2)');await change('[aria-label=主题]','dark');
    await click('.settings-nav button:nth-child(1)');await evaluate("document.querySelector('input[type=checkbox]').click()");
    await click('.settings-view button[type=submit]');await pause(100);
    assert.equal(await evaluate('window.settingUpdates.length'),1);assert.equal(await evaluate('window.settings.theme'),'dark');assert.equal(await evaluate('window.settings.hide_on_blur'),true);
    await click('.settings-nav button:nth-child(2)');await change('[aria-label=主题]','light');await click('.settings-cancel');
    assert.equal(await evaluate('document.documentElement.dataset.theme'),'dark');
    await click('.settings-toggle');await until(()=>evaluate("!!document.querySelector('.settings-view form')"),'设置未重开');
    assert.equal(await evaluate("document.querySelector('[aria-label=主题]').value"),'dark');
  });
  await test('R03 紧凑分组与类型字号、行高一致',async()=>{
    await metrics(390);await nav();await appearance({compact:true});
    assert.equal(await evaluate("getComputedStyle(document.querySelector('.compact-groups')).fontSize"),await evaluate("getComputedStyle(document.querySelector('.filter-label')).fontSize"));
    assert.equal(await evaluate("getComputedStyle(document.querySelector('.compact-groups select')).lineHeight"),await evaluate("getComputedStyle(document.querySelector('.kind-filters button')).lineHeight"),JSON.stringify(await evaluate("[...document.querySelectorAll('.compact-groups select,.kind-filters button')].map(n=>({tag:n.outerHTML,line:getComputedStyle(n).lineHeight}))")));
  });

  await test('R07 常规设置入口位于分组侧栏底部，顶部仅紧凑切换与窗口操作',async()=>{
    await metrics();await nav();
    assert.equal(await evaluate("document.querySelector('header .settings-toggle,header .count')"),null);
    assert.equal(await evaluate("document.querySelectorAll('header>button').length"),1);
    const bounds=await evaluate("(()=>{const n=document.querySelector('.sidebar-bottom .settings-toggle'),b=n.getBoundingClientRect(),s=n.closest('.group-sidebar').getBoundingClientRect();return {inside:b.left>=s.left&&b.right<=s.right,gap:s.bottom-b.bottom,available:b.width>0&&b.height>0};})()");
    assert(bounds.inside&&bounds.available&&bounds.gap<=25,JSON.stringify(bounds));
    await capture('followup-main-sidebar');
    await click('.sidebar-bottom .settings-toggle');await until(()=>evaluate("!!document.querySelector('.settings-view form')"),'侧栏设置未打开');
  });
  for(const theme of ['light','dark'])await test('R07 '+theme+' 右侧独立底栏、左提示与右按钮',async()=>{
    await metrics();await nav();await appearance({theme});await click('.settings-toggle');await until(()=>evaluate("!!document.querySelector('.settings-view form')"),'设置未打开');
    const box=await evaluate("(()=>{const r=n=>document.querySelector(n).getBoundingClientRect().toJSON();return {nav:r('.settings-nav'),content:r('.settings-content'),footer:r('.settings-actions'),feedback:r('.settings-feedback'),buttons:r('.settings-buttons')};})()");
    assert.equal(box.footer.left,box.content.left);assert.equal(box.footer.right,box.content.right);
    assert(box.nav.bottom>=box.footer.bottom&&box.nav.right<=box.footer.left+1&&box.content.bottom<=box.footer.top+1,JSON.stringify(box));
    assert(box.feedback.right<=box.buttons.left&&box.footer.right-box.buttons.right<=25,JSON.stringify(box));
    await capture('followup-'+theme+'-settings-footer');
    await click('.settings-nav button:nth-child(3)');await evaluate("const n=document.querySelector('input[type=number]');n.value='10';n.dispatchEvent(new Event('input',{bubbles:true}))");
    await click('.settings-buttons button[type=submit]');await until(()=>evaluate("!!document.querySelector('.settings-feedback .delete-confirm')"),'清理确认不在左侧提示区');
    assert(await evaluate("document.querySelector('.settings-content').getBoundingClientRect().bottom<=document.querySelector('.settings-actions').getBoundingClientRect().top+1"));
  });
  await test('R07 紧凑入口可用且窄窗底栏按钮靠右',async()=>{
    await metrics(390);await nav();await appearance({compact:true});
    assert.equal(await evaluate("document.querySelectorAll('.settings-toggle').length"),1);
    assert(await evaluate("document.querySelector('footer .settings-toggle').getBoundingClientRect().width>0"));
    await click('footer .settings-toggle');await until(()=>evaluate("!!document.querySelector('.settings-view form')"),'紧凑设置未打开');
    const b=await evaluate("(()=>{const f=document.querySelector('.settings-actions').getBoundingClientRect(),t=document.querySelector('.settings-feedback').getBoundingClientRect(),a=document.querySelector('.settings-buttons').getBoundingClientRect();return {right:f.right-a.right,aligned:t.right<=a.left,bottom:a.bottom,overflow:document.documentElement.scrollWidth>innerWidth};})()");
    assert(b.right<=17&&b.aligned&&b.bottom<=700&&!b.overflow,JSON.stringify(b));
    await capture('followup-compact-settings-footer');
    await click('.settings-cancel');assert(await evaluate("!!document.querySelector('footer .settings-toggle')"));
  });

  await test('R08 品牌与设计稿结构：线性图标、搜索提示、侧栏分组、预览底部操作',async()=>{
    await metrics();await nav();
    assert.equal(await evaluate("document.querySelector('header h1').textContent"),'片筏剪贴板');
    assert(await evaluate("document.title.includes('ClipRaft')"));
    assert(await evaluate("!!document.querySelector('.brand-mark svg')&&!!document.querySelector('.searchbox svg')&&!!document.querySelector('.group-views svg')"));
    assert.equal(await evaluate("document.querySelector('.searchbox kbd').textContent"),'Ctrl F');
    assert.equal(await evaluate("document.querySelector('.groups-heading').textContent"),'我的分组');
    const bounds=await evaluate("(()=>{const r=n=>document.querySelector(n).getBoundingClientRect();return {bottom:r('.detail-actions').bottom,detail:r('.detail').bottom,status:r('.main-statusbar').top,statusLeft:r('.main-statusbar').left,mainLeft:r('main').left};})()");
    assert(bounds.bottom<bounds.detail&&bounds.detail<=bounds.status+1&&Math.abs(bounds.statusLeft-bounds.mainLeft)<=1,JSON.stringify(bounds));
    await evaluate("document.querySelector('.item').focus();document.activeElement.dispatchEvent(new KeyboardEvent('keydown',{key:'f',ctrlKey:true,bubbles:true,cancelable:true}))");await pause(50);
    assert(await evaluate("document.activeElement===document.querySelector('input[type=search]')"));
    await capture('brand-light-list');
  });
  for(const theme of ['light','dark'])await test('R08 '+theme+' 紧凑置顶标识可见且不挤出内容',async()=>{
    await metrics(390);await nav();await evaluate("window.rows[0].pinned=true;window.rows[0].summary='用于检验长摘要与置顶标识的合成内容'.repeat(8);window.emitEvent('history-changed')");await pause(80);await appearance({theme,compact:true});
    const row=await evaluate("(()=>{const n=document.querySelector('[data-id=\"row-0\"]'),p=n.querySelector('.compact-pin'),r=n.getBoundingClientRect(),b=p.getBoundingClientRect();return {height:r.height,pinWidth:b.width,pinInside:b.right<=r.right&&b.left>=r.left&&b.bottom<=r.bottom,label:p.getAttribute('aria-label'),summary:n.querySelector('.summary').getBoundingClientRect().right,pinLeft:b.left};})()");
    assert.equal(row.height,54);assert(row.pinWidth>0&&row.pinInside&&row.summary<=row.pinLeft,JSON.stringify(row));assert.equal(row.label,'置顶');
    assert.equal(await evaluate("document.querySelector('[data-id=\"row-1\"] .compact-pin')"),null);
    await capture('brand-'+theme+'-compact-pinned');
  });

  await test('R09 小尺寸紧凑图标与预览右上刷新图标',async()=>{
    await metrics();await nav();
    const b=await evaluate("(()=>{const n=document.querySelector('.compact-toggle'),r=n.getBoundingClientRect(),h=document.querySelector('header').getBoundingClientRect(),f=document.querySelector('.preview-refresh'),p=f.getBoundingClientRect(),c=document.querySelector('.detail-caption').getBoundingClientRect();return {w:r.width,h:r.height,inside:r.top>=h.top&&r.bottom<=h.bottom,icon:!!n.querySelector('svg'),label:n.getAttribute('aria-label'),refresh:!!f.querySelector('svg'),refreshLabel:f.getAttribute('aria-label'),right:c.right-p.right,top:p.top-c.top,bottom:!!document.querySelector('.detail-bottom .preview-refresh')};})()");
    assert.equal(b.w,28);assert.equal(b.h,28);assert(b.inside&&b.icon&&b.refresh&&!b.bottom);assert.equal(b.label,'紧凑模式');assert.equal(b.refreshLabel,'刷新状态');assert(b.right<=1&&b.top<=1,JSON.stringify(b));
    await click('.compact-toggle');await pause(80);assert.equal(await evaluate("document.querySelector('.compact-toggle').getAttribute('aria-label')"),'展开视图');assert.equal(await evaluate("document.querySelector('.compact-toggle').getAttribute('aria-pressed')"),'true');
  });
  for(const theme of ['light','dark'])for(const mode of ['list','compact','cards'])await test('R10 '+theme+'/'+mode+' 统一类型图标与左侧图片缩略图',async()=>{
    await metrics(mode==='compact'?390:1000);await nav();
    await evaluate("window.rows[4]={...window.rows[4],kind:'image',thumbnail:'synthetic-thumb',summary:'合成图片长摘要'.repeat(30)};window.rows[0].pinned=true;window.emitEvent('history-changed')");await pause(80);
    await appearance({theme,layout:mode==='cards'?'cards':'list',compact:mode==='compact'});await click('[data-id=\"row-4\"]');
    await until(()=>evaluate("document.querySelector('[data-id=\"row-4\"] .record-thumbnail img')?.naturalWidth===160"),'右侧合成图片未加载');
    const boxes=await evaluate("(()=>{const n=document.querySelector('[data-id=\"row-4\"]'),r=n.getBoundingClientRect(),i=n.querySelector('.type-icon')?.getBoundingClientRect(),p=n.querySelector('.record-thumbnail').getBoundingClientRect(),c=n.querySelector('.content').getBoundingClientRect();return {row:r.toJSON(),icon:i?.toJSON(),preview:p.toJSON(),content:c.toJSON(),fit:getComputedStyle(n.querySelector('img')).objectFit,icons:[...document.querySelectorAll('.item>.type-icon')].slice(0,6).map(n=>({w:n.getBoundingClientRect().width,h:n.getBoundingClientRect().height}))};})()");
    assert.equal(boxes.fit,'contain');
    assert(await evaluate("!!document.querySelector('[data-id=\"row-2\"] .record-thumbnail .image-placeholder svg')&&!!document.querySelector('[data-id=\"row-2\"] .record-thumbnail .image-placeholder').getAttribute('aria-label')"));
    if(mode!=='cards'){
      const size=mode==='compact'?32:48;assert(boxes.icons.every(b=>b.w===size&&b.h===size),JSON.stringify(boxes.icons));
      assert(!boxes.icon&&boxes.preview.right<=boxes.content.left&&boxes.preview.left>=boxes.row.left,JSON.stringify(boxes));
      if(mode==='compact'){assert.equal(boxes.row.height,54);assert.equal(boxes.preview.width,52);assert.equal(boxes.preview.height,38);}
    }else{assert.equal(boxes.row.height,248);assert(boxes.preview.bottom<=boxes.content.top,JSON.stringify(boxes));}
    assert(await evaluate("document.documentElement.scrollWidth<=innerWidth"));
    await capture('r10-'+theme+'-'+mode);
    await evaluate("window.rows[4].sensitive=true;window.rows[4].summary='合成敏感••••';window.emitEvent('history-changed')");await pause(60);
    assert.equal(await evaluate("document.querySelector('[data-id=\"row-4\"] .record-thumbnail')"),null);
    assert(await evaluate("!!document.querySelector('[data-id=\"row-4\"] .type-icon svg')"));
  });


  for(const theme of ['light','dark'])await test('R11 '+theme+' 常规图标与图片共用中心线且正文对齐',async()=>{
    await metrics();await nav();await appearance({theme});
    await evaluate("window.rows[4]={...window.rows[4],kind:'image',thumbnail:'synthetic-thumb'};window.emitEvent('history-changed')");await pause(80);
    const rows=await evaluate("[...document.querySelectorAll('.item')].slice(0,6).map(n=>{const r=n.getBoundingClientRect(),v=n.querySelector('.type-icon,.record-thumbnail').getBoundingClientRect(),c=n.querySelector('.content').getBoundingClientRect();return {rowHeight:r.height,cx:(v.left+v.right)/2,cy:(v.top+v.bottom)/2,rowCy:(r.top+r.bottom)/2,textLeft:c.left,w:v.width,h:v.height,image:!!n.querySelector('.record-thumbnail')};})");
    assert(rows.every(r=>r.rowHeight>=84&&Math.abs(r.cx-rows[0].cx)<=1&&Math.abs(r.cy-r.rowCy)<=1&&Math.abs(r.textLeft-rows[0].textLeft)<=1),JSON.stringify(rows));
    assert(rows.filter(r=>r.image).every(r=>r.w===80&&r.h===60));assert(rows.filter(r=>!r.image).every(r=>r.w===48&&r.h===48));
    assert(await evaluate("[...document.querySelectorAll('.item>.type-icon>.ui-icon')].every(n=>n.getBoundingClientRect().width===26&&n.getBoundingClientRect().height===26)"));
    await capture('r11-'+theme+'-centered-list');
  });
  await test('R10 紧凑浮窗请求跟随选择、设置、隐藏与重新显示',async()=>{
    await metrics(390);await nav();
    assert.equal(await evaluate("window.calls.filter(c=>c.cmd==='sync_compact_preview').at(-1)?.args.id"),null);
    await appearance({compact:true});await click('[data-id=\"row-2\"]');await pause(70);
    assert.equal(await evaluate("window.calls.filter(c=>c.cmd==='sync_compact_preview').at(-1).args.id"),'row-2');
    await click('.settings-toggle');await pause(70);assert.equal(await evaluate("window.calls.filter(c=>c.cmd==='sync_compact_preview').at(-1).args.id"),null);
    await click('.settings-cancel');await pause(70);assert.equal(await evaluate("window.calls.filter(c=>c.cmd==='sync_compact_preview').at(-1).args.id"),'row-2');
    await evaluate("window.emitEvent('main-hidden')");await click('[data-id=\"row-4\"]');await pause(70);assert.equal(await evaluate("window.calls.filter(c=>c.cmd==='sync_compact_preview').at(-1).args.id"),null);
    await evaluate("window.emitEvent('main-shown')");await pause(70);assert.equal(await evaluate("window.calls.filter(c=>c.cmd==='sync_compact_preview').at(-1).args.id"),'row-4');
    await appearance({compact:false});assert.equal(await evaluate("window.calls.filter(c=>c.cmd==='sync_compact_preview').at(-1).args.id"),null);
  });

  await test('B04 浮窗显示回送 focus 时不会重复请求或形成反馈环',async()=>{
    await metrics(390);await nav();await evaluate('window.echoPreviewFocus=true;window.calls=[]');await appearance({compact:true});await pause(100);
    assert.equal(await evaluate("window.calls.filter(c=>c.cmd==='sync_compact_preview'&&c.args.id).length"),1);
    await evaluate("for(let i=0;i<20;i++){window.dispatchEvent(new Event('focus'));window.emitEvent('main-activated')}");await pause(70);
    assert.equal(await evaluate("window.calls.filter(c=>c.cmd==='sync_compact_preview'&&c.args.id).length"),1);
    await click('[data-id=\"row-2\"]');await pause(80);assert.equal(await evaluate("window.calls.filter(c=>c.cmd==='sync_compact_preview'&&c.args.id).length"),2);assert.equal(await evaluate('window.previewFocusEchoes'),2);
    await evaluate("window.emitEvent('main-deactivated')");await pause(60);assert.equal(await evaluate("window.calls.filter(c=>c.cmd==='sync_compact_preview').at(-1).args.id"),null);
    await evaluate("window.emitEvent('main-activated')");await pause(60);assert.equal(await evaluate("window.calls.filter(c=>c.cmd==='sync_compact_preview').at(-1).args.id"),'row-2');
  });
  await test('B04 常规窗口反复 focus/blur 不清空或重复加载普通详情',async()=>{
    await metrics();await nav();await until(()=>evaluate("!!document.querySelector('.text-detail')"),'普通详情未加载');
    const before=await evaluate("window.calls.filter(c=>c.cmd==='item_detail').length");
    await evaluate("for(let i=0;i<20;i++){window.dispatchEvent(new Event('blur'));window.dispatchEvent(new Event('focus'));window.emitEvent('main-activated')}");await pause(80);
    assert.equal(await evaluate("window.calls.filter(c=>c.cmd==='item_detail').length"),before);
    assert.equal(await evaluate("document.querySelector('.text-detail')?.textContent"),'合成详情');
  });
  const floatingNav=async(query='')=>{await send('Emulation.setDeviceMetricsOverride',{width:360,height:420,deviceScaleFactor:1,mobile:false});await send('Page.navigate',{url:origin+'/?panel=preview'+query});await until(()=>evaluate("!!document.querySelector('.floating-preview')"),'浮窗入口未加载');await pause(70);};
  const floatingContent=(id='image')=>({generation:42,item:{id,kind:'image',summary:'合成图片',thumbnail:'synthetic-thumb',preview:'synthetic-thumb',files:[],formats:[]},detail:{text:'',files:[],truncated:false}});
  await test('R10 浮窗独立入口与真实图片 contain、无主窗口或粘贴入口',async()=>{
    await floatingNav();await evaluate(`window.emitEvent('compact-preview-changed',${JSON.stringify(floatingContent())})`);await until(()=>evaluate("document.querySelector('.floating-preview img')?.naturalWidth===160"),'浮窗图片未实际加载');
    assert.equal(await evaluate("getComputedStyle(document.querySelector('.floating-preview img')).objectFit"),'contain');
    assert(await evaluate("document.querySelector('.floating-preview img').getBoundingClientRect().width>=280&&Math.abs(document.querySelector('.floating-preview img').getBoundingClientRect().width-document.querySelector('.floating-body').clientWidth)<=1"));
    assert.equal(await evaluate("document.querySelector('.main-window,.quick,button')"),null);
    assert.equal(await evaluate("window.calls.filter(c=>['list_items','item_detail','panel_state','paste_item','sync_main_layout','sync_compact_preview'].includes(c.cmd)).length"),0);
    assert(await evaluate("document.documentElement.scrollWidth<=360"));await capture('r10-floating-image-light');
    await appearance({theme:'dark'});assert.equal(await evaluate('document.documentElement.dataset.theme'),'dark');await capture('r10-floating-image-dark');
  });
  await test('R10 浮窗文本、文件、敏感打码与清空',async()=>{
    const content=floatingContent('text');content.item.kind='text';content.item.thumbnail=null;content.item.preview=null;content.detail.text='<b>合成文本</b>\n'+('独立预览内容'.repeat(180));
    await evaluate(`window.emitEvent('compact-preview-changed',${JSON.stringify(content)})`);await pause(60);
    assert.equal(await evaluate("document.querySelector('.floating-text b')"),null);assert(await evaluate("document.querySelector('.floating-content').scrollHeight>document.querySelector('.floating-content').clientHeight"));await capture('r10-floating-text');
    content.item.kind='files';content.detail.text='';content.detail.files=[{path:'C:/synthetic/missing.png',exists:false}];
    await evaluate(`window.emitEvent('compact-preview-changed',${JSON.stringify(content)})`);await pause(40);assert(await evaluate("document.querySelector('.floating-files').textContent.includes('源文件已不存在')"));
    content.item.sensitive=true;content.item.thumbnail='synthetic-thumb';content.item.preview='synthetic-thumb';content.detail.files=[];content.detail.text='合成敏感••••';
    await evaluate(`window.emitEvent('compact-preview-changed',${JSON.stringify(content)})`);await pause(40);assert.equal(await evaluate("document.querySelector('.floating-preview img')"),null);assert(await evaluate("document.querySelector('.floating-preview header').textContent.includes('已打码')"));
    await evaluate("window.emitEvent('compact-preview-changed',null)");await pause(40);assert.equal(await evaluate("document.querySelector('.floating-text,.floating-files,img')"),null);
  });
  await test('R10 浮窗迟到初始快照不覆盖更新或清空',async()=>{
    await floatingNav('&holdpreview=1');await until(()=>evaluate("typeof window.releasePreviewSnapshot==='function'"),'未挂起初始快照');
    await evaluate("window.emitEvent('compact-preview-changed',null)");await appearance({theme:'dark'});await evaluate(`window.releasePreviewSnapshot(${JSON.stringify(floatingContent('late'))})`);await pause(80);
    assert.equal(await evaluate("document.querySelector('.floating-preview img')"),null);assert.equal(await evaluate('document.documentElement.dataset.theme'),'dark');
  });

  for(const theme of ['light','dark'])await test('R12 '+theme+' 两行摘要、圆角按钮和详情复位',async()=>{
    await metrics();await nav();await appearance({theme});
    await evaluate("window.rows[0].summary='单行长句自动换行验证'.repeat(35);window.rows[4].summary='const value = 42;\\nfunction add(a, b) { return a + b; }';window.emitEvent('history-changed')");await pause(90);
    const size=await evaluate("(()=>{const n=document.querySelector('[data-id=\"row-0\"]'),s=n.querySelector('.summary'),r=s.getBoundingClientRect(),c=n.querySelector('.card-status').getBoundingClientRect();return {height:r.height,line:parseFloat(getComputedStyle(s).lineHeight),bottom:c.bottom,rowBottom:n.getBoundingClientRect().bottom,white:getComputedStyle(s).whiteSpace,radius:getComputedStyle(document.querySelector('.paste-primary')).borderRadius}})()");
    assert(size.height>size.line&&size.height<=size.line*2+1,JSON.stringify(size));assert(size.bottom<=size.rowBottom);assert.equal(size.white,'pre-wrap');assert.equal(size.radius,'8px');
    await evaluate("window.rows[0].summary='Z'.repeat(600);window.emitEvent('history-changed')");await pause(50);assert(await evaluate("document.documentElement.scrollWidth<=innerWidth"));
    await evaluate("document.querySelector('.detail-scroll').scrollTop=100;window.rows[0].summary='长文本';const original=window.__TAURI_INTERNALS__.invoke;window.__TAURI_INTERNALS__.invoke=(cmd,args)=>cmd==='item_detail'?Promise.resolve({text:'从正文开头开始\\n'+('正文内容'.repeat(2000)),files:[],truncated:false}):original(cmd,args);window.emitEvent('history-changed')");await pause(90);
    await evaluate("document.querySelector('.detail-scroll').scrollTop=160");assert(await evaluate("document.querySelector('.detail-scroll').scrollTop>0"));
    await click('.preview-refresh');await pause(60);assert.equal(await evaluate("document.querySelector('.detail-scroll').scrollTop"),0);
    await evaluate("document.querySelector('.detail-scroll').scrollTop=160");
    await click('[data-id="row-4"]');await pause(80);assert.equal(await evaluate("document.querySelector('.detail-scroll').scrollTop"),0);assert(await evaluate("document.querySelector('.text-detail').textContent.startsWith('从正文开头开始')"));
    await capture('r12-'+theme+'-list');
  });
  for(const theme of ['light','dark'])await test('R12 '+theme+' 紧凑对齐、状态、几何同步与表单',async()=>{
    await metrics(390);await nav();await appearance({theme,compact:true});
    await evaluate("window.rows[4]={...window.rows[4],kind:'image',thumbnail:'synthetic-thumb'};window.rows[1].summary='极长文件名'.repeat(80);window.emitEvent('history-changed')");await pause(90);
    const geometry=await evaluate("[...document.querySelectorAll('.item')].slice(0,6).map(n=>{const r=n.getBoundingClientRect(),i=n.querySelector('.type-icon,.record-thumbnail').getBoundingClientRect(),c=n.querySelector('.content').getBoundingClientRect(),s=n.querySelector('.sensitive-badge,.file-status')?.getBoundingClientRect();return {h:r.height,cx:(i.left+i.right)/2,cy:(i.top+i.bottom)/2,y:(r.top+r.bottom)/2,left:c.left,status:s?.bottom,bottom:r.bottom}})");
    assert(geometry.every(r=>r.h===54&&Math.abs(r.cx-geometry[0].cx)<=1&&Math.abs(r.cy-r.y)<=1&&r.left===geometry[0].left&&(!r.status||r.status<=r.bottom)),JSON.stringify(geometry));
    assert(await evaluate("Math.abs(document.querySelector('.compact-groups select').getBoundingClientRect().left-document.querySelector('.kind-filters').getBoundingClientRect().left)<=1"));
    await click('[data-id="row-4"]');await pause(90);
    assert.equal(await evaluate("window.calls.filter(c=>c.cmd==='position_compact_preview').at(-1)?.args.anchor?.id"),'row-4');
    const before=await evaluate("window.calls.filter(c=>c.cmd==='sync_compact_preview').length");
    await evaluate("document.querySelector('.list').scrollTop=500;document.querySelector('.list').dispatchEvent(new Event('scroll'))");await pause(90);
    assert.equal(await evaluate("window.calls.filter(c=>c.cmd==='position_compact_preview').at(-1)?.args.anchor"),null);
    assert.equal(await evaluate("window.calls.filter(c=>c.cmd==='sync_compact_preview').length"),before);
    await evaluate("document.querySelector('.list').scrollTop=0;document.querySelector('.list').dispatchEvent(new Event('scroll'))");await pause(60);await capture('r12-'+theme+'-compact');
    await click('.settings-toggle');await pause(60);await click('.settings-nav button:nth-child(2)').catch(()=>{});
    const checkbox=await evaluate("(()=>{const n=document.querySelector('.settings-view input[type=checkbox]');const s=getComputedStyle(n);return {appearance:s.appearance,w:s.width,radius:s.borderRadius,accent:s.accentColor}})()");
    assert.equal(checkbox.appearance,'none');assert.equal(checkbox.w,'15px');assert.equal(checkbox.radius,'4px');await capture('r12-'+theme+'-settings');
  });

  await test('R12 高图片完整展示并在切换记录后回到顶部',async()=>{
    await metrics();await nav();
    await evaluate("const original=window.__TAURI_INTERNALS__.convertFileSrc;window.__TAURI_INTERNALS__.convertFileSrc=hash=>hash==='tall'?'data:image/svg+xml,'+encodeURIComponent('<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"160\" height=\"800\"><rect width=\"160\" height=\"80\" fill=\"#277d77\"/><rect y=\"80\" width=\"160\" height=\"720\" fill=\"#eaf4f2\"/></svg>'):original(hash);window.rows[2]={...window.rows[2],kind:'image',thumbnail:'tall',preview:'tall'};window.emitEvent('history-changed')");await pause(70);await click('[data-id="row-2"]');
    await until(()=>evaluate("document.querySelector('.detail img')?.naturalHeight===800"),'高图未解码');
    const box=await evaluate("(()=>{const n=document.querySelector('.detail img'),r=n.getBoundingClientRect(),s=document.querySelector('.detail-scroll');return {w:r.width,h:r.height,scroll:s.scrollTop,total:s.scrollHeight,height:s.clientHeight,max:getComputedStyle(n).maxHeight}})()");
    assert(Math.abs(box.h/box.w-5)<.01,JSON.stringify(box));assert.equal(box.max,'none');assert.equal(box.scroll,0);assert(box.total>box.height);
    await evaluate("document.querySelector('.detail-scroll').scrollTop=500");await click('[data-id="row-4"]');await pause(50);await click('[data-id="row-2"]');await pause(70);assert.equal(await evaluate("document.querySelector('.detail-scroll').scrollTop"),0);
    await capture('r12-tall-image');
  });
  await test('R12 浮窗同记录更新复位与双向箭头',async()=>{
    await floatingNav();const content=floatingContent('same');content.item.kind='text';content.item.thumbnail=null;content.item.preview=null;content.detail.text='正文起点\n'+'预览内容'.repeat(4000);
    await evaluate(`window.emitEvent('compact-preview-changed',${JSON.stringify(content)})`);await pause(60);
    await evaluate("document.querySelector('.floating-content').scrollTop=300");assert(await evaluate("document.querySelector('.floating-content').scrollTop>0"));
    await evaluate(`window.emitEvent('compact-preview-changed',${JSON.stringify(content)});window.emitEvent('compact-preview-geometry',{right:false,arrow:210})`);await pause(70);
    assert.equal(await evaluate("document.querySelector('.floating-content').scrollTop"),0);assert(await evaluate("document.querySelector('.floating-shell').classList.contains('on-left')"));
    assert.equal(await evaluate("getComputedStyle(document.querySelector('.preview-arrow')).top"),'203px');await capture('r12-floating-left');
    await evaluate("window.emitEvent('compact-preview-geometry',{right:true,arrow:40})");await pause(40);assert(await evaluate("document.querySelector('.floating-shell').classList.contains('on-right')"));
    assert.equal(await evaluate("getComputedStyle(document.querySelector('.preview-arrow')).top"),'33px');
  });

  for(const theme of ['light','dark'])await test('R13 '+theme+' 分组色跨侧栏标签详情一致且改名稳定',async()=>{
    await metrics();await nav();await appearance({theme});
    const colors=await evaluate("(()=>{const side=document.querySelector('.colored-group'),dot=side.querySelector('.group-dot'),badge=document.querySelector('[data-id=\"row-0\"] .group-badge'),detail=document.querySelector('.detail-group');return {side:getComputedStyle(side).color,dot:getComputedStyle(dot).backgroundColor,badge:getComputedStyle(badge).color,detail:getComputedStyle(detail).color,fill:getComputedStyle(badge).backgroundColor,other:getComputedStyle(document.querySelectorAll('.colored-group')[1]).color}})()");
    assert.equal(colors.side,colors.badge);assert.equal(colors.dot,colors.badge);assert.equal(colors.detail,colors.badge);assert.notEqual(colors.other,colors.badge);
    const contrast=(a,b)=>{const lum=s=>{const rgb=s.match(/[\d.]+/g).slice(0,3).map(Number).map(v=>v/255).map(v=>v<=.04045?v/12.92:((v+.055)/1.055)**2.4);return rgb[0]*.2126+rgb[1]*.7152+rgb[2]*.0722};const x=lum(a),y=lum(b);return (Math.max(x,y)+.05)/(Math.min(x,y)+.05)};
    assert(contrast(colors.badge,colors.fill)>=4.5,JSON.stringify(colors));
    await evaluate("const original=window.__TAURI_INTERNALS__.invoke;window.__TAURI_INTERNALS__.invoke=(cmd,args)=>cmd==='list_groups'?Promise.resolve([{id:7,name:'改名后的工作分组',count:35},{id:8,name:'另一分组',count:2}]):original(cmd,args);window.emitEvent('history-changed')");await pause(80);
    assert.equal(await evaluate("getComputedStyle(document.querySelector('.colored-group')).color"),colors.badge);assert(await evaluate("document.querySelector('[data-id=\"row-0\"] .group-badge').textContent.includes('改名后')"));
    await capture('r13-'+theme+'-groups');
    await appearance({layout:'cards'});assert.equal(await evaluate("getComputedStyle(document.querySelector('[data-id=\"row-0\"] .group-badge')).color"),colors.badge);
    await appearance({layout:'list',compact:true});await change('.compact-groups select','7');await pause(60);assert.equal(await evaluate("getComputedStyle(document.querySelector('.compact-groups select')).color"),colors.badge);
    await metrics();await nav();await appearance({theme});assert.equal(await evaluate("getComputedStyle(document.querySelector('.colored-group')).color"),colors.badge);
  });
  await test('R13 透明轨道与居中checkbox保持语义和键盘操作',async()=>{
    await metrics();await nav();await click('.settings-toggle');await pause(70);
    const before=await evaluate("(()=>{const n=document.querySelector('input[type=checkbox]'),s=getComputedStyle(n,'::after');n.focus();return {checked:n.checked,transform:s.transform,w:s.width,h:s.height,track:getComputedStyle(document.querySelector('.settings-content'),'::-webkit-scrollbar-track').backgroundColor,thumb:getComputedStyle(document.querySelector('.settings-content'),'::-webkit-scrollbar-thumb').backgroundClip}})()");
    assert.equal(before.transform,'none');assert.equal(before.w,before.h);assert.equal(before.track,'rgba(0, 0, 0, 0)');assert.equal(before.thumb,'padding-box');
    await send('Input.dispatchKeyEvent',{type:'keyDown',key:' ',code:'Space',windowsVirtualKeyCode:32});await send('Input.dispatchKeyEvent',{type:'keyUp',key:' ',code:'Space',windowsVirtualKeyCode:32});
    assert.equal(await evaluate("document.activeElement.checked"),!before.checked);await capture('r13-checkbox-scrollbars');
  });

  const key=async(name,code,shift=false)=>{await send('Input.dispatchKeyEvent',{type:'keyDown',key:name,code:name,windowsVirtualKeyCode:code,modifiers:shift?8:0});await send('Input.dispatchKeyEvent',{type:'keyUp',key:name,code:name,windowsVirtualKeyCode:code,modifiers:shift?8:0});};
  await test('R14 分组焦点经方向键进入记录，Enter和Shift+Enter各粘贴一次',async()=>{
    await metrics();await nav();
    await evaluate("const b=document.querySelector('.colored-group');b.click();b.focus()");await pause(90);
    assert(await evaluate("document.activeElement.classList.contains('colored-group')"));const before=await selected();
    await key('ArrowDown',40);await pause(60);const after=await selected();assert.notEqual(after,before);
    assert.equal(await evaluate("document.activeElement.dataset.id"),after);
    await key('Enter',13);await pause(60);assert.equal(await evaluate("window.calls.filter(c=>c.cmd==='paste_item').length"),1);
    assert.deepEqual(await evaluate("window.calls.filter(c=>c.cmd==='paste_item').at(-1).args"),{id:after,plain:false});
    await key('ArrowUp',38);await pause(50);assert.equal(await evaluate("document.activeElement.dataset.id"),await selected());
    await key('Enter',13,true);await pause(50);assert.equal(await evaluate("window.calls.filter(c=>c.cmd==='paste_item').length"),2);assert.equal(await evaluate("window.calls.filter(c=>c.cmd==='paste_item').at(-1).args.plain"),true);
    await evaluate("document.querySelector('.searchbox input').focus()");await key('ArrowDown',40);await pause(40);assert(await evaluate("document.activeElement.matches('input[type=search]')"));
    await key('Enter',13);await pause(50);assert.equal(await evaluate("window.calls.filter(c=>c.cmd==='paste_item').length"),3);
    await click('.settings-toggle');await pause(40);await key('Enter',13);assert.equal(await evaluate("window.calls.filter(c=>c.cmd==='paste_item').length"),3);
  });

  await test('R14 紧凑分组提交后方向键和Enter继续操作记录，未提交时保留下拉行为',async()=>{
    await metrics(390);await nav();await appearance({compact:true});
    await evaluate("document.querySelector('.compact-groups select').focus()");
    // 未change时不劫持select的按键，也不发送粘贴。
    const prevented=await evaluate("(()=>{const n=document.activeElement;return !n.dispatchEvent(new KeyboardEvent('keydown',{key:'Enter',bubbles:true,cancelable:true}))})()");
    assert.equal(prevented,false);assert.equal(await evaluate("window.calls.filter(c=>c.cmd==='paste_item').length"),0);
    await change('.compact-groups select','7');await pause(90);
    assert(await evaluate("document.activeElement.matches('input[type=search]')"));
    const before=await selected();await key('ArrowDown',40);await pause(40);const after=await selected();assert.notEqual(after,before);
    assert(await evaluate("document.activeElement.matches('input[type=search]')"));
    await key('Enter',13);await pause(40);assert.deepEqual(await evaluate("window.calls.filter(c=>c.cmd==='paste_item').map(c=>c.args)"),[{id:after,plain:false}]);
    await key('ArrowUp',38);await pause(40);await key('Enter',13,true);await pause(40);
    assert.deepEqual(await evaluate("window.calls.filter(c=>c.cmd==='paste_item').at(-1).args"),{id:before,plain:true});
    // 点击记录后的焦点也可直接Enter；与常规模式共用同一处理。
    await evaluate("const n=document.querySelectorAll('.item')[1];n.click();n.focus()");const focused=await evaluate('document.activeElement.dataset.id');
    await key('Enter',13);await pause(40);assert.equal(await evaluate("window.calls.filter(c=>c.cmd==='paste_item').length"),3);assert.equal(await evaluate("window.calls.filter(c=>c.cmd==='paste_item').at(-1).args.id"),focused);
    // 回到全部及未分组，同样完成焦点交接。
    for(const value of ['all','ungrouped']){await evaluate("document.querySelector('.compact-groups select').focus()");await change('.compact-groups select',value);await pause(60);assert(await evaluate("document.activeElement.matches('input[type=search]')"));}
  });
  for(const theme of ['light','dark'])await test('R14 '+theme+' 浮窗透明边距、独立边框与阴影',async()=>{
    await floatingNav();await appearance({theme});await evaluate(`window.emitEvent('compact-preview-changed',${JSON.stringify(floatingContent())});window.emitEvent('compact-preview-geometry',{right:true,arrow:60})`);await pause(60);
    const b=await evaluate("(()=>{const n=document.querySelector('.floating-preview'),r=n.getBoundingClientRect(),s=getComputedStyle(n),root=getComputedStyle(document.documentElement),shell=getComputedStyle(document.querySelector('.floating-shell'));return {root:root.backgroundColor,shell:shell.backgroundColor,shadow:s.boxShadow,border:s.borderTopColor,w:s.borderTopWidth,left:r.left,top:r.top,right:innerWidth-r.right,bottom:innerHeight-r.bottom}})()");
    assert.equal(b.root,'rgba(0, 0, 0, 0)');assert.equal(b.shell,'rgba(0, 0, 0, 0)');assert.notEqual(b.shadow,'none');assert.equal(b.w,'1px');assert.notEqual(b.border,await evaluate("getComputedStyle(document.querySelector('.floating-preview')).backgroundColor"));assert(b.left>=18&&b.top>=8&&b.right>=8&&b.bottom>=10,JSON.stringify(b));
    // 用对比底色模拟桌面，验证阴影有绘制空间；这不是原生WebView透明验收。
    await evaluate("document.documentElement.style.setProperty('background','#b7bdc4','important')");await capture('r14-'+theme+'-floating-shadow');
  });

  for(const theme of ['light','dark'])await test('R15 '+theme+' 模式切换后键盘选中只显示一层边框',async()=>{
    await metrics();await nav();await appearance({theme});
    await evaluate("const n=document.querySelector('[data-id=\"row-4\"]');n.click();n.focus()");
    for(const compact of [true,false,true]){
      await metrics(compact?390:1000);await appearance({compact});await key('ArrowDown',40);await pause(50);
      const box=await evaluate("(()=>{const n=document.activeElement,s=getComputedStyle(n);return {id:n.dataset.id,selected:n.classList.contains('selected'),visible:n.matches(':focus-visible'),outline:s.outlineStyle,border:s.borderTopWidth,shadow:s.boxShadow}})()");
      assert.equal(box.id,await selected());assert(box.selected&&box.visible,JSON.stringify(box));assert.equal(box.outline,'none');assert.equal(box.border,'1px');assert.equal(box.shadow,'none');
      await capture('r15-'+theme+'-'+(compact?'compact':'list')+'-focus');
    }
    await evaluate("document.querySelector('[data-id=\"row-0\"]').focus()");
    assert.notEqual(await evaluate("getComputedStyle(document.activeElement).outlineStyle"),'none');
  });
  await test('R15 首条开关默认关闭，草稿取消与保存保持原设置协议',async()=>{
    await metrics();await nav();assert.equal(await evaluate('window.settings.select_first_on_show'),false);
    const toggle="(()=>{const label=[...document.querySelectorAll('.settings-view label.check')].find(n=>n.textContent.includes('每次唤出主窗口'));label.querySelector('input').click()})()";
    await click('.settings-toggle');await pause(40);await evaluate(toggle);await click('.settings-back');await pause(40);assert.equal(await evaluate('window.settings.select_first_on_show'),false);
    await click('.settings-toggle');await pause(40);await evaluate(toggle);await click('.settings-view button[type=submit]');await pause(50);assert.equal(await evaluate('window.settings.select_first_on_show'),true);await click('.settings-back');
  });
  for(const compact of [false,true])await test('R15 '+(compact?'紧凑':'普通')+' 唤出选择首条、滚动归零、保留筛选及手动选择优先',async()=>{
    await metrics(compact?390:1000);await nav();await appearance({compact});
    await click('[data-id="row-12"]');await evaluate("window.emitEvent('main-shown')");await pause(80);assert.equal(await selected(),'row-12');
    await appearance({select_first_on_show:true});
    await evaluate("window.rows.unshift({...window.rows[0],id:'new-head',summary:'隐藏期间新增的记录'});document.querySelector('.list').scrollTop=300;window.emitEvent('main-shown')");await pause(100);
    assert.equal(await selected(),'new-head');assert.equal(await evaluate("document.querySelector('.list').scrollTop"),0);
    if(compact)await change('.compact-groups select','7');else await click('.colored-group');await pause(70);
    await click('[data-id="row-12"]');await evaluate("window.emitEvent('main-shown')");await pause(80);assert.equal(await selected(),'new-head');assert(await evaluate("window.calls.filter(c=>c.cmd==='list_items').at(-1).args.filter.group_id===7"));
    // 刷新慢时用户自己选择的记录应保留，不能迟到后再次拉回顶部。
    await evaluate("const original=window.__TAURI_INTERNALS__.invoke;window.holdShow=true;window.__TAURI_INTERNALS__.invoke=(cmd,args)=>cmd==='list_items'&&window.holdShow?new Promise(resolve=>window.releaseShow=()=>{window.holdShow=false;resolve(original(cmd,args))}):original(cmd,args);window.emitEvent('main-shown')");
    await until(()=>evaluate("typeof window.releaseShow==='function'"),'唤出刷新未挂起');await click('[data-id="row-12"]');await evaluate('window.releaseShow()');await pause(90);assert.equal(await selected(),'row-12');
    await evaluate("window.holdShow=true;window.releaseShow=undefined;window.emitEvent('main-shown')");await until(()=>evaluate("typeof window.releaseShow==='function'"),'第二次唤出刷新未挂起');
    await key('ArrowDown',40);await pause(40);const keyboardChoice=await selected();assert.notEqual(keyboardChoice,'new-head');await evaluate('window.releaseShow()');await pause(80);assert.equal(await selected(),keyboardChoice);
    await appearance({select_first_on_show:false});await evaluate("window.emitEvent('main-shown')");await pause(70);assert.equal(await selected(),keyboardChoice);
    await evaluate("window.rows=[];window.emitEvent('history-changed')");await pause(60);await appearance({select_first_on_show:true});await evaluate("window.emitEvent('main-shown')");await pause(70);assert.equal(await selected(),undefined);
  });

  await test('R16 浮窗按内容报告高度、图片解码重测、长内容有上限且可缩回',async()=>{
    await floatingNav();const text=floatingContent('short');text.item={...text.item,kind:'text',thumbnail:null,preview:null};text.detail.text='一行短文本';
    await evaluate(`window.emitEvent('compact-preview-changed',${JSON.stringify(text)})`);await pause(100);
    const short=await evaluate("window.calls.filter(c=>c.cmd==='resize_compact_preview').at(-1).args");assert.equal(short.generation,42);assert(short.height>=120&&short.height<230,JSON.stringify(short));
    const before=await evaluate("window.calls.filter(c=>c.cmd==='resize_compact_preview').length");await pause(100);assert.equal(await evaluate("window.calls.filter(c=>c.cmd==='resize_compact_preview').length"),before);
    const long={...text,generation:43,detail:{...text.detail,text:'长内容\n'.repeat(500)}};await evaluate(`window.emitEvent('compact-preview-changed',${JSON.stringify(long)})`);await pause(100);assert(await evaluate("window.calls.filter(c=>c.cmd==='resize_compact_preview').at(-1).args.height===640"));
    // 模拟原生应用已按上限限制视口后的实际CSS渲染。
    await send('Emulation.setDeviceMetricsOverride',{width:360,height:640,deviceScaleFactor:1,mobile:false});await pause(50);assert(await evaluate("document.querySelector('.floating-content').scrollHeight>document.querySelector('.floating-content').clientHeight"));
    await evaluate(`window.emitEvent('compact-preview-changed',${JSON.stringify({...text,generation:44})})`);await pause(100);assert(await evaluate("window.calls.filter(c=>c.cmd==='resize_compact_preview').at(-1).args.height<230"));
    const image=floatingContent('adaptive-image');image.generation=45;await evaluate(`window.emitEvent('compact-preview-changed',${JSON.stringify(image)})`);await until(()=>evaluate("document.querySelector('.floating-preview img')?.naturalWidth===160"),'自适应图片未解码');await pause(90);
    const height=await evaluate("window.calls.filter(c=>c.cmd==='resize_compact_preview').at(-1).args.height");assert(height>200&&height<420,JSON.stringify(height));
    await send('Emulation.setDeviceMetricsOverride',{width:360,height,deviceScaleFactor:1,mobile:false});await pause(80);await capture('r16-adaptive-image');
    await evaluate("window.emitEvent('compact-preview-changed',null)");await pause(40);const cleared=await evaluate("window.calls.filter(c=>c.cmd==='resize_compact_preview').length");await pause(70);assert.equal(await evaluate("window.calls.filter(c=>c.cmd==='resize_compact_preview').length"),cleared);
  });
  await test('R17 预览只保留逐文件打开，单文件菜单打开和统一定位，失败可见',async()=>{
    await metrics();await nav();
    await evaluate("window.rows[1]={...window.rows[1],files:['C:/synthetic/甲 乙,丙.txt','C:/synthetic/第二张.png'],file_status:'available'};const original=window.__TAURI_INTERNALS__.invoke;window.__TAURI_INTERNALS__.invoke=(cmd,args)=>cmd==='item_detail'?Promise.resolve({text:'',files:window.rows[1].files.map(path=>({path,exists:true})),truncated:false}):original(cmd,args);window.emitEvent('history-changed')");await pause(70);await click('[data-id="row-1"]');await pause(50);
    assert.equal(await evaluate("document.querySelectorAll('.file-detail .file-actions').length"),2);
    await click('.file-detail li:nth-child(2) .file-actions button');await pause(40);assert.deepEqual(await evaluate("window.calls.filter(c=>c.cmd==='file_action').at(-1).args"),{id:'row-1',index:1,action:'open'});
    assert.equal(await evaluate("[...document.querySelectorAll('.file-detail .file-actions button')].filter(n=>n.textContent.includes('定位')).length"),0);assert.equal(await evaluate("document.querySelectorAll('.file-detail .file-actions button').length"),2);await capture('r17-files-detail');
    await evaluate("window.rows[1].files=['C:/synthetic/仅一个.txt'];window.emitEvent('history-changed')");await pause(60);
    const context=async id=>{await evaluate(`document.querySelector('[data-id="${id}"]').dispatchEvent(new MouseEvent('contextmenu',{bubbles:true,cancelable:true,clientX:280,clientY:250}))`);await pause(40);};
    await context('row-1');assert(await evaluate("document.querySelector('.context-menu').textContent.includes('用默认应用打开')"));
    await evaluate("[...document.querySelectorAll('.context-menu button')].find(n=>n.textContent==='在资源管理器中定位').click()");await pause(50);assert.equal(await evaluate("window.calls.filter(c=>c.cmd==='file_action').at(-1).args.action"),'reveal');
    await context('row-0');assert(!await evaluate("document.querySelector('.context-menu').textContent.includes('用默认应用打开')"));await key('Escape',27);
    await evaluate("const old=window.__TAURI_INTERNALS__.invoke;window.__TAURI_INTERNALS__.invoke=(cmd,args)=>cmd==='file_action'?Promise.reject('源文件已不存在'):old(cmd,args)");await context('row-1');await evaluate("[...document.querySelectorAll('.context-menu button')].find(n=>n.textContent==='用默认应用打开').click()");await pause(60);assert(await evaluate("document.querySelector('[role=alert]').textContent.includes('源文件已不存在')"));
  });

  for(const compact of [false,true])await test('R17 '+(compact?'紧凑':'普通')+' 多文件菜单仅一个定位入口，不显示打开或文件子菜单',async()=>{
    await metrics(compact?390:1000);await nav();await appearance({compact});await evaluate("window.rows[1].files=['C:/synthetic/第一.txt','C:/synthetic/第二.png'];window.rows[1].file_status='available';window.emitEvent('history-changed')");await pause(70);
    await evaluate("document.querySelector('[data-id=\"row-1\"]').dispatchEvent(new MouseEvent('contextmenu',{bubbles:true,cancelable:true,clientX:140,clientY:220}))");await pause(40);
    assert.equal(await evaluate("document.querySelector('.file-submenu')"),null);
    const labels=await evaluate("[...document.querySelectorAll('.context-menu button')].map(n=>n.textContent.trim())");
    assert.equal(labels.filter(s=>s.includes('定位')).length,1);assert(!labels.some(s=>s.includes('打开')));assert(!labels.some(s=>s.includes('第一.txt')||s.includes('第二.png')),JSON.stringify(labels));
    assert(await evaluate("(()=>{const r=document.querySelector('.context-menu').getBoundingClientRect();return r.left>=0&&r.right<=innerWidth&&r.top>=0&&r.bottom<=innerHeight})()"));
    await evaluate("[...document.querySelectorAll('.context-menu button')].find(n=>n.textContent==='在资源管理器中定位').focus()");await capture('r17-'+(compact?'compact':'list')+'-files-menu');await key('Enter',13);await pause(50);
    assert.deepEqual(await evaluate("window.calls.filter(c=>c.cmd==='file_action').map(c=>c.args)"),[{id:'row-1',index:0,action:'reveal'}]);assert.equal(await evaluate("window.calls.filter(c=>c.cmd==='paste_item').length"),0);
  });

  await test('R19 唤出位置仅保留记忆和鼠标、草稿取消与保存跨布局保留',async()=>{
    await metrics();await nav();assert.equal(await evaluate('window.settings.main_position'),'remember');
    await click('.settings-toggle');await pause(40);assert.deepEqual(await evaluate("[...document.querySelector('select[aria-label=主窗口唤出位置]').options].map(n=>n.value)"),['remember','mouse']);
    await change('select[aria-label=主窗口唤出位置]','mouse');await click('.settings-back');await pause(40);assert.equal(await evaluate('window.settings.main_position'),'remember');
    for(const mode of ['mouse','remember']){
      await click('.settings-toggle');await pause(40);await change('select[aria-label=主窗口唤出位置]',mode);await click('.settings-view button[type=submit]');await pause(50);assert.equal(await evaluate('window.settings.main_position'),mode);
      await click('.settings-back');await pause(30);await appearance({compact:true});await click('.settings-toggle');await pause(40);assert.equal(await evaluate("document.querySelector('select[aria-label=主窗口唤出位置]').value"),mode);await click('.settings-back');await appearance({compact:false});
    }
  });

  await test('R21 保留快捷键录制和保存，移除合并入口',async()=>{
    await metrics();await nav();await click('.settings-toggle');await pause(60);await click('.settings-nav button:nth-child(5)');
    const button=()=>evaluate("document.querySelector('[data-hotkey-record=main_hotkey]').click()");
    await button();await pause(40);assert.equal(await evaluate("window.calls.filter(c=>c.cmd==='set_hotkey_recording').at(-1).args.active"),true);
    await evaluate("window.emitEvent('hotkey-recorded',{session:1,hotkey:null})");await pause(50);assert.equal(await evaluate("document.querySelector('select[aria-label=主窗口快捷键]').value"),'alt_v');
    await button();await pause(30);await evaluate("window.emitEvent('hotkey-recorded',{session:1,hotkey:'custom:1:67'})");await pause(40);assert.equal(await evaluate("document.querySelector('select[aria-label=主窗口快捷键]').value"),'alt_v');assert(await evaluate("document.querySelector('[data-hotkey-record=main_hotkey]').title.includes('保留给系统')"));
    await button();await pause(30);await evaluate("window.emitEvent('hotkey-recorded',{session:0,hotkey:'custom:4:88'})");await pause(30);assert.equal(await evaluate("document.querySelector('select[aria-label=主窗口快捷键]').value"),'alt_v');await evaluate("window.emitEvent('hotkey-recorded',{session:1,hotkey:'custom:4:67'})");await pause(50);assert.equal(await evaluate("document.querySelector('select[aria-label=主窗口快捷键]').selectedOptions[0].textContent"),'Alt + C');
    assert.equal(await evaluate("document.querySelector('.settings-view').textContent.includes('合并主窗口')"),false);assert.equal(await evaluate("document.querySelector('select[aria-label=快速面板快捷键]').disabled"),false);
    await pause(350);assert.equal(await evaluate("!!document.querySelector('.hotkey-check')"),false);
    await click('.settings-view button[type=submit]');await pause(70);assert.equal(await evaluate('window.settings.main_hotkey'),'custom:4:67');assert.equal(await evaluate("'unified_hotkey' in window.settings"),false);await click('.settings-back');
    assert(await evaluate("document.querySelector('header .brand p').textContent.includes('Alt + C')"));
  });
  await test('R19 启动录制的迟到响应在退出设置后再次清理，不遗留录制',async()=>{
    await metrics();await nav();await click('.settings-toggle');await pause(60);await click('.settings-nav button:nth-child(5)');
    await evaluate("window.holdRecording=true;document.querySelector('[data-hotkey-record=main_hotkey]').click()");await pause(30);
    assert.equal(await evaluate("window.calls.filter(c=>c.cmd==='set_hotkey_recording'&&c.args.active).length"),1);
    await click('.settings-back');await pause(30);const stopped=await evaluate("window.calls.filter(c=>c.cmd==='set_hotkey_recording'&&!c.args.active).length");
    await evaluate('window.releaseRecording()');await pause(50);
    assert(await evaluate("window.calls.filter(c=>c.cmd==='set_hotkey_recording'&&!c.args.active).length")>stopped);
    assert.equal(await evaluate("window.calls.filter(c=>c.cmd==='set_hotkey_recording').at(-1).args.active"),false);
    await click('.settings-toggle');await pause(50);assert.equal(await evaluate("document.querySelector('select[aria-label=主窗口快捷键]').value"),'alt_v');
  });
  for(const compact of [false,true])await test('R20 '+(compact?'紧凑':'普通')+' 实际键盘事件录制与Esc只取消当前录制',async()=>{
    await metrics(compact?390:1000);await nav();await appearance({compact});await click('.settings-toggle');await pause(60);await click('.settings-nav button:nth-child(5)');
    const begin=()=>evaluate("document.querySelector('[data-hotkey-record=main_hotkey]').click()");
    for(const chord of [{key:'c',code:'KeyC',vk:67,mods:1,value:'custom:4:67'},{key:'C',code:'KeyC',vk:67,mods:10,value:'custom:3:67'}]){
      await begin();await pause(30);
      await send('Input.dispatchKeyEvent',{type:'keyDown',key:chord.key,code:chord.code,windowsVirtualKeyCode:chord.vk,modifiers:chord.mods});
      await send('Input.dispatchKeyEvent',{type:'keyUp',key:chord.key,code:chord.code,windowsVirtualKeyCode:chord.vk,modifiers:chord.mods});await pause(50);
      assert.equal(await evaluate("document.querySelector('select[aria-label=主窗口快捷键]').value"),chord.value);
      assert.equal(await evaluate('window.settings.main_hotkey'),'alt_v');assert.equal(await evaluate('window.settingUpdates.length'),0);
      await evaluate("window.emitEvent('hotkey-recorded',{session:1,hotkey:'custom:4:88'})");await pause(30);assert.equal(await evaluate("document.querySelector('select[aria-label=主窗口快捷键]').value"),chord.value);
    }
    await begin();await pause(30);await key('Escape',27);await pause(40);
    assert(await evaluate("!!document.querySelector('.settings-view')"));assert.equal(await evaluate("document.querySelector('select[aria-label=主窗口快捷键]').value"),'custom:3:67');
    assert.equal(await evaluate("window.calls.filter(c=>c.cmd==='set_hotkey_recording').at(-1).args.active"),false);
    assert.equal(await evaluate("window.calls.filter(c=>c.cmd==='paste_item').length"),0);
    await key('Escape',27);await pause(40);assert.equal(await evaluate("!!document.querySelector('.settings-view')"),false);
  });
  await test('R20 底部取消与行内取消均只停止录制，保留设置草稿',async()=>{
    await metrics();await nav();await click('.settings-toggle');await pause(60);await click('.settings-nav button:nth-child(5)');
    const begin=()=>evaluate("document.querySelector('[data-hotkey-record=main_hotkey]').click()");
    await change('select[aria-label=主窗口快捷键]','ctrl_alt_v');
    for(const selector of ["document.querySelector('.settings-cancel')","[...document.querySelectorAll('.settings-content button')].find(n=>n.textContent==='取消录制')"]){
      await begin();await pause(30);await evaluate(selector+'.click()');await pause(40);
      assert(await evaluate("!!document.querySelector('.settings-view')"));assert.equal(await evaluate("document.querySelector('select[aria-label=主窗口快捷键]').value"),'ctrl_alt_v');
    }
    await click('.settings-cancel');await pause(40);assert.equal(await evaluate("!!document.querySelector('.settings-view')"),false);assert.equal(await evaluate('window.settings.main_hotkey'),'alt_v');
  });
  await test('R20 启动响应在途时Esc先取消录制，迟到响应再次清理',async()=>{
    await metrics();await nav();await click('.settings-toggle');await pause(60);await click('.settings-nav button:nth-child(5)');
    await evaluate("window.holdRecording=true;document.querySelector('[data-hotkey-record=main_hotkey]').click()");await pause(30);
    await key('Escape',27);await pause(30);assert(await evaluate("!!document.querySelector('.settings-view')"));
    const stopped=await evaluate("window.calls.filter(c=>c.cmd==='set_hotkey_recording'&&!c.args.active).length");await evaluate('window.releaseRecording()');await pause(40);
    assert(await evaluate("window.calls.filter(c=>c.cmd==='set_hotkey_recording'&&!c.args.active).length")>stopped);assert.equal(await evaluate('window.settingUpdates.length'),0);
  });
  await test('R20 录制期间Enter和Space不激活按钮或退出设置',async()=>{
    await metrics();await nav();await click('.settings-toggle');await pause(60);await click('.settings-nav button:nth-child(5)');
    for(const chord of [{key:'Enter',code:'Enter',vk:13},{key:' ',code:'Space',vk:32}]){
      await evaluate("document.querySelector('[data-hotkey-record=main_hotkey]').click()");await pause(30);
      await evaluate("document.querySelector('.settings-cancel').focus()");
      await send('Input.dispatchKeyEvent',{type:'keyDown',key:chord.key,code:chord.code,windowsVirtualKeyCode:chord.vk});
      await send('Input.dispatchKeyEvent',{type:'keyUp',key:chord.key,code:chord.code,windowsVirtualKeyCode:chord.vk});await pause(40);
      assert(await evaluate("!!document.querySelector('.settings-view')"));assert.equal(await evaluate('window.settingUpdates.length'),0);assert(await evaluate("!!document.querySelector('[data-hotkey-record=main_hotkey][aria-invalid=true]')"));
    }
  });
  await test('R20 原生录制结果早于启动响应时按会话缓冲，不被迟到旧会话覆盖',async()=>{
    await metrics();await nav();await click('.settings-toggle');await pause(60);await click('.settings-nav button:nth-child(5)');
    await evaluate("window.holdRecording=true;document.querySelector('[data-hotkey-record=main_hotkey]').click()");await pause(30);
    await evaluate("window.emitEvent('hotkey-recorded',{session:1,hotkey:'custom:4:67'});window.emitEvent('hotkey-recorded',{session:0,hotkey:'custom:4:88'})");await pause(30);
    assert.equal(await evaluate("document.querySelector('select[aria-label=主窗口快捷键]').value"),'alt_v');
    await evaluate('window.releaseRecording()');await pause(50);
    assert.equal(await evaluate("document.querySelector('select[aria-label=主窗口快捷键]').value"),'custom:4:67');assert.equal(await evaluate("window.calls.filter(c=>c.cmd==='set_hotkey_recording').at(-1).args.active"),false);
    assert.equal(await evaluate('window.settingUpdates.length'),0);assert(await evaluate("!!document.querySelector('.settings-view')"));
  });
  await test('R22 更改组合后自动检测，已知冲突在对应行提示并阻止保存',async()=>{
    await metrics();await nav();await click('.settings-toggle');await pause(60);await click('.settings-nav button:nth-child(5)');
    await evaluate("window.hotkeyCheckResult={main:{status:'conflict',message:'此组合已被系统或其他程序注册'},quick:{status:'available',message:'未发现系统注册冲突'}}");
    await change('select[aria-label=主窗口快捷键]','ctrl_alt_v');await pause(400);
    assert.equal(await evaluate("document.querySelector('[data-hotkey-warning=main_hotkey]').textContent"),'检测到快捷键冲突');assert(await evaluate("document.querySelector('[data-hotkey-warning=main_hotkey]').title.includes('其他程序注册')"));assert.equal(await evaluate("!!document.querySelector('[data-hotkey-warning=quick_hotkey]')"),false);assert(await evaluate("document.querySelector('.settings-view button[type=submit]').disabled"));assert.equal(await evaluate('window.settingUpdates.length'),0);
    await evaluate('window.hotkeyCheckResult=undefined');await change('select[aria-label=主窗口快捷键]','win_alt_v');await pause(350);
    assert.equal(await evaluate("document.querySelector('.settings-view button[type=submit]').disabled"),false);await click('.settings-view button[type=submit]');await pause(50);
    assert.equal(await evaluate('window.settings.main_hotkey'),'win_alt_v');assert(await evaluate('window.hotkeyChecks.length>=3'));
  });
  await test('R21 检测失败可重试，保存时复检，失败不写设置',async()=>{
    await metrics();await nav();await click('.settings-toggle');await pause(60);await click('.settings-nav button:nth-child(5)');
    await change('select[aria-label=主窗口快捷键]','ctrl_alt_v');await pause(350);
    await evaluate('window.failHotkeyCheck=true');await click('.settings-view button[type=submit]');await pause(50);
    assert.equal(await evaluate('window.settingUpdates.length'),0);assert(await evaluate("[...document.querySelectorAll('[data-hotkey-warning]')].length===2&&[...document.querySelectorAll('[data-hotkey-warning]')].every(n=>n.textContent==='检测失败'&&n.title.includes('合成检测失败'))"));
    await evaluate('window.failHotkeyCheck=false');await click('.settings-view button[type=submit]');await pause(50);assert.equal(await evaluate('window.settings.main_hotkey'),'ctrl_alt_v');
  });
  await test('R21 检测迟到结果不能覆盖新快捷键的结果',async()=>{
    await metrics();await nav();await click('.settings-toggle');await pause(60);await click('.settings-nav button:nth-child(5)');
    await evaluate('window.holdHotkeyCheck=true');await change('select[aria-label=主窗口快捷键]','ctrl_alt_v');await pause(350);assert(await evaluate("typeof window.releaseHotkeyCheck==='function'"));
    await evaluate('window.holdHotkeyCheck=false');await change('select[aria-label=主窗口快捷键]','win_alt_v');await pause(350);
    await evaluate("window.releaseHotkeyCheck({main:{status:'conflict',message:'旧组合占用'},quick:{status:'available',message:'旧结果'}})");await pause(40);
    assert.equal(await evaluate("document.querySelectorAll('[data-hotkey-warning]').length"),0);assert.equal(await evaluate("document.querySelector('.settings-view button[type=submit]').disabled"),false);
  });
  await test('R21 相同修饰键与系统保留F12在本地拒绝，不提交检测',async()=>{
    await metrics();await nav();await click('.settings-toggle');await pause(60);await click('.settings-nav button:nth-child(5)');
    await evaluate("document.querySelector('[data-hotkey-record=main_hotkey]').click()");await pause(30);await evaluate("window.emitEvent('hotkey-recorded',{session:1,hotkey:'custom:4:123'})");await pause(40);
    assert(await evaluate("document.querySelector('[data-hotkey-record=main_hotkey]').title.includes('保留给系统')"));assert.equal(await evaluate("document.querySelector('select[aria-label=主窗口快捷键]').value"),'alt_v');
    await evaluate("document.querySelector('[data-hotkey-record=quick_hotkey]').click()");await pause(30);await evaluate("window.emitEvent('hotkey-recorded',{session:1,hotkey:'custom:4:67'})");await pause(40);
    const count=await evaluate('window.hotkeyChecks.length');await pause(350);
    assert.equal(await evaluate('window.hotkeyChecks.length'),count);assert(await evaluate("document.querySelectorAll('[data-hotkey-warning]').length===2&&[...document.querySelectorAll('[data-hotkey-warning]')].every(n=>n.textContent==='检测到快捷键冲突'&&n.title.includes('相同的修饰键'))"));assert(await evaluate("document.querySelector('.settings-view button[type=submit]').disabled"));
  });
  await test('R22 打开设置检测一次，改键再次检测，保存其他设置及取消录制不重复检测',async()=>{
    await metrics();await nav();await click('.settings-toggle');await pause(400);assert.equal(await evaluate('window.hotkeyChecks.length'),1);
    await click('.settings-nav button:nth-child(5)');await pause(350);assert.equal(await evaluate('window.hotkeyChecks.length'),1);
    assert.equal(await evaluate("!!document.querySelector('.hotkey-check')||document.querySelector('.settings-view').textContent.includes('检测冲突')||document.querySelector('.settings-view').textContent.includes('未发现系统注册冲突')||document.querySelector('.settings-view').textContent.includes('仅检查系统登记')"),false);
    await evaluate("const n=document.querySelector('[data-category=hotkeys] input[type=number]');n.value='6';n.dispatchEvent(new Event('input',{bubbles:true}))");await click('.settings-view button[type=submit]');await pause(350);assert.equal(await evaluate('window.hotkeyChecks.length'),1);
    await click('.settings-back');await click('.settings-toggle');await pause(350);assert.equal(await evaluate('window.hotkeyChecks.length'),2);await click('.settings-nav button:nth-child(5)');
    await evaluate("document.querySelector('[data-hotkey-record=main_hotkey]').click()");await pause(30);await evaluate("window.emitEvent('hotkey-recorded',{session:1,hotkey:'custom:4:67'})");await pause(350);
    assert.equal(await evaluate('window.hotkeyChecks.length'),3);assert.equal(await evaluate('window.hotkeyChecks.at(-1).mainHotkey'),'custom:4:67');
    await evaluate("document.querySelector('[data-hotkey-record=main_hotkey]').click()");await pause(30);await key('Escape',27);await pause(350);assert.equal(await evaluate('window.hotkeyChecks.length'),3);
    await evaluate("document.querySelector('[data-hotkey-record=main_hotkey]').click()");await pause(30);await evaluate("window.emitEvent('hotkey-recorded',{session:1,hotkey:'custom:4:67'})");await pause(350);assert.equal(await evaluate('window.hotkeyChecks.length'),4);
  });
  await test('R23 两主题普通与紧凑下，冲突直接显示在红色录制按钮中且无溢出',async()=>{
    for(const theme of ['light','dark'])for(const compact of [false,true]){
      await metrics(compact?390:1000);await nav();await appearance({theme,compact});
      await evaluate("window.hotkeyCheckResult={main:{status:'owned',message:'当前由本应用注册'},quick:{status:'conflict',message:'此组合已被系统或其他程序注册'}}");await click('.settings-toggle');await pause(60);
      if(compact)await change('select[aria-label=设置分类]','hotkeys');else await click('.settings-nav button:nth-child(5)');
      await until(()=>evaluate("!!document.querySelector('[data-hotkey-warning=quick_hotkey]')"),'冲突行提示缺失');
      assert.equal(await evaluate("!!document.querySelector('[data-hotkey-warning=main_hotkey]')"),false);
      assert(await evaluate("(()=>{const n=document.querySelector('[data-hotkey-warning=quick_hotkey]'),r=n.getBoundingClientRect(),c=getComputedStyle(n).color.match(/\\d+/g).map(Number),bg=getComputedStyle(n).backgroundColor.match(/\\d+/g).map(Number);return n.tagName==='BUTTON'&&n.classList.contains('hotkey-problem')&&n.textContent==='检测到快捷键冲突'&&n.scrollWidth<=n.clientWidth+1&&r.right<=innerWidth&&c[0]>c[1]&&c[0]>c[2]&&bg[0]>bg[1]&&bg[0]>bg[2]&&document.documentElement.scrollWidth<=innerWidth})()"));
      if(!compact&&theme==='light')await capture('r23-conflict-button');
      if(compact&&theme==='dark')await capture('r23-compact-dark-conflict-button');
    }
  });
  await test('R23 非法录制错误在恢复预设、重选原组合和成功保存后清除',async()=>{
    await metrics();await nav();await click('.settings-toggle');await pause(60);await click('.settings-nav button:nth-child(5)');
    const invalid=async(field)=>{await evaluate("(()=>{const b=document.querySelector('[data-hotkey-record="+field+"]');b.focus();b.click()})()");await pause(30);await evaluate("window.emitEvent('hotkey-recorded',{session:1,hotkey:'custom:1:67'})");await pause(40);assert.equal(await evaluate("document.querySelector('[data-hotkey-record="+field+"]').textContent"),'检测到快捷键冲突');};
    await invalid('main_hotkey');assert.equal(await evaluate("!!document.querySelector('[data-category=hotkeys] .error')"),false);assert.equal(await evaluate("document.querySelector('select[aria-label=主窗口快捷键]').value"),'alt_v');
    await change('select[aria-label=主窗口快捷键]','ctrl_alt_v');await pause(350);assert.equal(await evaluate("document.querySelector('[data-hotkey-record=main_hotkey]').textContent"),'录制主窗口快捷键');
    await invalid('main_hotkey');await change('select[aria-label=主窗口快捷键]','ctrl_alt_v');await pause(350);assert.equal(await evaluate("document.querySelectorAll('.hotkey-problem').length"),0,'重选相同预设应清除录制错误');
    await invalid('main_hotkey');await send('Emulation.setFocusEmulationEnabled',{enabled:true});
    try {await evaluate("document.querySelector('select[aria-label=主窗口快捷键]').focus()");await pause(30);assert.equal(await evaluate("document.querySelectorAll('.hotkey-problem').length"),0,'聚焦当前预设应清除旧录制错误');}
    finally {await send('Emulation.setFocusEmulationEnabled',{enabled:false});}
    await invalid('quick_hotkey');assert.equal(await evaluate("document.querySelector('[data-hotkey-record=main_hotkey]').textContent"),'录制主窗口快捷键');await click('.settings-view button[type=submit]');await pause(80);
    assert.equal(await evaluate('window.settings.main_hotkey'),'ctrl_alt_v');assert.equal(await evaluate("document.querySelectorAll('.hotkey-problem').length"),0,'成功保存应清除各行录制错误');assert.equal(await evaluate("!!document.querySelector('[data-category=hotkeys] .error')"),false);
  });
  await test('R23 当前占用组合改回原组合后按钮恢复，冲突按钮仍可直接录制',async()=>{
    await metrics();await nav();await click('.settings-toggle');await pause(60);await click('.settings-nav button:nth-child(5)');
    await evaluate("window.hotkeyCheckResult={main:{status:'conflict',message:'合成系统占用'},quick:{status:'available',message:'可注册'}}");await change('select[aria-label=主窗口快捷键]','ctrl_alt_v');await pause(350);
    assert.equal(await evaluate("document.querySelector('[data-hotkey-record=main_hotkey]').textContent"),'检测到快捷键冲突');await click('[data-hotkey-record=main_hotkey]');await pause(30);assert.equal(await evaluate("document.querySelector('[data-hotkey-record=main_hotkey]').textContent"),'取消录制');await key('Escape',27);await pause(30);
    await evaluate('window.hotkeyCheckResult=undefined');await change('select[aria-label=主窗口快捷键]','alt_v');await pause(350);
    assert.equal(await evaluate("document.querySelector('[data-hotkey-record=main_hotkey]').textContent"),'录制主窗口快捷键');assert.equal(await evaluate("document.querySelectorAll('.hotkey-problem').length"),0);assert.equal(await evaluate("document.querySelector('.settings-view button[type=submit]').disabled"),false);
  });
  await test('R23 录制启动或停止失败归属本行，不误报冲突或写入迟到结果',async()=>{
    await metrics();await nav();await click('.settings-toggle');await pause(60);await click('.settings-nav button:nth-child(5)');
    await evaluate("window.recordInvoke=window.__TAURI_INTERNALS__.invoke;window.failStart=true;window.failStop=false;window.__TAURI_INTERNALS__.invoke=(cmd,args)=>cmd==='set_hotkey_recording'&&((args.active&&window.failStart)||(!args.active&&window.failStop))?Promise.reject('合成录制通信失败'):window.recordInvoke(cmd,args)");
    await click('[data-hotkey-record=main_hotkey]');await pause(40);assert.equal(await evaluate("document.querySelector('[data-hotkey-record=main_hotkey]').textContent"),'录制失败');assert(await evaluate("document.querySelector('[data-hotkey-record=main_hotkey]').title.includes('合成录制通信失败')"));
    await evaluate('window.failStart=false;window.failStop=true');await click('[data-hotkey-record=main_hotkey]');await pause(30);await evaluate("window.emitEvent('hotkey-recorded',{session:1,hotkey:'custom:4:67'})");await pause(40);
    assert.equal(await evaluate("document.querySelector('[data-hotkey-record=main_hotkey]').textContent"),'录制失败');assert.equal(await evaluate("document.querySelector('select[aria-label=主窗口快捷键]').value"),'alt_v');
    await evaluate('window.failStop=false');await click('[data-hotkey-record=main_hotkey]');await pause(30);await evaluate("window.emitEvent('hotkey-recorded',{session:1,hotkey:'custom:4:67'})");await pause(50);
    assert.equal(await evaluate("document.querySelector('select[aria-label=主窗口快捷键]').value"),'custom:4:67');assert.equal(await evaluate("document.querySelectorAll('.hotkey-problem').length"),0);assert.equal(await evaluate("!!document.querySelector('[data-category=hotkeys] .error')"),false);
  });
  await test('R23 停止录制在途时改回预设，迟到结果不写入且自动检测不遗漏',async()=>{
    await metrics();await nav();await click('.settings-toggle');await pause(60);await click('.settings-nav button:nth-child(5)');
    await evaluate("window.recordInvoke=window.__TAURI_INTERNALS__.invoke;window.holdStop=false;window.__TAURI_INTERNALS__.invoke=(cmd,args)=>cmd==='set_hotkey_recording'&&!args.active&&window.holdStop?new Promise(resolve=>window.releaseStop=resolve):window.recordInvoke(cmd,args)");
    for(const row of [{payload:'custom:1:67',preset:'ctrl_alt_v'},{payload:'custom:4:67',preset:'win_alt_v'}]){
      const before=await evaluate('window.hotkeyChecks.length');await evaluate('window.holdStop=true');await click('[data-hotkey-record=main_hotkey]');await pause(30);
      await evaluate("void window.emitEvent('hotkey-recorded',{session:1,hotkey:"+JSON.stringify(row.payload)+"})");await until(()=>evaluate("typeof window.releaseStop==='function'"),'停止请求未开始');
      await change('select[aria-label=主窗口快捷键]',row.preset);await pause(30);assert.equal(await evaluate('window.hotkeyChecks.length'),before);
      await evaluate('window.holdStop=false;window.releaseStop(1);window.releaseStop=undefined');await pause(350);
      assert.equal(await evaluate("document.querySelector('select[aria-label=主窗口快捷键]').value"),row.preset);assert.equal(await evaluate('window.hotkeyChecks.length'),before+1);assert.equal(await evaluate('window.hotkeyChecks.at(-1).mainHotkey'),row.preset);assert.equal(await evaluate("document.querySelectorAll('.hotkey-problem').length"),0);
    }
  });
  await writeFile(path.join(out,'results.json'),JSON.stringify({passed:checks.length,failed:failures.length,checks,failures,scope:'Edge 无头实际渲染，合成 IPC；原生窗口/焦点/粘贴未测'},null,2));
  console.log(JSON.stringify({passed:checks.length,failed:failures.length,failures},null,2));
  await send('Browser.close').catch(()=>{});
  if(failures.length)process.exitCode=1;
}finally{ws?.close();browser.kill();await new Promise(resolve=>server.close(resolve));}

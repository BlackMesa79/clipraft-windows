// 离屏渲染 + 合成 IPC 数据；不操作用户桌面，不代表原生粘贴验收。
import { createServer } from 'node:http';
import { spawn } from 'node:child_process';
import { readFile, writeFile, mkdir } from 'node:fs/promises';
import path from 'node:path';
import assert from 'node:assert/strict';
import { settingsMock } from './settings-ui-mock.mjs';
const root = process.cwd();
const out = path.join(root, '.test-data/m1-t5-ui');
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
  const chosen=()=>evaluate("Array.from(document.querySelectorAll('.item.selected .summary')).map(n=>n.textContent)");
  const settled=()=>until(()=>evaluate("document.querySelector('.list')?.getAttribute('aria-busy')==='false' || (!!document.querySelector('.empty') && !document.querySelector('.empty').textContent.includes('正在搜索'))"),'刷新未完成');
  const input=async value=>{await evaluate(`(()=>{const el=document.querySelector('input[type=search]');el.focus();el.value=${JSON.stringify(value)};el.dispatchEvent(new Event('input',{bubbles:true}));})()`);};
  const press=(key,extras={},target="document.activeElement")=>evaluate(`${target}.dispatchEvent(new KeyboardEvent('keydown',{key:${JSON.stringify(key)},bubbles:true,cancelable:true,...${JSON.stringify(extras)}}))`);
  const click=(i,extras={})=>evaluate(`document.querySelectorAll('.item')[${i}].dispatchEvent(new MouseEvent('click',{bubbles:true,...${JSON.stringify(extras)}}))`);
  const scroll=()=>evaluate("(()=>{const el=document.querySelector('.list');el.scrollTop=el.scrollHeight;el.dispatchEvent(new Event('scroll'));})()");
  const fresh=async()=>{await send('Page.navigate',{url:origin});await ready();await count(100);};
  const button = (label, selector='button')=>evaluate(`(()=>{const el=Array.from(document.querySelectorAll(${JSON.stringify(selector)})).find(b=>b.textContent.trim()===${JSON.stringify(label)});if(!el)throw Error('按钮未找到 '+${JSON.stringify(label)});el.click();})()`);
  const groupButton = label=>button(label,'.group-views>button');
  const context = async i=>{await evaluate(`document.querySelectorAll('.item')[${i}].dispatchEvent(new MouseEvent('contextmenu',{bubbles:true,clientX:980,clientY:680}))`);await pause(30);};
  const groupContext = async label=>{await evaluate(`Array.from(document.querySelectorAll('.group-views>button')).find(b=>b.textContent.trim().startsWith(${JSON.stringify(label)})).dispatchEvent(new MouseEvent('contextmenu',{bubbles:true,clientX:300,clientY:190}))`);await pause(30);};
  const renameInput = async (label,value)=>evaluate(`(()=>{const el=document.querySelector('input[aria-label=${JSON.stringify(label)}]');el.focus();el.value=${JSON.stringify(value)};el.dispatchEvent(new Event('input',{bubbles:true}));})()`);
  const mutated = async n=>{await until(()=>evaluate(`window.mutations.length===${n}`),'修改未发出 '+n);await settled();await pause(40);};
  const noErrors=async()=>assert.deepEqual(await evaluate("Array.from(document.querySelectorAll('.error')).map(n=>n.textContent)"),[]);
  // CDP 按键会执行浏览器默认行为，覆盖合成 keydown 无法验证的 Tab 焦点移动。
  const nativeKey = async (key, code, windowsVirtualKeyCode) => {
    await send('Input.dispatchKeyEvent', { type: 'keyDown', key, code, windowsVirtualKeyCode });
    await send('Input.dispatchKeyEvent', { type: 'keyUp', key, code, windowsVirtualKeyCode });
  };
  for (const menuType of ['record', 'group']) {
    for (const source of ['input[type=search]', '.item']) {
      await fresh();
      await evaluate(`document.querySelector(${JSON.stringify(source)}).focus()`);
      if (menuType === 'record') await context(0); else await groupContext('工作');
      assert(await evaluate("document.activeElement===document.querySelector('.context-menu [role=menuitem]')"));
      await nativeKey('Tab', 'Tab', 9);
      assert(await evaluate("document.activeElement===document.querySelector('.context-menu [role=menuitem]')"));
      // 即使其他代码把焦点移出菜单，Enter 也不能落到全局粘贴处理。
      await evaluate("document.querySelector('input[type=search]').focus()");
      await nativeKey('Enter', 'Enter', 13);
      assert.equal(await evaluate('window.pastes.length'),0);
      assert.equal(await evaluate('window.mutations.length'),0);
      assert(await evaluate("!!document.querySelector('.context-menu')"));
      // 再回到菜单验证 Esc 关闭后恢复打开之前的元素。
      await nativeKey('ArrowDown', 'ArrowDown', 40);
      await nativeKey('Escape', 'Escape', 27);
      assert.equal(await evaluate("document.querySelector('.context-menu')"),null);
      assert(await evaluate(`document.activeElement===document.querySelector(${JSON.stringify(source)})`));
      assert.equal(await evaluate('window.hidden'),0);
    }
  }
  for (const menuType of ['record', 'group']) {
    await fresh();
    if (menuType === 'record') await context(0); else await groupContext('工作');
    await nativeKey('Tab', 'Tab', 9);
    await nativeKey('Enter', 'Enter', 13);
    assert.equal(await evaluate('window.pastes.length'),0);
    if (menuType === 'record') {
      await mutated(1);
      assert.equal(await evaluate('window.mutations[0].cmd'),'set_pinned');
    } else {
      assert(await evaluate("document.activeElement===document.querySelector('input[aria-label=重命名分组]')"));
      await press('Escape');
    }
    // Esc 也必须能从意外移出的焦点恢复到原列表项。
    await evaluate("document.querySelector('.item').focus()");
    if (menuType === 'record') await context(0); else await groupContext('工作');
    await evaluate("document.querySelector('input[type=search]').focus()");
    await nativeKey('Escape', 'Escape', 27);
    assert(await evaluate("document.activeElement===document.querySelector('.item')"));
  }
  checks.push('分组/记录菜单首项聚焦，真实 Tab 不逃逸，外部焦点 Enter 不粘贴，Esc 恢复搜索框/列表项焦点');
  await fresh();await context(0);
  await press('ArrowUp');assert.equal(await evaluate('document.activeElement.textContent'),'粘贴');
  await press('ArrowDown');assert.equal(await evaluate('document.activeElement.textContent'),'置顶');
  for (const key of ['p','d','a']) assert.equal(await press(key,{ctrlKey:true}),false);
  await press('Delete');assert.equal(await evaluate('window.deletes.length'),0);
  assert.equal(await evaluate('window.mutations.length'),0);
  assert.equal((await chosen()).length,1);
  await press('ArrowDown');await press('ArrowDown');await press('ArrowRight');
  assert.equal(await evaluate('document.activeElement.textContent'),'移出分组');
  await press('ArrowLeft');assert.equal(await evaluate('document.activeElement.textContent'),'移到分组 ▸');
  assert.equal(await evaluate("document.querySelector('.group-submenu')"),null);
  await press('Enter');assert.equal(await evaluate('document.activeElement.textContent'),'移出分组');
  await press('ArrowDown');await press('ArrowDown');await press(' ');await mutated(1);
  assert.deepEqual(await evaluate('window.mutations[0]'),{cmd:'move_to_group',args:{ids:['item-0'],groupId:2}});
  await context(0);await evaluate("document.body.dispatchEvent(new PointerEvent('pointerdown',{bubbles:true}))");
  assert.equal(await evaluate("document.querySelector('.context-menu')"),null);
  await groupContext('工作');await press('Enter');
  assert(await evaluate("document.activeElement===document.querySelector('input[aria-label=重命名分组]')"));
  await press('Escape');
  await groupContext('工作');await evaluate("document.body.dispatchEvent(new PointerEvent('pointerdown',{bubbles:true}))");
  assert.equal(await evaluate("document.querySelector('.context-menu')"),null);
  checks.push('菜单上下循环、Enter/Space 激活、子菜单右进左退、快捷键隔离和外部点击关闭');
  await fresh();await button('⚙ 设置');
  for (const target of ['document.body', "document.querySelector('.settings-view input, .settings-view select, .settings-view button')"]) {
    for (const key of ['p','d']) {
      assert.equal(await press(key,{ctrlKey:true},target),false);
      assert.equal(await press(key,{ctrlKey:true,repeat:true},target),false);
      assert.equal(await press(key,{ctrlKey:true,isComposing:true},target),true);
      assert.equal(await press(key,{ctrlKey:true,keyCode:229},target),true);
    }
  }
  assert.equal(await evaluate('window.mutations.length'),0);
  assert.equal(await evaluate('window.pastes.length'),0);
  checks.push('设置页 Ctrl+P/D 阻止原生行为且不修改记录；IME 和 229 保持默认行为');
  await fresh();await noErrors();
  assert(await evaluate("document.querySelector('.group-views').textContent.includes('工作 120')"));
  assert(await evaluate("document.querySelector('.item-badges').textContent.includes('工作')"));
  await groupButton('收藏');await count(100);await settled();
  assert.equal(await evaluate('window.calls.at(-1).filter.favorite_only'),true);
  await scroll();await count(200);await scroll();await count(220);
  await input('Hello');await count(100);await settled();await button('文本','.kind-filters button');await settled();
  assert.deepEqual(await evaluate('window.calls.at(-1).filter'),{query:'Hello',kind:'text',favorite_only:true});
  assert.deepEqual(await chosen(),['你好 Hello nihao nh 0']);
  checks.push('收藏视图、搜索和类型组合，220 条分页，切换回第一页首条');
  await groupButton('工作 120');await count(100);await settled();await scroll();await count(120);
  assert.equal(await evaluate('window.calls.at(-1).filter.group_id'),1);
  await groupButton('未分组');await count(100);await settled();
  assert.equal(await evaluate('window.calls.at(-1).filter.group_id'),-1);assert.deepEqual(await chosen(),['你好 Hello nihao nh 120']);
  await input('');await count(100);await settled();await scroll();await count(130);
  checks.push('分组/未分组和搜索组合，独立分页与计数');
  await fresh();await click(2);await click(4,{ctrlKey:true});await context(4);
  await button('置顶','.context-menu button');await mutated(1);
  assert.deepEqual(await evaluate('window.mutations[0]'),{cmd:'set_pinned',args:{ids:['item-2','item-4'],value:true}});
  assert.deepEqual(await evaluate("Array.from(document.querySelectorAll('.item .summary')).slice(0,2).map(n=>n.textContent)"),['你好 Hello nihao nh 2','你好 Hello nihao nh 4']);
  assert.equal((await chosen()).length,2);
  await context(0);await button('取消置顶','.context-menu button');await mutated(2);
  assert.equal(await evaluate("document.querySelector('.item .summary').textContent"),'你好 Hello nihao nh 0');
  checks.push('右键置顶/取消置顶作用于多选，置顶排序并保留多选');
  await context(2);await button('取消收藏','.context-menu button');await mutated(3);
  assert.deepEqual(await evaluate('window.mutations.at(-1).args'),{ids:['item-2','item-4'],value:false});
  await context(2);await button('收藏','.context-menu button');await mutated(4);
  checks.push('右键批量收藏与取消，状态标记更新');
  await context(2);await button('移到分组 ▸','.context-menu button');await pause(50);
  assert(await evaluate("(()=>{const r=document.querySelector('.context-menu').getBoundingClientRect();return r.bottom<=innerHeight&&r.right<=innerWidth;})()"));
  await button('私人','.group-submenu button');await mutated(5);
  assert.deepEqual(await evaluate('window.mutations.at(-1).args'),{ids:['item-2','item-4'],groupId:2});
  assert(await evaluate("document.querySelector('.group-views').textContent.includes('私人 2')"));
  await groupButton('私人 2');await count(2);await settled();await click(1,{ctrlKey:true});await context(0);await button('移到分组 ▸','.context-menu button');await button('移出分组','.group-submenu button');await mutated(6);await count(0);
  assert.equal(await evaluate('window.rows.length'),250);assert.equal(await evaluate('window.mutations.at(-1).args.groupId'),null);
  checks.push('分组子菜单不溢出；批量移入/移出后计数、标签与当前视图同步');
  await fresh();
  assert.equal(await press('p',{ctrlKey:true}),false);await mutated(1);assert.equal(await evaluate('window.rows[0].pinned'),true);
  assert(await evaluate("document.activeElement===document.querySelector('input[type=search]')"));
  assert.equal(await press('p',{ctrlKey:true}),false);await mutated(2);assert.equal(await evaluate('window.rows[0].pinned'),false);
  assert.equal(await press('d',{ctrlKey:true}),false);await mutated(3);assert.equal(await evaluate('window.rows[0].favorite'),false);
  assert.equal(await press('d',{ctrlKey:true,repeat:true}),false);assert.equal(await evaluate('window.mutations.length'),3);
  assert.equal(await press('p',{ctrlKey:true,isComposing:true}),true);assert.equal(await press('d',{ctrlKey:true,keyCode:229}),true);
  await evaluate("document.querySelector('input[type=search]').dispatchEvent(new CompositionEvent('compositionstart',{bubbles:true}))");
  assert.equal(await press('p',{ctrlKey:true}),true);assert.equal(await evaluate('window.mutations.length'),3);
  await evaluate("document.querySelector('input[type=search]').dispatchEvent(new CompositionEvent('compositionend',{bubbles:true}))");await pause(200);await settled();
  checks.push('Ctrl+P/D 切换并 preventDefault；重复键、IME 组合与 229 不触发修改');
  await groupButton('+ 新建分组');await renameInput('新分组名称','  Inbox  ');
  assert.equal(await press('p',{ctrlKey:true}),true);assert.equal(await press('d',{ctrlKey:true}),true);assert.equal(await evaluate('window.mutations.length'),3);
  await press('Delete');assert.equal(await evaluate('window.deletes.length'),0);
  await button('创建','.group-editor button');await mutated(4);
  assert(await evaluate("document.querySelector('.group-views').textContent.includes('Inbox 0')"));
  await groupContext('Inbox');await button('重命名','.context-menu button');await renameInput('重命名分组','新名称');
  await button('保存','.group-editor button');await mutated(5);assert(await evaluate("document.querySelector('.group-views').textContent.includes('新名称 0')"));
  await groupContext('新名称');await button('重命名','.context-menu button');await renameInput('重命名分组','工作');await button('保存','.group-editor button');await pause(80);
  assert(await evaluate("document.querySelector('.group-sidebar .error').textContent.includes('同名')"));
  await press('Escape');assert.equal(await evaluate("document.querySelector('.group-editor')"),null);
  checks.push('分组新建去空白、就地改名与错误保留；名称输入框不响应列表快捷键和删除');
  await groupContext('工作');await press('Escape',{},'document.body');assert.equal(await evaluate("document.querySelector('.context-menu')"),null);assert.equal(await evaluate('window.hidden'),0);
  await groupButton('工作 120');await count(100);await settled();await groupContext('工作');await button('删除分组','.context-menu button');
  assert(await evaluate("document.querySelector('.group-sidebar .delete-confirm').textContent.includes('组内 120 条记录会回到未分组，不会删除记录')"));
  await button('确认删除分组','.group-sidebar .delete-confirm button');await mutated(7);await count(100);await settled();
  assert.equal(await evaluate('window.rows.length'),250);assert.equal(await evaluate('window.rows.filter(i=>i.group_id===1).length'),0);
  assert.equal(await evaluate('window.calls.at(-1).filter.group_id'),-1);
  checks.push('分组菜单 Esc 只关闭菜单；删除确认不删除记录，当前分组转未分组');
  await fresh();await evaluate('window.rows[0].pinned=true;window.rows[1].favorite=true;window.rows[2].favorite=false;window.changed()');await settled();
  await click(0);await click(1,{ctrlKey:true});await click(2,{ctrlKey:true});await context(0);await button('删除','.context-menu button');
  assert(await evaluate("document.querySelector('.delete-confirm').textContent.includes('将删除 3 条，其中 2 条是置顶或收藏记录')"));
  const shot=await send('Page.captureScreenshot',{format:'jpeg',quality:65});await writeFile(path.join(out,'confirmation.jpg'),Buffer.from(shot.data,'base64'));
  await button('确认','.delete-confirm button');await settled();assert.equal(await evaluate('window.rows.length'),247);
  checks.push('批量删除确认计数按置顶或收藏的并集，不阻止手动删除');
  await fresh();await evaluate('window.failMutation=true');await press('p',{ctrlKey:true});await mutated(1);
  assert.equal(await evaluate('window.rows[0].pinned'),false);assert(await evaluate("document.querySelector('.error').textContent.includes('合成修改失败')"));
  await evaluate('window.holdMutation=true');await press('d',{ctrlKey:true});await until(()=>evaluate('!!window.releaseMutation'),'未挂起修改');
  await press('p',{ctrlKey:true});assert.equal(await evaluate('window.mutations.length'),2);
  await evaluate('window.releaseMutation()');await settled();await pause(80);await noErrors();
  checks.push('失败保留真实状态，进行中的修改阻止重复操作');
  await send('Emulation.setDeviceMetricsOverride',{width:680,height:620,deviceScaleFactor:1,mobile:false});
  assert(await evaluate('document.documentElement.scrollWidth<=680'));checks.push('680px 窄窗口实际渲染无页面横向溢出');
  await send('Page.navigate',{url:origin+'/?panel=quick'});await ready();await count(200);
  assert.equal(await evaluate("document.querySelector('.group-sidebar')"),null);assert.equal(await evaluate('window.calls.length'),0);
  await press('p',{ctrlKey:true});await press('d',{ctrlKey:true});assert.equal(await evaluate('window.mutations.length'),0);
  checks.push('快速面板仍只使用全部历史缓存，不显示分组栏，不响应修改快捷键');

  await writeFile(path.join(out,'results.json'),JSON.stringify({passed:checks.length,checks},null,2));console.log(JSON.stringify({passed:checks.length,checks},null,2));
} finally {
  ws?.close();browser.kill();await new Promise(resolve=>server.close(resolve));
}

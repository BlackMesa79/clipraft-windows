import { createAppearanceRecorder } from './appearance-baseline.mjs';
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
      const timer = setTimeout(() => {
        pending.delete(id);
        reject(Error(method + ' 等待超过15秒'));
      }, 15_000);
      pending.set(id, {
        resolve: (value) => {
          clearTimeout(timer);
          resolve(value);
        },
        reject: (error) => {
          clearTimeout(timer);
          reject(error);
        },
      });
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
    window.calls=[];window.ready=false;window.failResize=false;
    window.rows=[
      {id:'safe',kind:'text',summary:'合成普通文本',formats:['CF_UNICODETEXT'],files:[]},
      {id:'secret',kind:'text',summary:'sk-S••••••••ef',sensitive:true,formats:['CF_UNICODETEXT'],files:[],thumbnail:null},
      {id:'file',kind:'files',summary:'文件',formats:['CF_HDROP'],files:['C:/synthetic/report.txt'],file_status:'missing'}
    ];
    window.rows=Array.from({length:60},(_,i)=>({id:'row-'+i,kind:i===1?'files':i===2?'image':'text',summary:i===3?'合成敏感••••':'合成记录 '+i+' 用于外观和布局验收',sensitive:i===3,favorite:i%2===0,group_id:i%3===0?7:null,formats:['CF_UNICODETEXT'],files:i===1?['C:/synthetic/report.txt']:[],file_status:i===1?'missing':undefined,source_exe:'synthetic.exe',thumbnail:null}));
    if(location.search.includes('fluent-fixture')){
      window.rows.forEach((item,index)=>item.created=index===59?Number.MAX_SAFE_INTEGER:Date.UTC(2026,10,18,6,36-index));
      window.rows[2].thumbnail='synthetic-thumb';
    }
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
        if(cmd==='panel_state'){if(location.search.includes('fail=panel'))throw Error('合成 panel_state 失败');return {items:window.rows,selected:1,visible:true,error:null,serial:0,session:1,position:1,total:window.rows.length,loading:false};}
        if(cmd==='frontend_ready'){window.ready=true;return;}
        if(cmd==='set_hotkey_recording'){if(args.active&&window.holdRecording)await new Promise(resolve=>window.releaseRecording=resolve);return 1;}
        if(cmd==='selected_items')return args.ids.flatMap(id=>window.rows.filter(row=>row.id===id));
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
  await send('Page.enable');
  await send('Runtime.enable');
  await send('Page.addScriptToEvaluateOnNewDocument', {
    source:
      mock +
      settingsMock +
      `
    const query = new URLSearchParams(location.search);
    if (query.has('auto-update')) window.settings.auto_check_updates = true;
    if (query.has('system-theme')) Object.assign(window.settings, {skin:'fluent',theme:'system'});
    if (query.has('compact-about')) Object.assign(window.settings, {skin:'fluent',theme:'dark',compact:true});
    window.releaseChecks = [];
    window.releasePageChecks = 0;
    window.releaseNotesTags = [];
    const releaseBody = ['前言', '## 更新内容', '- 合成发布说明 <b>纯文本</b>',
      '### 细节', '保留子节', '## 下载与运行', '下载命令'].join(String.fromCharCode(10));
    window.releaseReply = {status:200, body:{tag_name:'v0.8.10',draft:false,prerelease:false,body:releaseBody}};
    const networkFetch = window.fetch.bind(window);
    window.fetch = async (url, init) => {
      if (url !== 'https://api.github.com/repos/BlackMesa79/clipraft-windows/releases/latest') return networkFetch(url, init);
      window.releaseChecks.push({credentials:init.credentials,redirect:init.redirect,referrerPolicy:init.referrerPolicy});
      if (window.releaseReply.offline) throw Error('合成断网');
      if (window.releaseReply.pending) return new Promise((_, reject) => init.signal.addEventListener('abort', () => reject(init.signal.reason), {once:true}));
      return new Response(JSON.stringify(window.releaseReply.body ?? {}),{status:window.releaseReply.status});
    };
    const invokeWithSettings=window.__TAURI_INTERNALS__.invoke;
    window.__TAURI_INTERNALS__.invoke=async(cmd,args)=>{
      if(cmd==='plugin:app|version' && query.has('cross-updates')) return '0.6.9';
      if(cmd==='list_release_notes') {
        if(window.releaseHistoryOffline) throw Error('合成发布历史读取失败');
        return window.releaseHistory ?? {releases:[],complete:true};
      }
      if(cmd==='get_latest_release_tag') {
        window.releasePageChecks++;
        if(window.releasePageOffline) throw Error('合成发布页断网');
        return window.releasePageTag ?? null;
      }
      if(cmd==='get_release_update_notes') {
        window.releaseNotesTags.push(args.tag);
        return window.releasePageNotes ?? '';
      }
      if(cmd==='get_settings'&&location.search.includes('holdappearance')) {
        const snapshot=await invokeWithSettings(cmd,args);
        return new Promise(resolve=>window.releaseAppearance=()=>resolve(snapshot));
      }
      if(cmd==='get_settings'&&location.search.includes('fail=settings'))throw Error('合成 settings 失败');
      return invokeWithSettings(cmd,args);
    };`,
  });
  const checks = [],
    failures = [];
  const test = async (name, fn) => {
    if (process.env.U2_TEST_FILTER && !name.includes(process.env.U2_TEST_FILTER)) return;
    try {
      await fn();
      checks.push(name);
    } catch (e) {
      failures.push({ name, error: e.stack ?? String(e) });
    }
  };
  const click = (selector) =>
    evaluate(`document.querySelector(${JSON.stringify(selector)}).click()`);
  const change = (selector, value) =>
    evaluate(
      `(()=>{const n=document.querySelector(${JSON.stringify(selector)});n.value=${JSON.stringify(value)};n.dispatchEvent(new Event('change',{bubbles:true}));})()`,
    );
  const appearance = async (patch) => {
    await evaluate(
      `window.settings={...window.settings,...${JSON.stringify(patch)}};window.emitEvent('settings-changed',window.settings)`,
    );
    await pause(100);
  };
  const nav = async (query = '') => {
    await send('Page.navigate', { url: origin + '/' + query });
    await until(() => evaluate('window.ready===true'), '未 ready');
    await pause(100);
  };
  const metrics = async (width = 1000) =>
    send('Emulation.setDeviceMetricsOverride', {
      width,
      height: 700,
      deviceScaleFactor: 1,
      mobile: false,
    });
  const appearanceRecorder = createAppearanceRecorder(evaluate, out);
  const capture = async (name) => {
    await appearanceRecorder.capture(name);
    const shot = await send('Page.captureScreenshot', { format: 'jpeg', quality: 65 });
    const bytes = Buffer.from(shot.data, 'base64');
    assert(bytes.length <= 300 * 1024);
    await writeFile(path.join(out, name + '.jpg'), bytes);
  };
  const selected = () =>
    evaluate("document.querySelector('.item[aria-selected=true]')?.dataset.id");
  const anchor = () =>
    evaluate(
      `(()=>{const l=document.querySelector('.list'),b=l.getBoundingClientRect();const r=[...l.querySelectorAll('.item')].find(r=>r.getBoundingClientRect().bottom>b.top+1);return {id:r?.dataset.id,offset:r?.getBoundingClientRect().top-b.top,scroll:l.scrollTop,height:r?.getBoundingClientRect().height};})()`,
    );
  await metrics();
  await nav();
  await test('AC1 默认浅色单列非紧凑与失焦关闭', async () => {
    assert.equal(await evaluate('document.documentElement.dataset.theme'), 'light');
    assert.equal(await evaluate('document.documentElement.dataset.skin'), 'clipraft');
    assert.equal(
      await evaluate("document.querySelector('main').classList.contains('compact')"),
      false,
    );
    assert.equal(await evaluate('window.settings.hide_on_blur'), false);
  });
  await click('[data-id="row-12"]');
  for (const theme of ['light', 'dark'])
    for (const mode of ['list', 'cards', 'compact']) {
      await metrics(mode === 'compact' ? 390 : 1000);
      await appearance({
        theme,
        layout: mode === 'cards' ? 'cards' : 'list',
        compact: mode === 'compact',
      });
      await test('AC2 ' + theme + '/' + mode + ' 布局、尺寸、选择', async () => {
        assert.equal(await selected(), 'row-12');
        assert.equal(await evaluate('document.documentElement.dataset.theme'), theme);
        assert.equal(await evaluate('document.documentElement.dataset.skin'), 'clipraft');
        assert.equal(
          await evaluate(
            "getComputedStyle(document.querySelector('.preview-secondary button') ?? document.querySelector('.kind-filters button')).borderRadius",
          ),
          '5px',
        );
        if (mode === 'compact') {
          assert.equal(await evaluate("!!document.querySelector('.detail')"), false);
          assert.equal(
            await evaluate("getComputedStyle(document.querySelector('.group-sidebar')).display"),
            'none',
          );
          assert.equal(
            await evaluate("document.querySelector('.item').getBoundingClientRect().height"),
            54,
          );
          assert.equal(
            await evaluate("document.querySelector('.type-icon').getBoundingClientRect().width"),
            32,
          );
          assert.equal(
            await evaluate("document.querySelector('select[aria-label=分组筛选]').options.length"),
            4,
          );
        } else {
          assert(await evaluate("!!document.querySelector('.detail')"));
          assert.equal(
            await evaluate("getComputedStyle(document.querySelector('.list')).display"),
            mode === 'cards' ? 'grid' : 'flex',
          );
        }
        assert(await evaluate('document.documentElement.scrollWidth<=innerWidth'));
      });
      await capture(theme + '-' + mode);
    }
  await test('AC3 分组×类型交集且切换不清空', async () => {
    await change('select[aria-label=分组筛选]', '7');
    await pause(70);
    await evaluate(
      "[...document.querySelectorAll('.kind-filters button')].find(b=>b.textContent==='文本').click()",
    );
    await pause(70);
    assert.equal(
      await evaluate("document.querySelector('select[aria-label=分组筛选]').value"),
      '7',
    );
    assert.equal(await evaluate("document.querySelector('.favorite-filter')"), null);
    assert.deepEqual(
      await evaluate("window.calls.filter(c=>c.cmd==='list_items').at(-1).args.filter"),
      { query: '', kind: 'text', group_id: 7 },
    );
    assert.equal(await evaluate("document.querySelectorAll('.item').length"), 20);
    await appearance({ theme: 'light', layout: 'cards', compact: false });
    assert.equal(
      await evaluate("window.calls.filter(c=>c.cmd==='list_items').at(-1).args.filter.group_id"),
      7,
    );
    assert.equal(await evaluate("document.querySelector('.favorite-filter')"), null);
    await appearance({ compact: true });
    await change('select[aria-label=分组筛选]', '8');
    await pause(70);
    assert.equal(await evaluate("document.querySelectorAll('.item').length"), 0);
    assert(await evaluate("document.body.textContent.includes('没有匹配的记录')"));
    await capture('empty');
  });
  await nav();
  await click('[data-id="row-12"]');
  await evaluate("document.querySelector('.list').scrollTop=700");
  await pause(80);
  for (const [name, patch] of [
    ['主题', { theme: 'dark' }],
    ['卡片', { layout: 'cards' }],
    ['紧凑', { compact: true }],
    ['退出紧凑', { compact: false }],
  ]) {
    await test('AC4 ' + name + ' 保留选择及滚动锚点', async () => {
      const before = await anchor();
      await appearance(patch);
      const after = await anchor();
      assert.equal(await selected(), 'row-12');
      const kept = await evaluate(
        `(()=>{const l=document.querySelector('.list'),r=document.querySelector('[data-id="${before.id}"]');return {offset:r.getBoundingClientRect().top-l.getBoundingClientRect().top,height:r.getBoundingClientRect().height};})()`,
      );
      assert(
        Math.abs(kept.offset - Math.max(before.offset, 2 - kept.height)) <= 2,
        JSON.stringify({ before, after, kept }),
      );
    });
  }
  await test('AC5 页首紧凑切换保存且退出恢复 cards', async () => {
    await appearance({ layout: 'cards', compact: false });
    await click('.compact-toggle');
    await pause(80);
    assert.equal(await evaluate('window.settingUpdates.at(-1).compact'), true);
    await click('.compact-toggle');
    await pause(80);
    assert.equal(await evaluate('window.settings.layout'), 'cards');
    assert.equal(await evaluate('window.settings.compact'), false);
  });
  await test('AC5 保存失败保留外观并显示错误', async () => {
    await evaluate('window.failSettings=true');
    await click('.compact-toggle');
    await pause(80);
    assert.equal(await evaluate('window.settings.compact'), false);
    assert(
      await evaluate("document.querySelector('[role=alert]')?.textContent.includes('快捷键')"),
    );
    await evaluate('window.failSettings=false');
  });
  await test('AC6 设置页编辑未生效、保存后生效及失焦配置往返', async () => {
    await click('.settings-toggle');
    await until(() => evaluate("!!document.querySelector('.settings-view form')"), '设置未打开');
    await change('select[aria-label=主题]', 'light');
    await change('select[aria-label=常规布局]', 'list');
    await evaluate(
      "[...document.querySelectorAll('.check')].find(l=>l.textContent.includes('失去焦点')).querySelector('input').click()",
    );
    assert.equal(await evaluate('document.documentElement.dataset.theme'), 'dark');
    await click('.settings-view button[type=submit]');
    await pause(100);
    assert.equal(await evaluate('document.documentElement.dataset.theme'), 'light');
    assert.equal(await evaluate('window.settings.hide_on_blur'), true);
    await capture('settings');
    await click('.settings-toggle');
    await click('.settings-toggle');
    await until(() => evaluate("!!document.querySelector('.settings-view form')"), '设置重开失败');
    assert.equal(
      await evaluate(
        "[...document.querySelectorAll('.check')].find(l=>l.textContent.includes('失去焦点')).querySelector('input').checked",
      ),
      true,
    );
    await click('.settings-toggle');
  });
  await nav();
  const reveal = async () => {
    await click('[data-id="row-3"]');
    await pause(50);
    await evaluate(
      "[...document.querySelectorAll('.sensitive-actions button')].find(b=>b.textContent.includes('显示原文')).click()",
    );
    await pause(50);
  };
  for (const [name, action] of [
    ['主题', () => appearance({ theme: 'dark' })],
    ['紧凑', () => appearance({ compact: true })],
    ['隐藏事件', () => evaluate("window.emitEvent('main-hidden')")],
    ['失焦', () => evaluate("window.dispatchEvent(new Event('blur'))")],
  ]) {
    await test('AC7 敏感原文在' + name + '后清理', async () => {
      await nav();
      await reveal();
      assert(await evaluate("document.body.textContent.includes('SYNTHETIC_SECRET_RAW')"));
      await action();
      await pause(60);
      assert.equal(
        await evaluate("document.body.textContent.includes('SYNTHETIC_SECRET_RAW')"),
        false,
      );
    });
  }
  await test('AC7 隐藏后迟到 reveal 响应被丢弃', async () => {
    await nav();
    await evaluate('window.holdReveal=true');
    await reveal();
    await evaluate("window.emitEvent('main-hidden');window.releaseReveal()");
    await pause(60);
    assert.equal(
      await evaluate("document.body.textContent.includes('SYNTHETIC_SECRET_RAW')"),
      false,
    );
  });
  await test('AC8 窄屏长错误不产生页面横向溢出', async () => {
    await metrics(390);
    await appearance({ compact: true, theme: 'dark' });
    await evaluate("window.emitEvent('operation-error','合成很长的错误信息'.repeat(40))");
    await pause(60);
    await capture('narrow-error');
    assert(await evaluate('document.documentElement.scrollWidth<=innerWidth'));
  });
  await test('AC9 quick 只应用主题、不请求主窗口尺寸或历史', async () => {
    await metrics(460);
    await nav('?panel=quick');
    await appearance({ theme: 'dark', layout: 'cards', compact: true });
    assert.equal(await evaluate('document.documentElement.dataset.theme'), 'dark');
    assert.equal(
      await evaluate(
        "window.calls.filter(c=>['sync_main_layout','list_items','item_detail'].includes(c.cmd)).length",
      ),
      0,
    );
    assert(
      await evaluate(
        "!!document.querySelector('.quick')&&!document.querySelector('.compact-toggle')",
      ),
    );
    await capture('quick-dark');
  });
  await test('AC10 外观根容器无旧样式外边距和间距', async () => {
    await metrics();
    await nav();
    assert.equal(
      await evaluate("getComputedStyle(document.querySelector('.main-window')).paddingTop"),
      '0px',
    );
    assert.equal(
      await evaluate("getComputedStyle(document.querySelector('.main-window')).rowGap"),
      '0px',
    );
  });
  await test('AC10 卡片图片降级信息和摘要均在自身边界内', async () => {
    await appearance({ layout: 'cards' });
    await capture('cards-image-fallback');
    const box = await evaluate(
      `(()=>{const r=document.querySelector('[data-id="row-2"]'),s=r.querySelector('.content');return {row:r.getBoundingClientRect().toJSON(),summary:s.getBoundingClientRect().toJSON(),display:getComputedStyle(r).display,rows:getComputedStyle(r).gridTemplateRows,assets:[...document.querySelectorAll('link[rel=stylesheet]')].map(l=>l.href)};})()`,
    );
    assert(box.summary.bottom <= box.row.bottom, JSON.stringify(box));
  });
  await test('AC4 设置页往返保留滚动锚点及选择', async () => {
    await nav();
    await click('[data-id="row-12"]');
    await evaluate("document.querySelector('.list').scrollTop=700");
    await pause(60);
    const before = await anchor();
    await click('.settings-toggle');
    await until(() => evaluate("!!document.querySelector('.settings-view form')"), '设置页未打开');
    await click('.settings-toggle');
    await pause(80);
    assert.equal(await selected(), 'row-12');
    const after = await anchor();
    assert.equal(after.id, before.id, JSON.stringify({ before, after }));
  });
  await test('AC10 390px 紧凑容器不继承旧响应式边距', async () => {
    await metrics(390);
    await appearance({ compact: true });
    assert.equal(
      await evaluate("getComputedStyle(document.querySelector('.main-window')).paddingTop"),
      '0px',
    );
  });
  await test('AC4 从设置修改布局返回保持选择、筛选和滚动锚点', async () => {
    await metrics();
    await nav();
    await evaluate("document.querySelector('.kind-filters button:nth-child(2)').click()");
    await pause(80);
    await click('[data-id="row-12"]');
    await evaluate("document.querySelector('.list').scrollTop=450");
    await pause(60);
    const before = await anchor();
    await click('.settings-toggle');
    await until(() => evaluate("!!document.querySelector('.settings-view form')"), '设置未打开');
    await change('select[aria-label=常规布局]', 'cards');
    await click('.settings-view button[type=submit]');
    await pause(100);
    await click('.settings-toggle');
    await pause(100);
    assert.equal(await selected(), 'row-12');
    assert.equal(
      await evaluate(
        "document.querySelector('.kind-filters button:nth-child(2)').getAttribute('aria-pressed')",
      ),
      'true',
    );
    const after = await anchor();
    const restored = await evaluate(
      `(()=>{const l=document.querySelector('.list'),r=document.querySelector('[data-id="${before.id}"]');return {id:r?.dataset.id,offset:r?.getBoundingClientRect().top-l.getBoundingClientRect().top};})()`,
    );
    assert.equal(restored.id, before.id);
    assert(
      Math.abs(restored.offset - before.offset) <= 2,
      JSON.stringify({ before, after, restored }),
    );
  });
  await test('AC11 真正加载的图片缩略图保持 contain 及紧凑尺寸', async () => {
    await metrics(390);
    await nav();
    await evaluate(
      "window.rows[4]={...window.rows[4],kind:'image',thumbnail:'synthetic-thumb'};window.emitEvent('history-changed')",
    );
    await pause(100);
    await appearance({ compact: true });
    await until(
      () => evaluate('document.querySelector(\'[data-id="row-4"] img\')?.naturalWidth===160'),
      '合成图片未解码',
    );
    assert.equal(
      await evaluate(
        'getComputedStyle(document.querySelector(\'[data-id="row-4"] img\')).objectFit',
      ),
      'contain',
    );
    assert.equal(
      await evaluate(
        'document.querySelector(\'[data-id="row-4"] .preview\').getBoundingClientRect().height',
      ),
      38,
    );
    await capture('compact-image-loaded');
  });
  await test('AC8 原生尺寸 IPC 失败明确提示且设置仍持久化', async () => {
    await metrics();
    await nav();
    await evaluate(
      "const prev=window.__TAURI_INTERNALS__.invoke;window.__TAURI_INTERNALS__.invoke=(cmd,args)=>cmd==='sync_main_layout'?Promise.reject('合成原生调整失败'):prev(cmd,args)",
    );
    await click('.compact-toggle');
    await pause(100);
    assert.equal(await evaluate('window.settings.compact'), true);
    assert(
      await evaluate(
        "document.querySelector('[role=alert]')?.textContent.includes('窗口尺寸调整失败')",
      ),
    );
  });

  await test('R01 自绘窗口操作 IPC 与拖动区', async () => {
    await metrics();
    await nav();
    assert(
      await evaluate("document.querySelector('header').hasAttribute('data-tauri-drag-region')"),
    );
    assert.equal(
      await evaluate(
        "document.querySelectorAll('.window-controls button[data-tauri-drag-region]').length",
      ),
      0,
    );
    for (const label of ['最小化', '最大化或还原', '隐藏到托盘'])
      await click(`button[aria-label="${label}"]`);
    assert.deepEqual(
      await evaluate("window.calls.filter(c=>c.cmd.startsWith('plugin:window|')).map(c=>c.cmd)"),
      ['plugin:window|minimize', 'plugin:window|toggle_maximize', 'plugin:window|close'],
    );
  });
  await test('R02 收藏入口、标记、Ctrl+D 均移除且旧记录可见', async () => {
    await nav();
    assert.equal(await evaluate("document.body.textContent.includes('收藏')"), false);
    assert.equal(await evaluate("document.querySelectorAll('.item').length"), 60);
    await evaluate(
      "window.dispatchEvent(new KeyboardEvent('keydown',{key:'d',ctrlKey:true,bubbles:true}))",
    );
    assert.equal(await evaluate("window.calls.filter(c=>c.cmd==='set_favorite').length"), 0);
    await evaluate(
      "document.querySelector('.item').dispatchEvent(new MouseEvent('contextmenu',{bubbles:true,clientX:200,clientY:160}))",
    );
    assert.equal(
      await evaluate("document.querySelector('.context-menu').textContent.includes('收藏')"),
      false,
    );
  });
  for (const theme of ['light', 'dark']) {
    await test('R04 ' + theme + ' 移出分组红底、说明及键盘可达', async () => {
      await metrics();
      await nav();
      await appearance({ theme });
      await evaluate(
        "document.querySelector('.item').dispatchEvent(new MouseEvent('contextmenu',{bubbles:true,clientX:200,clientY:160}))",
      );
      await click('[aria-haspopup=menu]');
      const style = await evaluate(
        "(()=>{const n=document.querySelector('.remove-group'),s=getComputedStyle(n);return {title:n.title,background:s.backgroundColor,color:s.color,separator:!!n.previousElementSibling.matches('[role=separator]'),focused:document.activeElement.closest('.group-submenu')!==null};})()",
      );
      assert.equal(style.title, '保留记录，仅移出当前分组');
      assert(style.separator && style.focused);
      assert.notEqual(style.background, 'rgba(0, 0, 0, 0)');
      await capture('revision-' + theme + '-menu');
      await evaluate(
        "document.querySelector('.remove-group').focus();document.activeElement.dispatchEvent(new KeyboardEvent('keydown',{key:'Enter',bubbles:true,cancelable:true}))",
      );
      await pause(80);
      assert.deepEqual(await evaluate("window.calls.findLast(c=>c.cmd==='move_to_group').args"), {
        ids: ['row-0'],
        groupId: null,
      });
    });
    await test('R05 ' + theme + ' 混合内容卡片等宽等高且状态完整', async () => {
      await metrics();
      await nav();
      await appearance({ theme, layout: 'cards' });
      await evaluate(
        "window.rows[0].summary='长摘要'.repeat(200);window.rows[0].pinned=true;window.rows[0].sensitive=true;window.rows[0].expires_at=Date.now()+120000;window.emitEvent('history-changed')",
      );
      await pause(100);
      const boxes = await evaluate(
        "[...document.querySelectorAll('.item')].slice(0,6).map(n=>{const r=n.getBoundingClientRect();return {w:r.width,h:r.height,inside:[...n.querySelectorAll('.summary,.content,.sensitive-badge,.file-status,.item-badges,.source-name')].every(c=>{const b=c.getBoundingClientRect();return b.bottom<=r.bottom&&b.right<=r.right}),summary:getComputedStyle(n.querySelector('.summary')).webkitLineClamp}})",
      );
      assert(
        boxes.every(
          (b) =>
            b.h === boxes[0].h && Math.abs(b.w - boxes[0].w) < 1 && b.inside && b.summary === '2',
        ),
        JSON.stringify(boxes),
      );
      await capture('revision-' + theme + '-cards');
    });
    for (const width of [1000, 390])
      await test('R06 ' + theme + ' ' + width + ' 分类草稿、取消与独立底栏', async () => {
        await metrics(width);
        await nav();
        await appearance({ theme, compact: width === 390 });
        await click('.settings-toggle');
        await until(
          () => evaluate("!!document.querySelector('.settings-view form')"),
          '设置未打开',
        );
        const category = async (id) =>
          width === 390
            ? change('select[aria-label=设置分类]', id)
            : click(
                `.settings-nav button:nth-child(${['general', 'appearance', 'history', 'privacy', 'hotkeys'].indexOf(id) + 1})`,
              );
        await category('appearance');
        await change('[aria-label=主题]', theme === 'light' ? 'dark' : 'light');
        await category('privacy');
        assert.equal(
          await evaluate(
            "[...document.querySelectorAll('fieldset')].filter(n=>getComputedStyle(n).display!=='none').map(n=>n.dataset.category).join()",
          ),
          'privacy',
        );
        await evaluate(
          "document.querySelector('.add-app input').value='unsaved.exe';document.querySelector('.add-app input').dispatchEvent(new Event('input',{bubbles:true}))",
        );
        await category('appearance');
        assert.equal(
          await evaluate("document.querySelector('[aria-label=主题]').value"),
          theme === 'light' ? 'dark' : 'light',
        );
        await category('privacy');
        assert.equal(
          await evaluate("document.querySelector('.add-app input').value"),
          'unsaved.exe',
        );
        await evaluate("document.querySelector('.settings-content').scrollTop=99999");
        const bounds = await evaluate(
          "(()=>{const c=document.querySelector('.settings-content').getBoundingClientRect(),f=document.querySelector('.settings-actions').getBoundingClientRect(),n=document.querySelector('.settings-cancel').getBoundingClientRect();return {content:c.bottom,footer:f.top,bottom:f.bottom,cancel:n.bottom,overflow:document.documentElement.scrollWidth>innerWidth};})()",
        );
        assert(
          bounds.content <= bounds.footer + 1 &&
            bounds.bottom <= 700 &&
            bounds.cancel <= 700 &&
            !bounds.overflow,
          JSON.stringify(bounds),
        );
        await capture('revision-' + theme + '-settings-' + width);
        await click('.settings-cancel');
        await pause(60);
        assert.equal(await evaluate("document.querySelector('.settings-view')"), null);
        await click('.settings-toggle');
        await until(
          () => evaluate("!!document.querySelector('.settings-view form')"),
          '设置未重开',
        );
        assert.equal(await evaluate("document.querySelector('[aria-label=主题]').value"), theme);
        assert.equal(await evaluate("document.querySelector('.add-app input').value"), '');
        assert.equal(await evaluate('window.settingUpdates.length'), 0);
      });
  }
  await test('R06 长内容独立滚动、跨分类统一保存及取消恢复新保存值', async () => {
    await metrics();
    await nav();
    await click('.settings-toggle');
    await until(() => evaluate("!!document.querySelector('.settings-view form')"), '设置未打开');
    await click('.settings-nav button:nth-child(4)');
    await evaluate(
      "window.settings.ignored_apps=Array.from({length:80},(_,i)=>'synthetic'+i+'.exe')",
    );
    await click('.settings-cancel');
    await click('.settings-toggle');
    await until(() => evaluate("!!document.querySelector('.settings-view form')"), '设置未重开');
    await click('.settings-nav button:nth-child(4)');
    const top = await evaluate(
      "document.querySelector('.settings-actions').getBoundingClientRect().top",
    );
    await evaluate(
      "const n=document.querySelector('.settings-content');n.scrollTop=n.scrollHeight",
    );
    assert(await evaluate("document.querySelector('.settings-content').scrollTop>500"));
    assert.equal(
      await evaluate("document.querySelector('.settings-actions').getBoundingClientRect().top"),
      top,
    );
    await capture('revision-settings-long');
    await click('.settings-nav button:nth-child(2)');
    await change('[aria-label=主题]', 'dark');
    await click('.settings-nav button:nth-child(1)');
    await evaluate("document.querySelector('input[type=checkbox]').click()");
    await click('.settings-view button[type=submit]');
    await pause(100);
    assert.equal(await evaluate('window.settingUpdates.length'), 1);
    assert.equal(await evaluate('window.settings.theme'), 'dark');
    assert.equal(await evaluate('window.settings.hide_on_blur'), true);
    await click('.settings-nav button:nth-child(2)');
    await change('[aria-label=主题]', 'light');
    await click('.settings-cancel');
    assert.equal(await evaluate('document.documentElement.dataset.theme'), 'dark');
    await click('.settings-toggle');
    await until(() => evaluate("!!document.querySelector('.settings-view form')"), '设置未重开');
    assert.equal(await evaluate("document.querySelector('[aria-label=主题]').value"), 'dark');
  });
  await test('R03 紧凑分组与类型字号、行高一致', async () => {
    await metrics(390);
    await nav();
    await appearance({ compact: true, skin: 'fluent' });
    const groupSelect = await evaluate(`(() => {
      const select = document.querySelector('.compact-groups select');
      const style = getComputedStyle(select);
      const selected = getComputedStyle(select.options[select.selectedIndex]);
      const option = getComputedStyle(select.options[1]);
      return {
        baseSelectSupported: CSS.supports('appearance', 'base-select'),
        appearance: style.appearance,
        background: style.backgroundColor,
        image: style.backgroundImage,
        borderBottom: style.borderBottomColor,
        radius: style.borderRadius,
        paddingRight: style.paddingRight,
        pickerRadius: getComputedStyle(select, '::picker(select)').borderRadius,
        pickerShadow: getComputedStyle(select, '::picker(select)').boxShadow,
        pickerBackground: getComputedStyle(select, '::picker(select)').backgroundColor,
        optionColor: option.color,
        optionBackground: option.backgroundColor,
        selectedBackground: selected.backgroundColor,
      };
    })()`);
    assert.equal(groupSelect.appearance, groupSelect.baseSelectSupported ? 'base-select' : 'none');
    assert.equal(groupSelect.background, 'rgb(255, 255, 255)');
    assert.equal(groupSelect.image === 'none', groupSelect.baseSelectSupported);
    assert.equal(groupSelect.borderBottom, 'rgb(170, 187, 180)');
    assert.equal(groupSelect.radius, '4px');
    assert.equal(groupSelect.paddingRight, groupSelect.baseSelectSupported ? '8px' : '30px');
    assert.equal(groupSelect.optionColor, 'rgb(32, 41, 39)');
    assert.equal(groupSelect.optionBackground, 'rgb(255, 255, 255)');
    assert.equal(groupSelect.selectedBackground, 'rgb(227, 240, 236)');
    if (groupSelect.baseSelectSupported) {
      assert.equal(groupSelect.pickerRadius, '8px');
      assert.notEqual(groupSelect.pickerShadow, 'none');
      assert.equal(groupSelect.pickerBackground, 'rgb(255, 255, 255)');
      const selectPoint = await evaluate(
        "(()=>{const r=document.querySelector('.compact-groups select').getBoundingClientRect();return {x:r.x+r.width/2,y:r.y+r.height/2}})()",
      );
      for (const type of ['mousePressed', 'mouseReleased']) {
        await send('Input.dispatchMouseEvent', {
          type,
          ...selectPoint,
          button: 'left',
          clickCount: 1,
        });
      }
      await until(
        () => evaluate("!!document.querySelector('.compact-groups select:open')"),
        '紧凑分组 Fluent picker 未打开',
      );
      await capture('u7-fluent-compact-group-picker');
      await send('Input.dispatchKeyEvent', {
        type: 'keyDown',
        key: 'ArrowDown',
        code: 'ArrowDown',
        windowsVirtualKeyCode: 40,
      });
      await send('Input.dispatchKeyEvent', {
        type: 'keyUp',
        key: 'ArrowDown',
        code: 'ArrowDown',
        windowsVirtualKeyCode: 40,
      });
      await send('Input.dispatchKeyEvent', {
        type: 'keyDown',
        key: 'Escape',
        code: 'Escape',
        windowsVirtualKeyCode: 27,
      });
      await send('Input.dispatchKeyEvent', {
        type: 'keyUp',
        key: 'Escape',
        code: 'Escape',
        windowsVirtualKeyCode: 27,
      });
      assert.equal(await evaluate("document.querySelector('.compact-groups select').value"), 'all');
    }
    await capture('u7-fluent-compact-group-select');
    assert.equal(
      await evaluate("getComputedStyle(document.querySelector('.compact-groups')).fontSize"),
      await evaluate("getComputedStyle(document.querySelector('.filter-label')).fontSize"),
    );
    assert.equal(
      await evaluate(
        "getComputedStyle(document.querySelector('.compact-groups select')).lineHeight",
      ),
      await evaluate("getComputedStyle(document.querySelector('.kind-filters button')).lineHeight"),
      JSON.stringify(
        await evaluate(
          "[...document.querySelectorAll('.compact-groups select,.kind-filters button')].map(n=>({tag:n.outerHTML,line:getComputedStyle(n).lineHeight}))",
        ),
      ),
    );
  });

  await test('R07 常规设置入口位于分组侧栏底部，顶部仅紧凑切换与窗口操作', async () => {
    await metrics();
    await nav();
    assert.equal(
      await evaluate("document.querySelector('header .settings-toggle,header .count')"),
      null,
    );
    assert.equal(await evaluate("document.querySelectorAll('header>button').length"), 1);
    const bounds = await evaluate(
      "(()=>{const n=document.querySelector('.sidebar-bottom .settings-toggle'),b=n.getBoundingClientRect(),s=n.closest('.group-sidebar').getBoundingClientRect();return {inside:b.left>=s.left&&b.right<=s.right,gap:s.bottom-b.bottom,available:b.width>0&&b.height>0};})()",
    );
    assert(bounds.inside && bounds.available && bounds.gap <= 25, JSON.stringify(bounds));
    await capture('followup-main-sidebar');
    await click('.sidebar-bottom .settings-toggle');
    await until(
      () => evaluate("!!document.querySelector('.settings-view form')"),
      '侧栏设置未打开',
    );
  });
  for (const theme of ['light', 'dark'])
    await test('R07 ' + theme + ' 右侧独立底栏、左提示与右按钮', async () => {
      await metrics();
      await nav();
      await appearance({ theme });
      await click('.settings-toggle');
      await until(() => evaluate("!!document.querySelector('.settings-view form')"), '设置未打开');
      const box = await evaluate(
        "(()=>{const r=n=>document.querySelector(n).getBoundingClientRect().toJSON();return {nav:r('.settings-nav'),content:r('.settings-content'),footer:r('.settings-actions'),feedback:r('.settings-feedback'),buttons:r('.settings-buttons')};})()",
      );
      assert.equal(box.footer.left, box.content.left);
      assert.equal(box.footer.right, box.content.right);
      assert(
        box.nav.bottom >= box.footer.bottom &&
          box.nav.right <= box.footer.left + 1 &&
          box.content.bottom <= box.footer.top + 1,
        JSON.stringify(box),
      );
      assert(
        box.feedback.right <= box.buttons.left && box.footer.right - box.buttons.right <= 25,
        JSON.stringify(box),
      );
      await capture('followup-' + theme + '-settings-footer');
      await click('.settings-nav button:nth-child(3)');
      await evaluate(
        "const n=document.querySelector('input[type=number]');n.value='10';n.dispatchEvent(new Event('input',{bubbles:true}))",
      );
      await click('.settings-buttons button[type=submit]');
      await until(
        () => evaluate("!!document.querySelector('.settings-feedback .delete-confirm')"),
        '清理确认不在左侧提示区',
      );
      assert(
        await evaluate(
          "document.querySelector('.settings-content').getBoundingClientRect().bottom<=document.querySelector('.settings-actions').getBoundingClientRect().top+1",
        ),
      );
    });
  await test('R07 紧凑入口可用且窄窗底栏按钮靠右', async () => {
    await metrics(390);
    await nav();
    await appearance({ compact: true });
    assert.equal(await evaluate("document.querySelectorAll('.settings-toggle').length"), 1);
    assert(
      await evaluate(
        "document.querySelector('footer .settings-toggle').getBoundingClientRect().width>0",
      ),
    );
    await click('footer .settings-toggle');
    await until(
      () => evaluate("!!document.querySelector('.settings-view form')"),
      '紧凑设置未打开',
    );
    const b = await evaluate(
      "(()=>{const f=document.querySelector('.settings-actions').getBoundingClientRect(),t=document.querySelector('.settings-feedback').getBoundingClientRect(),a=document.querySelector('.settings-buttons').getBoundingClientRect();return {right:f.right-a.right,aligned:t.right<=a.left,bottom:a.bottom,overflow:document.documentElement.scrollWidth>innerWidth};})()",
    );
    assert(b.right <= 17 && b.aligned && b.bottom <= 700 && !b.overflow, JSON.stringify(b));
    await capture('followup-compact-settings-footer');
    await click('.settings-cancel');
    assert(await evaluate("!!document.querySelector('footer .settings-toggle')"));
  });

  await test('R08 品牌与设计稿结构：线性图标、搜索提示、侧栏分组、预览底部操作', async () => {
    await metrics();
    await nav();
    assert.equal(await evaluate("document.querySelector('header h1').textContent"), '片筏剪贴板');
    assert(await evaluate("document.title.includes('ClipRaft')"));
    assert(
      await evaluate(
        "!!document.querySelector('.brand-mark img')?.naturalWidth&&!!document.querySelector('.searchbox svg')&&!!document.querySelector('.group-views svg')",
      ),
    );
    assert.equal(await evaluate("document.querySelector('.searchbox kbd').textContent"), 'Ctrl+F');
    assert.equal(
      await evaluate("document.querySelector('.groups-heading').textContent"),
      '我的分组',
    );
    const bounds = await evaluate(
      "(()=>{const r=n=>document.querySelector(n).getBoundingClientRect();return {bottom:r('.detail-actions').bottom,detail:r('.detail').bottom,status:r('.main-statusbar').top,statusLeft:r('.main-statusbar').left,mainLeft:r('main').left};})()",
    );
    assert(
      bounds.bottom < bounds.detail &&
        bounds.detail <= bounds.status + 1 &&
        Math.abs(bounds.statusLeft - bounds.mainLeft) <= 1,
      JSON.stringify(bounds),
    );
    await evaluate(
      "document.querySelector('.item').focus();document.activeElement.dispatchEvent(new KeyboardEvent('keydown',{key:'f',ctrlKey:true,bubbles:true,cancelable:true}))",
    );
    await pause(50);
    assert(await evaluate("document.activeElement===document.querySelector('input[type=search]')"));
    await capture('brand-light-list');
  });
  for (const theme of ['light', 'dark'])
    await test('R08 ' + theme + ' 紧凑置顶标识可见且不挤出内容', async () => {
      await metrics(390);
      await nav();
      await evaluate(
        "window.rows[0].pinned=true;window.rows[0].summary='用于检验长摘要与置顶标识的合成内容'.repeat(8);window.emitEvent('history-changed')",
      );
      await pause(80);
      await appearance({ theme, compact: true });
      const row = await evaluate(
        "(()=>{const n=document.querySelector('[data-id=\"row-0\"]'),p=n.querySelector('.compact-pin'),r=n.getBoundingClientRect(),b=p.getBoundingClientRect();return {height:r.height,pinWidth:b.width,pinInside:b.right<=r.right&&b.left>=r.left&&b.bottom<=r.bottom,label:p.getAttribute('aria-label'),summary:n.querySelector('.summary').getBoundingClientRect().right,pinLeft:b.left};})()",
      );
      assert.equal(row.height, 54);
      assert(row.pinWidth > 0 && row.pinInside && row.summary <= row.pinLeft, JSON.stringify(row));
      assert.equal(row.label, '置顶');
      assert.equal(
        await evaluate('document.querySelector(\'[data-id="row-1"] .compact-pin\')'),
        null,
      );
      await capture('brand-' + theme + '-compact-pinned');
    });

  await test('R09 小尺寸紧凑图标与预览右上刷新图标', async () => {
    await metrics();
    await nav();
    const b = await evaluate(
      "(()=>{const n=document.querySelector('.compact-toggle'),r=n.getBoundingClientRect(),h=document.querySelector('header').getBoundingClientRect(),f=document.querySelector('.preview-refresh'),p=f.getBoundingClientRect(),c=document.querySelector('.detail-caption').getBoundingClientRect();return {w:r.width,h:r.height,inside:r.top>=h.top&&r.bottom<=h.bottom,icon:!!n.querySelector('svg'),label:n.getAttribute('aria-label'),refresh:!!f.querySelector('svg'),refreshLabel:f.getAttribute('aria-label'),right:c.right-p.right,top:p.top-c.top,bottom:!!document.querySelector('.detail-bottom .preview-refresh')};})()",
    );
    assert.equal(b.w, 28);
    assert.equal(b.h, 28);
    assert(b.inside && b.icon && b.refresh && !b.bottom);
    assert.equal(b.label, '紧凑模式');
    assert.equal(b.refreshLabel, '刷新状态');
    assert(b.right <= 1 && b.top <= 1, JSON.stringify(b));
    await click('.compact-toggle');
    await pause(80);
    assert.equal(
      await evaluate("document.querySelector('.compact-toggle').getAttribute('aria-label')"),
      '展开视图',
    );
    assert.equal(
      await evaluate("document.querySelector('.compact-toggle').getAttribute('aria-pressed')"),
      'true',
    );
  });
  for (const theme of ['light', 'dark'])
    for (const mode of ['list', 'compact', 'cards'])
      await test('R10 ' + theme + '/' + mode + ' 统一类型图标与左侧图片缩略图', async () => {
        await metrics(mode === 'compact' ? 390 : 1000);
        await nav();
        await evaluate(
          "window.rows[4]={...window.rows[4],kind:'image',thumbnail:'synthetic-thumb',summary:'合成图片长摘要'.repeat(30)};window.rows[0].pinned=true;window.emitEvent('history-changed')",
        );
        await pause(80);
        await appearance({
          theme,
          layout: mode === 'cards' ? 'cards' : 'list',
          compact: mode === 'compact',
        });
        await click('[data-id="row-4"]');
        await until(
          () =>
            evaluate(
              'document.querySelector(\'[data-id="row-4"] .record-thumbnail img\')?.naturalWidth===160',
            ),
          '右侧合成图片未加载',
        );
        if (mode !== 'compact') {
          assert.equal(await evaluate("!!document.querySelector('.detail .preview-title')"), false);
        }
        const boxes = await evaluate(
          "(()=>{const n=document.querySelector('[data-id=\"row-4\"]'),r=n.getBoundingClientRect(),i=n.querySelector('.type-icon')?.getBoundingClientRect(),p=n.querySelector('.record-thumbnail').getBoundingClientRect(),c=n.querySelector('.content').getBoundingClientRect();return {row:r.toJSON(),icon:i?.toJSON(),preview:p.toJSON(),content:c.toJSON(),fit:getComputedStyle(n.querySelector('img')).objectFit,icons:[...document.querySelectorAll('.item>.type-icon')].slice(0,6).map(n=>({w:n.getBoundingClientRect().width,h:n.getBoundingClientRect().height}))};})()",
        );
        assert.equal(boxes.fit, 'contain');
        assert(
          await evaluate(
            "!!document.querySelector('[data-id=\"row-2\"] .record-thumbnail .image-placeholder svg')&&!!document.querySelector('[data-id=\"row-2\"] .record-thumbnail .image-placeholder').getAttribute('aria-label')",
          ),
        );
        if (mode !== 'cards') {
          const size = mode === 'compact' ? 32 : 48;
          assert(
            boxes.icons.every((b) => b.w === size && b.h === size),
            JSON.stringify(boxes.icons),
          );
          assert(
            !boxes.icon &&
              boxes.preview.right <= boxes.content.left &&
              boxes.preview.left >= boxes.row.left,
            JSON.stringify(boxes),
          );
          if (mode === 'compact') {
            assert.equal(boxes.row.height, 54);
            assert.equal(boxes.preview.width, 52);
            assert.equal(boxes.preview.height, 38);
          }
        } else {
          assert.equal(boxes.row.height, 248);
          assert(boxes.preview.bottom <= boxes.content.top, JSON.stringify(boxes));
        }
        assert(await evaluate('document.documentElement.scrollWidth<=innerWidth'));
        await capture('r10-' + theme + '-' + mode);
        await evaluate(
          "window.rows[4].sensitive=true;window.rows[4].summary='合成敏感••••';window.emitEvent('history-changed')",
        );
        await pause(60);
        assert.equal(
          await evaluate('document.querySelector(\'[data-id="row-4"] .record-thumbnail\')'),
          null,
        );
        assert(await evaluate('!!document.querySelector(\'[data-id="row-4"] .type-icon svg\')'));
      });

  for (const theme of ['light', 'dark'])
    await test('R11 ' + theme + ' 常规图标与图片共用中心线且正文对齐', async () => {
      await metrics();
      await nav();
      await appearance({ theme });
      await evaluate(
        "window.rows[4]={...window.rows[4],kind:'image',thumbnail:'synthetic-thumb'};window.emitEvent('history-changed')",
      );
      await pause(80);
      const rows = await evaluate(
        "[...document.querySelectorAll('.item')].slice(0,6).map(n=>{const r=n.getBoundingClientRect(),v=n.querySelector('.type-icon,.record-thumbnail').getBoundingClientRect(),c=n.querySelector('.content').getBoundingClientRect();return {rowHeight:r.height,cx:(v.left+v.right)/2,cy:(v.top+v.bottom)/2,rowCy:(r.top+r.bottom)/2,textLeft:c.left,w:v.width,h:v.height,image:!!n.querySelector('.record-thumbnail')};})",
      );
      assert(
        rows.every(
          (r) =>
            r.rowHeight >= 84 &&
            Math.abs(r.cx - rows[0].cx) <= 1 &&
            Math.abs(r.cy - r.rowCy) <= 1 &&
            Math.abs(r.textLeft - rows[0].textLeft) <= 1,
        ),
        JSON.stringify(rows),
      );
      assert(rows.filter((r) => r.image).every((r) => r.w === 80 && r.h === 60));
      assert(rows.filter((r) => !r.image).every((r) => r.w === 48 && r.h === 48));
      assert(
        await evaluate(
          "[...document.querySelectorAll('.item>.type-icon>.ui-icon')].every(n=>n.getBoundingClientRect().width===26&&n.getBoundingClientRect().height===26)",
        ),
      );
      await capture('r11-' + theme + '-centered-list');
    });
  await test('R10 紧凑浮窗请求跟随选择、设置、隐藏与重新显示', async () => {
    await metrics(390);
    await nav();
    assert.equal(
      await evaluate("window.calls.filter(c=>c.cmd==='sync_compact_preview').at(-1)?.args.id"),
      null,
    );
    await appearance({ compact: true });
    await click('[data-id="row-2"]');
    await pause(70);
    assert.equal(
      await evaluate("window.calls.filter(c=>c.cmd==='sync_compact_preview').at(-1).args.id"),
      'row-2',
    );
    await click('.settings-toggle');
    await pause(70);
    assert.equal(
      await evaluate("window.calls.filter(c=>c.cmd==='sync_compact_preview').at(-1).args.id"),
      null,
    );
    await click('.settings-cancel');
    await pause(70);
    assert.equal(
      await evaluate("window.calls.filter(c=>c.cmd==='sync_compact_preview').at(-1).args.id"),
      'row-2',
    );
    await evaluate("window.emitEvent('main-hidden')");
    await click('[data-id="row-4"]');
    await pause(70);
    assert.equal(
      await evaluate("window.calls.filter(c=>c.cmd==='sync_compact_preview').at(-1).args.id"),
      null,
    );
    await evaluate("window.emitEvent('main-shown')");
    await pause(70);
    assert.equal(
      await evaluate("window.calls.filter(c=>c.cmd==='sync_compact_preview').at(-1).args.id"),
      'row-4',
    );
    await appearance({ compact: false });
    assert.equal(
      await evaluate("window.calls.filter(c=>c.cmd==='sync_compact_preview').at(-1).args.id"),
      null,
    );
  });

  await test('B04 浮窗显示回送 focus 时不会重复请求或形成反馈环', async () => {
    await metrics(390);
    await nav();
    await evaluate('window.echoPreviewFocus=true;window.calls=[]');
    await appearance({ compact: true });
    await pause(100);
    assert.equal(
      await evaluate("window.calls.filter(c=>c.cmd==='sync_compact_preview'&&c.args.id).length"),
      1,
    );
    await evaluate(
      "for(let i=0;i<20;i++){window.dispatchEvent(new Event('focus'));window.emitEvent('main-activated')}",
    );
    await pause(70);
    assert.equal(
      await evaluate("window.calls.filter(c=>c.cmd==='sync_compact_preview'&&c.args.id).length"),
      1,
    );
    await click('[data-id="row-2"]');
    await pause(80);
    assert.equal(
      await evaluate("window.calls.filter(c=>c.cmd==='sync_compact_preview'&&c.args.id).length"),
      2,
    );
    assert.equal(await evaluate('window.previewFocusEchoes'), 2);
    await evaluate("window.emitEvent('main-deactivated')");
    await pause(60);
    assert.equal(
      await evaluate("window.calls.filter(c=>c.cmd==='sync_compact_preview').at(-1).args.id"),
      null,
    );
    await evaluate("window.emitEvent('main-activated')");
    await pause(60);
    assert.equal(
      await evaluate("window.calls.filter(c=>c.cmd==='sync_compact_preview').at(-1).args.id"),
      'row-2',
    );
  });
  await test('B04 常规窗口反复 focus/blur 不清空或重复加载普通详情', async () => {
    await metrics();
    await nav();
    await until(() => evaluate("!!document.querySelector('.text-detail')"), '普通详情未加载');
    const before = await evaluate("window.calls.filter(c=>c.cmd==='item_detail').length");
    await evaluate(
      "for(let i=0;i<20;i++){window.dispatchEvent(new Event('blur'));window.dispatchEvent(new Event('focus'));window.emitEvent('main-activated')}",
    );
    await pause(80);
    assert.equal(await evaluate("window.calls.filter(c=>c.cmd==='item_detail').length"), before);
    assert.equal(await evaluate("document.querySelector('.text-detail')?.textContent"), '合成详情');
  });
  const floatingNav = async (query = '') => {
    await send('Emulation.setDeviceMetricsOverride', {
      width: 360,
      height: 420,
      deviceScaleFactor: 1,
      mobile: false,
    });
    await send('Page.navigate', { url: origin + '/?panel=preview' + query });
    await until(() => evaluate("!!document.querySelector('.floating-preview')"), '浮窗入口未加载');
    await pause(70);
  };
  const floatingContent = (id = 'image') => ({
    generation: 42,
    item: {
      id,
      kind: 'image',
      summary: '合成图片',
      thumbnail: 'synthetic-thumb',
      preview: 'synthetic-thumb',
      files: [],
      formats: [],
    },
    detail: { text: '', files: [], truncated: false },
  });
  await test('R10 浮窗独立入口与真实图片 contain、无主窗口或粘贴入口', async () => {
    await floatingNav();
    await evaluate(
      `window.emitEvent('compact-preview-changed',${JSON.stringify(floatingContent())})`,
    );
    await until(
      () => evaluate("document.querySelector('.floating-preview img')?.naturalWidth===160"),
      '浮窗图片未实际加载',
    );
    assert.equal(
      await evaluate("getComputedStyle(document.querySelector('.floating-preview img')).objectFit"),
      'contain',
    );
    assert(
      await evaluate(
        "document.querySelector('.floating-preview img').getBoundingClientRect().width>=280&&Math.abs(document.querySelector('.floating-preview img').getBoundingClientRect().width-document.querySelector('.floating-body').clientWidth)<=1",
      ),
    );
    assert.equal(await evaluate("document.querySelector('.main-window,.quick,button')"), null);
    assert.equal(
      await evaluate(
        "window.calls.filter(c=>['list_items','item_detail','panel_state','paste_item','sync_main_layout','sync_compact_preview'].includes(c.cmd)).length",
      ),
      0,
    );
    assert(await evaluate('document.documentElement.scrollWidth<=360'));
    await capture('r10-floating-image-light');
    await appearance({ theme: 'dark' });
    assert.equal(await evaluate('document.documentElement.dataset.theme'), 'dark');
    assert.equal(await evaluate('document.documentElement.dataset.skin'), 'clipraft');
    assert.equal(
      await evaluate("getComputedStyle(document.querySelector('.floating-preview')).borderRadius"),
      '8px',
    );
    await capture('r10-floating-image-dark');
  });
  await test('R10 浮窗文本、文件、敏感打码与清空', async () => {
    const content = floatingContent('text');
    content.item.kind = 'text';
    content.item.thumbnail = null;
    content.item.preview = null;
    content.detail.text = '<b>合成文本</b>\n' + '独立预览内容'.repeat(180);
    await evaluate(`window.emitEvent('compact-preview-changed',${JSON.stringify(content)})`);
    await pause(60);
    assert.equal(await evaluate("document.querySelector('.floating-text b')"), null);
    assert(
      await evaluate(
        "document.querySelector('.floating-content').scrollHeight>document.querySelector('.floating-content').clientHeight",
      ),
    );
    await capture('r10-floating-text');
    content.item.kind = 'files';
    content.detail.text = '';
    content.detail.files = [{ path: 'C:/synthetic/missing.png', exists: false }];
    await evaluate(`window.emitEvent('compact-preview-changed',${JSON.stringify(content)})`);
    await pause(40);
    assert(
      await evaluate(
        "document.querySelector('.floating-files').textContent.includes('源文件已不存在')",
      ),
    );
    content.item.sensitive = true;
    content.item.thumbnail = 'synthetic-thumb';
    content.item.preview = 'synthetic-thumb';
    content.detail.files = [];
    content.detail.text = '合成敏感••••';
    await evaluate(`window.emitEvent('compact-preview-changed',${JSON.stringify(content)})`);
    await pause(40);
    assert.equal(await evaluate("document.querySelector('.floating-preview img')"), null);
    assert(
      await evaluate(
        "document.querySelector('.floating-preview header').textContent.includes('已打码')",
      ),
    );
    await evaluate("window.emitEvent('compact-preview-changed',null)");
    await pause(40);
    assert.equal(
      await evaluate("document.querySelector('.floating-text,.floating-files,img')"),
      null,
    );
  });
  await test('R10 浮窗迟到初始快照不覆盖更新或清空', async () => {
    await floatingNav('&holdpreview=1');
    await until(
      () => evaluate("typeof window.releasePreviewSnapshot==='function'"),
      '未挂起初始快照',
    );
    await evaluate("window.emitEvent('compact-preview-changed',null)");
    await appearance({ theme: 'dark' });
    await evaluate(`window.releasePreviewSnapshot(${JSON.stringify(floatingContent('late'))})`);
    await pause(80);
    assert.equal(await evaluate("document.querySelector('.floating-preview img')"), null);
    assert.equal(await evaluate('document.documentElement.dataset.theme'), 'dark');
  });

  for (const theme of ['light', 'dark'])
    await test('R12 ' + theme + ' 两行摘要、圆角按钮和详情复位', async () => {
      await metrics();
      await nav();
      await appearance({ theme });
      await evaluate(
        "window.rows[0].summary='单行长句自动换行验证'.repeat(35);window.rows[4].summary='const value = 42;\\nfunction add(a, b) { return a + b; }';window.emitEvent('history-changed')",
      );
      await pause(90);
      const size = await evaluate(
        "(()=>{const n=document.querySelector('[data-id=\"row-0\"]'),s=n.querySelector('.summary'),r=s.getBoundingClientRect(),c=n.querySelector('.card-status').getBoundingClientRect();return {height:r.height,line:parseFloat(getComputedStyle(s).lineHeight),bottom:c.bottom,rowBottom:n.getBoundingClientRect().bottom,white:getComputedStyle(s).whiteSpace,radius:getComputedStyle(document.querySelector('.paste-primary')).borderRadius}})()",
      );
      assert(size.height > size.line && size.height <= size.line * 2 + 1, JSON.stringify(size));
      assert(size.bottom <= size.rowBottom);
      assert.equal(size.white, 'pre-wrap');
      assert.equal(size.radius, '8px');
      await evaluate("window.rows[0].summary='Z'.repeat(600);window.emitEvent('history-changed')");
      await pause(50);
      assert(await evaluate('document.documentElement.scrollWidth<=innerWidth'));
      await evaluate(
        "document.querySelector('.detail-scroll').scrollTop=100;window.rows[0].summary='长文本';const original=window.__TAURI_INTERNALS__.invoke;window.__TAURI_INTERNALS__.invoke=(cmd,args)=>cmd==='item_detail'?Promise.resolve({text:'从正文开头开始\\n'+('正文内容'.repeat(2000)),files:[],truncated:false}):original(cmd,args);window.emitEvent('history-changed')",
      );
      await pause(90);
      await evaluate("document.querySelector('.detail-scroll').scrollTop=160");
      assert(await evaluate("document.querySelector('.detail-scroll').scrollTop>0"));
      await click('.preview-refresh');
      await pause(60);
      assert.equal(await evaluate("document.querySelector('.detail-scroll').scrollTop"), 0);
      await evaluate("document.querySelector('.detail-scroll').scrollTop=160");
      await click('[data-id="row-4"]');
      await pause(80);
      assert.equal(await evaluate("document.querySelector('.detail-scroll').scrollTop"), 0);
      assert(
        await evaluate(
          "document.querySelector('.text-detail').textContent.startsWith('从正文开头开始')",
        ),
      );
      await capture('r12-' + theme + '-list');
    });
  for (const theme of ['light', 'dark'])
    await test('R12 ' + theme + ' 紧凑对齐、状态、几何同步与表单', async () => {
      await metrics(390);
      await nav();
      await appearance({ theme, compact: true });
      await evaluate(
        "window.rows[4]={...window.rows[4],kind:'image',thumbnail:'synthetic-thumb'};window.rows[1].summary='极长文件名'.repeat(80);window.emitEvent('history-changed')",
      );
      await pause(90);
      const geometry = await evaluate(
        "[...document.querySelectorAll('.item')].slice(0,6).map(n=>{const r=n.getBoundingClientRect(),i=n.querySelector('.type-icon,.record-thumbnail').getBoundingClientRect(),c=n.querySelector('.content').getBoundingClientRect(),s=n.querySelector('.sensitive-badge,.file-status')?.getBoundingClientRect();return {h:r.height,cx:(i.left+i.right)/2,cy:(i.top+i.bottom)/2,y:(r.top+r.bottom)/2,left:c.left,status:s?.bottom,bottom:r.bottom}})",
      );
      assert(
        geometry.every(
          (r) =>
            r.h === 54 &&
            Math.abs(r.cx - geometry[0].cx) <= 1 &&
            Math.abs(r.cy - r.y) <= 1 &&
            r.left === geometry[0].left &&
            (!r.status || r.status <= r.bottom),
        ),
        JSON.stringify(geometry),
      );
      assert(
        await evaluate(
          "Math.abs(document.querySelector('.compact-groups select').getBoundingClientRect().left-document.querySelector('.kind-filters').getBoundingClientRect().left)<=1",
        ),
      );
      await click('[data-id="row-4"]');
      await pause(90);
      assert.equal(
        await evaluate(
          "window.calls.filter(c=>c.cmd==='position_compact_preview').at(-1)?.args.anchor?.id",
        ),
        'row-4',
      );
      const before = await evaluate(
        "window.calls.filter(c=>c.cmd==='sync_compact_preview').length",
      );
      await evaluate(
        "document.querySelector('.list').scrollTop=500;document.querySelector('.list').dispatchEvent(new Event('scroll'))",
      );
      await pause(90);
      assert.equal(
        await evaluate(
          "window.calls.filter(c=>c.cmd==='position_compact_preview').at(-1)?.args.anchor",
        ),
        null,
      );
      assert.equal(
        await evaluate("window.calls.filter(c=>c.cmd==='sync_compact_preview').length"),
        before,
      );
      await evaluate(
        "document.querySelector('.list').scrollTop=0;document.querySelector('.list').dispatchEvent(new Event('scroll'))",
      );
      await pause(60);
      await capture('r12-' + theme + '-compact');
      await click('.settings-toggle');
      await pause(60);
      await click('.settings-nav button:nth-child(2)').catch(() => {});
      const control = await evaluate(
        "(()=>{const n=document.querySelector('.settings-view input[type=checkbox]');const s=getComputedStyle(n);return {appearance:s.appearance,w:s.width,h:s.height,radius:s.borderRadius}})()",
      );
      assert.deepEqual(control, { appearance: 'none', w: '15px', h: '15px', radius: '4px' });
      await capture('r12-' + theme + '-settings');
    });

  await test('R12 高图片完整展示并在切换记录后回到顶部', async () => {
    await metrics();
    await nav();
    await evaluate(
      'const original=window.__TAURI_INTERNALS__.convertFileSrc;window.__TAURI_INTERNALS__.convertFileSrc=hash=>hash===\'tall\'?\'data:image/svg+xml,\'+encodeURIComponent(\'<svg xmlns="http://www.w3.org/2000/svg" width="160" height="800"><rect width="160" height="80" fill="#277d77"/><rect y="80" width="160" height="720" fill="#eaf4f2"/></svg>\'):original(hash);window.rows[2]={...window.rows[2],kind:\'image\',thumbnail:\'tall\',preview:\'tall\'};window.emitEvent(\'history-changed\')',
    );
    await pause(70);
    await click('[data-id="row-2"]');
    await until(
      () => evaluate("document.querySelector('.detail img')?.naturalHeight===800"),
      '高图未解码',
    );
    const box = await evaluate(
      "(()=>{const n=document.querySelector('.detail img'),r=n.getBoundingClientRect(),s=document.querySelector('.detail-scroll');return {w:r.width,h:r.height,scroll:s.scrollTop,total:s.scrollHeight,height:s.clientHeight,max:getComputedStyle(n).maxHeight}})()",
    );
    assert(Math.abs(box.h / box.w - 5) < 0.01, JSON.stringify(box));
    assert.equal(box.max, 'none');
    assert.equal(box.scroll, 0);
    assert(box.total > box.height);
    await evaluate("document.querySelector('.detail-scroll').scrollTop=500");
    await click('[data-id="row-4"]');
    await pause(50);
    await click('[data-id="row-2"]');
    await pause(70);
    assert.equal(await evaluate("document.querySelector('.detail-scroll').scrollTop"), 0);
    await capture('r12-tall-image');
  });
  await test('R12 浮窗同记录更新复位与双向箭头', async () => {
    await floatingNav();
    const content = floatingContent('same');
    content.item.kind = 'text';
    content.item.thumbnail = null;
    content.item.preview = null;
    content.detail.text = '正文起点\n' + '预览内容'.repeat(4000);
    await evaluate(`window.emitEvent('compact-preview-changed',${JSON.stringify(content)})`);
    await pause(60);
    await evaluate("document.querySelector('.floating-content').scrollTop=300");
    assert(await evaluate("document.querySelector('.floating-content').scrollTop>0"));
    await evaluate(
      `window.emitEvent('compact-preview-changed',${JSON.stringify(content)});window.emitEvent('compact-preview-geometry',{right:false,arrow:210})`,
    );
    await pause(70);
    assert.equal(await evaluate("document.querySelector('.floating-content').scrollTop"), 0);
    assert(
      await evaluate("document.querySelector('.floating-shell').classList.contains('on-left')"),
    );
    assert.equal(
      await evaluate("getComputedStyle(document.querySelector('.preview-arrow')).top"),
      '203px',
    );
    await capture('r12-floating-left');
    await evaluate("window.emitEvent('compact-preview-geometry',{right:true,arrow:40})");
    await pause(40);
    assert(
      await evaluate("document.querySelector('.floating-shell').classList.contains('on-right')"),
    );
    assert.equal(
      await evaluate("getComputedStyle(document.querySelector('.preview-arrow')).top"),
      '33px',
    );
  });

  for (const theme of ['light', 'dark'])
    await test('R13 ' + theme + ' 分组色跨侧栏标签详情一致且改名稳定', async () => {
      await metrics();
      await nav();
      await appearance({ theme });
      const colors = await evaluate(
        "(()=>{const side=document.querySelector('.colored-group'),dot=side.querySelector('.group-dot'),badge=document.querySelector('[data-id=\"row-0\"] .group-badge'),detail=document.querySelector('.detail-group');return {side:getComputedStyle(side).color,dot:getComputedStyle(dot).backgroundColor,badge:getComputedStyle(badge).color,detail:getComputedStyle(detail).color,fill:getComputedStyle(badge).backgroundColor,other:getComputedStyle(document.querySelectorAll('.colored-group')[1]).color}})()",
      );
      assert.equal(colors.side, colors.badge);
      assert.equal(colors.dot, colors.badge);
      assert.equal(colors.detail, colors.badge);
      assert.notEqual(colors.other, colors.badge);
      const contrast = (a, b) => {
        const lum = (s) => {
          const rgb = s
            .match(/[\d.]+/g)
            .slice(0, 3)
            .map(Number)
            .map((v) => v / 255)
            .map((v) => (v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4));
          return rgb[0] * 0.2126 + rgb[1] * 0.7152 + rgb[2] * 0.0722;
        };
        const x = lum(a),
          y = lum(b);
        return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05);
      };
      assert(contrast(colors.badge, colors.fill) >= 4.5, JSON.stringify(colors));
      await evaluate(
        "const original=window.__TAURI_INTERNALS__.invoke;window.__TAURI_INTERNALS__.invoke=(cmd,args)=>cmd==='list_groups'?Promise.resolve([{id:7,name:'改名后的工作分组',count:35},{id:8,name:'另一分组',count:2}]):original(cmd,args);window.emitEvent('history-changed')",
      );
      await pause(80);
      assert.equal(
        await evaluate("getComputedStyle(document.querySelector('.colored-group')).color"),
        colors.badge,
      );
      assert(
        await evaluate(
          "document.querySelector('[data-id=\"row-0\"] .group-badge').textContent.includes('改名后')",
        ),
      );
      await capture('r13-' + theme + '-groups');
      await appearance({ layout: 'cards' });
      assert.equal(
        await evaluate(
          'getComputedStyle(document.querySelector(\'[data-id="row-0"] .group-badge\')).color',
        ),
        colors.badge,
      );
      await appearance({ layout: 'list', compact: true });
      await change('.compact-groups select', '7');
      await pause(60);
      assert.equal(
        await evaluate("getComputedStyle(document.querySelector('.compact-groups select')).color"),
        colors.badge,
      );
      await metrics();
      await nav();
      await appearance({ theme });
      assert.equal(
        await evaluate("getComputedStyle(document.querySelector('.colored-group')).color"),
        colors.badge,
      );
    });
  await test('R13 透明滚动轨道与默认皮肤复选框保持键盘操作', async () => {
    await metrics();
    await nav();
    await click('.settings-toggle');
    await pause(70);
    const before = await evaluate(
      "(()=>{const n=document.querySelector('[data-category=general] input[type=checkbox]');n.focus();return {checked:n.checked,w:getComputedStyle(n).width,h:getComputedStyle(n).height,track:getComputedStyle(document.querySelector('.settings-content'),'::-webkit-scrollbar-track').backgroundColor,thumb:getComputedStyle(document.querySelector('.settings-content'),'::-webkit-scrollbar-thumb').backgroundClip}})()",
    );
    assert.equal(before.w, '15px');
    assert.equal(before.h, '15px');
    assert.equal(before.track, 'rgba(0, 0, 0, 0)');
    assert.equal(before.thumb, 'padding-box');
    await send('Input.dispatchKeyEvent', {
      type: 'keyDown',
      key: ' ',
      code: 'Space',
      windowsVirtualKeyCode: 32,
    });
    await send('Input.dispatchKeyEvent', {
      type: 'keyUp',
      key: ' ',
      code: 'Space',
      windowsVirtualKeyCode: 32,
    });
    assert.equal(await evaluate('document.activeElement.checked'), !before.checked);
    await capture('r13-checkbox-scrollbars');
  });

  const key = async (name, code, shift = false) => {
    await send('Input.dispatchKeyEvent', {
      type: 'keyDown',
      key: name,
      code: name,
      windowsVirtualKeyCode: code,
      modifiers: shift ? 8 : 0,
    });
    await send('Input.dispatchKeyEvent', {
      type: 'keyUp',
      key: name,
      code: name,
      windowsVirtualKeyCode: code,
      modifiers: shift ? 8 : 0,
    });
  };
  await test('预览按钮显示粘贴此条记录并提交选中的记录', async () => {
    await metrics();
    await nav();
    assert.equal(
      await evaluate("document.querySelector('.paste-primary').textContent.trim()"),
      '粘贴此条记录',
    );
    const id = await selected();
    await click('.paste-primary');
    assert.deepEqual(
      await evaluate("window.calls.filter(c=>c.cmd==='paste_item').map(c=>c.args)"),
      [{ id, plain: false }],
    );
  });
  await test('R14 分组焦点经方向键进入记录，Enter和Shift+Enter各粘贴一次', async () => {
    await metrics();
    await nav();
    await evaluate("const b=document.querySelector('.colored-group');b.click();b.focus()");
    await pause(90);
    assert(await evaluate("document.activeElement.classList.contains('colored-group')"));
    const before = await selected();
    await key('ArrowDown', 40);
    await pause(60);
    const after = await selected();
    assert.notEqual(after, before);
    assert.equal(await evaluate('document.activeElement.dataset.id'), after);
    await key('Enter', 13);
    await pause(60);
    assert.equal(await evaluate("window.calls.filter(c=>c.cmd==='paste_item').length"), 1);
    assert.deepEqual(await evaluate("window.calls.filter(c=>c.cmd==='paste_item').at(-1).args"), {
      id: after,
      plain: false,
    });
    await key('ArrowUp', 38);
    await pause(50);
    assert.equal(await evaluate('document.activeElement.dataset.id'), await selected());
    await key('Enter', 13, true);
    await pause(50);
    assert.equal(await evaluate("window.calls.filter(c=>c.cmd==='paste_item').length"), 2);
    assert.equal(
      await evaluate("window.calls.filter(c=>c.cmd==='paste_item').at(-1).args.plain"),
      true,
    );
    await evaluate("document.querySelector('.searchbox input').focus()");
    await key('ArrowDown', 40);
    await pause(40);
    assert(await evaluate("document.activeElement.matches('.item')"));
    await key('Enter', 13);
    await pause(50);
    assert.equal(await evaluate("window.calls.filter(c=>c.cmd==='paste_item').length"), 3);
    await click('.settings-toggle');
    await pause(40);
    await key('Enter', 13);
    assert.equal(await evaluate("window.calls.filter(c=>c.cmd==='paste_item').length"), 3);
  });

  await test('R14 紧凑分组提交后方向键和Enter继续操作记录，未提交时保留下拉行为', async () => {
    await metrics(390);
    await nav();
    await appearance({ compact: true });
    await evaluate("document.querySelector('.compact-groups select').focus()");
    // 未change时不劫持select的按键，也不发送粘贴。
    const prevented = await evaluate(
      "(()=>{const n=document.activeElement;return !n.dispatchEvent(new KeyboardEvent('keydown',{key:'Enter',bubbles:true,cancelable:true}))})()",
    );
    assert.equal(prevented, false);
    assert.equal(await evaluate("window.calls.filter(c=>c.cmd==='paste_item').length"), 0);
    await change('.compact-groups select', '7');
    await pause(90);
    assert(await evaluate("document.activeElement.matches('input[type=search]')"));
    const before = await selected();
    await key('ArrowDown', 40);
    await pause(40);
    const after = await selected();
    assert.notEqual(after, before);
    assert(await evaluate("document.activeElement.matches('.item')"));
    await key('Enter', 13);
    await pause(40);
    assert.deepEqual(
      await evaluate("window.calls.filter(c=>c.cmd==='paste_item').map(c=>c.args)"),
      [{ id: after, plain: false }],
    );
    await key('ArrowUp', 38);
    await pause(40);
    await key('Enter', 13, true);
    await pause(40);
    assert.deepEqual(await evaluate("window.calls.filter(c=>c.cmd==='paste_item').at(-1).args"), {
      id: before,
      plain: true,
    });
    // 点击记录后的焦点也可直接Enter；与常规模式共用同一处理。
    await evaluate("const n=document.querySelectorAll('.item')[1];n.click();n.focus()");
    const focused = await evaluate('document.activeElement.dataset.id');
    await key('Enter', 13);
    await pause(40);
    assert.equal(await evaluate("window.calls.filter(c=>c.cmd==='paste_item').length"), 3);
    assert.equal(
      await evaluate("window.calls.filter(c=>c.cmd==='paste_item').at(-1).args.id"),
      focused,
    );
    // 回到全部及未分组，同样完成焦点交接。
    for (const value of ['all', 'ungrouped']) {
      await evaluate("document.querySelector('.compact-groups select').focus()");
      await change('.compact-groups select', value);
      await pause(60);
      assert(await evaluate("document.activeElement.matches('input[type=search]')"));
    }
  });
  for (const theme of ['light', 'dark'])
    await test('R14 ' + theme + ' 浮窗透明边距、独立边框与阴影', async () => {
      await floatingNav();
      await appearance({ theme });
      await evaluate(
        `window.emitEvent('compact-preview-changed',${JSON.stringify(floatingContent())});window.emitEvent('compact-preview-geometry',{right:true,arrow:60})`,
      );
      await pause(60);
      const b = await evaluate(
        "(()=>{const n=document.querySelector('.floating-preview'),r=n.getBoundingClientRect(),s=getComputedStyle(n),root=getComputedStyle(document.documentElement),shell=getComputedStyle(document.querySelector('.floating-shell'));return {root:root.backgroundColor,shell:shell.backgroundColor,shadow:s.boxShadow,border:s.borderTopColor,w:s.borderTopWidth,left:r.left,top:r.top,right:innerWidth-r.right,bottom:innerHeight-r.bottom}})()",
      );
      assert.equal(b.root, 'rgba(0, 0, 0, 0)');
      assert.equal(b.shell, 'rgba(0, 0, 0, 0)');
      assert.notEqual(b.shadow, 'none');
      assert.equal(b.w, '1px');
      assert.notEqual(
        b.border,
        await evaluate(
          "getComputedStyle(document.querySelector('.floating-preview')).backgroundColor",
        ),
      );
      assert(b.left >= 18 && b.top >= 8 && b.right >= 8 && b.bottom >= 10, JSON.stringify(b));
      // 用对比底色模拟桌面，验证阴影有绘制空间；这不是原生WebView透明验收。
      await evaluate(
        "document.documentElement.style.setProperty('background','#b7bdc4','important')",
      );
      await capture('r14-' + theme + '-floating-shadow');
    });

  for (const theme of ['light', 'dark'])
    await test('R15 ' + theme + ' 模式切换后键盘选中只显示一层边框', async () => {
      await metrics();
      await nav();
      await appearance({ theme });
      await evaluate('const n=document.querySelector(\'[data-id="row-4"]\');n.click();n.focus()');
      for (const compact of [true, false, true]) {
        await metrics(compact ? 390 : 1000);
        await appearance({ compact });
        await key('ArrowDown', 40);
        await pause(50);
        const box = await evaluate(
          "(()=>{const n=document.activeElement,s=getComputedStyle(n);return {id:n.dataset.id,selected:n.classList.contains('selected'),visible:n.matches(':focus-visible'),outline:s.outlineStyle,border:s.borderTopWidth,shadow:s.boxShadow}})()",
        );
        assert.equal(box.id, await selected());
        assert(box.selected && box.visible, JSON.stringify(box));
        assert.equal(box.outline, 'none');
        assert.equal(box.border, '1px');
        assert.equal(box.shadow, 'none');
        await capture('r15-' + theme + '-' + (compact ? 'compact' : 'list') + '-focus');
      }
      await evaluate('document.querySelector(\'[data-id="row-0"]\').focus()');
      assert.notEqual(
        await evaluate('getComputedStyle(document.activeElement).outlineStyle'),
        'none',
      );
    });
  await test('R15 首条开关默认关闭，草稿取消与保存保持原设置协议', async () => {
    await metrics();
    await nav();
    assert.equal(await evaluate('window.settings.select_first_on_show'), false);
    const toggle =
      "(()=>{const label=[...document.querySelectorAll('.settings-view label.check')].find(n=>n.textContent.includes('每次唤出主窗口'));label.querySelector('input').click()})()";
    await click('.settings-toggle');
    await pause(40);
    await evaluate(toggle);
    await click('.settings-back');
    await pause(40);
    assert.equal(await evaluate('window.settings.select_first_on_show'), false);
    await click('.settings-toggle');
    await pause(40);
    await evaluate(toggle);
    await click('.settings-view button[type=submit]');
    await pause(50);
    assert.equal(await evaluate('window.settings.select_first_on_show'), true);
    await click('.settings-back');
  });
  for (const compact of [false, true])
    await test(
      'R15 ' + (compact ? '紧凑' : '普通') + ' 唤出选择首条、滚动归零、保留筛选及手动选择优先',
      async () => {
        await metrics(compact ? 390 : 1000);
        await nav();
        await appearance({ compact });
        await click('[data-id="row-12"]');
        await evaluate("window.emitEvent('main-shown')");
        await pause(80);
        assert.equal(await selected(), 'row-12');
        await appearance({ select_first_on_show: true });
        await evaluate(
          "window.rows.unshift({...window.rows[0],id:'new-head',summary:'隐藏期间新增的记录'});document.querySelector('.list').scrollTop=300;window.emitEvent('main-shown')",
        );
        await pause(100);
        assert.equal(await selected(), 'new-head');
        assert.equal(await evaluate("document.querySelector('.list').scrollTop"), 0);
        if (compact) await change('.compact-groups select', '7');
        else await click('.colored-group');
        await pause(70);
        await click('[data-id="row-12"]');
        await evaluate("window.emitEvent('main-shown')");
        await pause(80);
        assert.equal(await selected(), 'new-head');
        assert(
          await evaluate(
            "window.calls.filter(c=>c.cmd==='list_items').at(-1).args.filter.group_id===7",
          ),
        );
        // 刷新慢时用户自己选择的记录应保留，不能迟到后再次拉回顶部。
        await evaluate(
          "const original=window.__TAURI_INTERNALS__.invoke;window.holdShow=true;window.__TAURI_INTERNALS__.invoke=(cmd,args)=>cmd==='list_items'&&window.holdShow?new Promise(resolve=>window.releaseShow=()=>{window.holdShow=false;resolve(original(cmd,args))}):original(cmd,args);window.emitEvent('main-shown')",
        );
        await until(() => evaluate("typeof window.releaseShow==='function'"), '唤出刷新未挂起');
        await click('[data-id="row-12"]');
        await evaluate('window.releaseShow()');
        await pause(90);
        assert.equal(await selected(), 'row-12');
        await evaluate(
          "window.holdShow=true;window.releaseShow=undefined;window.emitEvent('main-shown')",
        );
        await until(
          () => evaluate("typeof window.releaseShow==='function'"),
          '第二次唤出刷新未挂起',
        );
        await key('ArrowDown', 40);
        await pause(40);
        const keyboardChoice = await selected();
        assert.notEqual(keyboardChoice, 'new-head');
        await evaluate('window.releaseShow()');
        await pause(80);
        assert.equal(await selected(), keyboardChoice);
        await appearance({ select_first_on_show: false });
        await evaluate("window.emitEvent('main-shown')");
        await pause(70);
        assert.equal(await selected(), keyboardChoice);
        await evaluate("window.rows=[];window.emitEvent('history-changed')");
        await pause(60);
        await appearance({ select_first_on_show: true });
        await evaluate("window.emitEvent('main-shown')");
        await pause(70);
        assert.equal(await selected(), undefined);
      },
    );

  await test('R16 浮窗按内容报告高度、图片解码重测、长内容有上限且可缩回', async () => {
    await floatingNav();
    const text = floatingContent('short');
    text.item = { ...text.item, kind: 'text', thumbnail: null, preview: null };
    text.detail.text = '一行短文本';
    await evaluate(`window.emitEvent('compact-preview-changed',${JSON.stringify(text)})`);
    await pause(100);
    const short = await evaluate(
      "window.calls.filter(c=>c.cmd==='resize_compact_preview').at(-1).args",
    );
    assert.equal(short.generation, 42);
    assert(short.height >= 120 && short.height < 230, JSON.stringify(short));
    const before = await evaluate(
      "window.calls.filter(c=>c.cmd==='resize_compact_preview').length",
    );
    await pause(100);
    assert.equal(
      await evaluate("window.calls.filter(c=>c.cmd==='resize_compact_preview').length"),
      before,
    );
    const long = {
      ...text,
      generation: 43,
      detail: { ...text.detail, text: '长内容\n'.repeat(500) },
    };
    await evaluate(`window.emitEvent('compact-preview-changed',${JSON.stringify(long)})`);
    await pause(100);
    assert(
      await evaluate(
        "window.calls.filter(c=>c.cmd==='resize_compact_preview').at(-1).args.height===640",
      ),
    );
    // 模拟原生应用已按上限限制视口后的实际CSS渲染。
    await send('Emulation.setDeviceMetricsOverride', {
      width: 360,
      height: 640,
      deviceScaleFactor: 1,
      mobile: false,
    });
    await pause(50);
    assert(
      await evaluate(
        "document.querySelector('.floating-content').scrollHeight>document.querySelector('.floating-content').clientHeight",
      ),
    );
    await evaluate(
      `window.emitEvent('compact-preview-changed',${JSON.stringify({ ...text, generation: 44 })})`,
    );
    await pause(100);
    assert(
      await evaluate(
        "window.calls.filter(c=>c.cmd==='resize_compact_preview').at(-1).args.height<230",
      ),
    );
    const image = floatingContent('adaptive-image');
    image.generation = 45;
    await evaluate(`window.emitEvent('compact-preview-changed',${JSON.stringify(image)})`);
    await until(
      () => evaluate("document.querySelector('.floating-preview img')?.naturalWidth===160"),
      '自适应图片未解码',
    );
    await pause(90);
    const height = await evaluate(
      "window.calls.filter(c=>c.cmd==='resize_compact_preview').at(-1).args.height",
    );
    assert(height > 200 && height < 420, JSON.stringify(height));
    await send('Emulation.setDeviceMetricsOverride', {
      width: 360,
      height,
      deviceScaleFactor: 1,
      mobile: false,
    });
    await pause(80);
    await capture('r16-adaptive-image');
    await evaluate("window.emitEvent('compact-preview-changed',null)");
    await pause(40);
    const cleared = await evaluate(
      "window.calls.filter(c=>c.cmd==='resize_compact_preview').length",
    );
    await pause(70);
    assert.equal(
      await evaluate("window.calls.filter(c=>c.cmd==='resize_compact_preview').length"),
      cleared,
    );
  });
  await test('R17 单文件底部次要打开、多文件逐项打开、失效和敏感保护、失败可见', async () => {
    await metrics();
    await nav();
    await evaluate(
      "window.rows[1]={...window.rows[1],files:['C:/synthetic/甲 乙,丙.txt','C:/synthetic/第二张.png'],file_status:'available'};const original=window.__TAURI_INTERNALS__.invoke;window.__TAURI_INTERNALS__.invoke=(cmd,args)=>cmd==='item_detail'?Promise.resolve({text:'',files:window.rows[1].files.map(path=>({path,exists:window.singleFileExists??true,is_directory:window.singleDirectory??false})),truncated:false}):original(cmd,args);window.emitEvent('history-changed')",
    );
    await pause(70);
    await click('[data-id="row-1"]');
    await pause(50);
    assert.equal(
      await evaluate("document.querySelectorAll('.file-detail .file-actions').length"),
      2,
    );
    await click('.file-detail li:nth-child(2) .file-actions button');
    await pause(40);
    assert.deepEqual(await evaluate("window.calls.filter(c=>c.cmd==='file_action').at(-1).args"), {
      id: 'row-1',
      index: 1,
      action: 'open',
    });
    assert.equal(
      await evaluate(
        "[...document.querySelectorAll('.file-detail .file-actions button')].filter(n=>n.textContent.includes('定位')).length",
      ),
      0,
    );
    assert.equal(
      await evaluate("document.querySelectorAll('.file-detail .file-actions button').length"),
      2,
    );
    assert.equal(await evaluate("!!document.querySelector('.single-file-actions')"), false);
    await capture('r17-files-detail');
    await evaluate(
      "window.rows[1]={...window.rows[1],files:['C:/synthetic/仅一个.txt']};window.emitEvent('history-changed')",
    );
    await pause(60);
    await click('[data-id="row-1"]');
    await until(
      () =>
        evaluate(
          "!!document.querySelector('.single-file-actions button')&&!document.querySelector('.single-file-actions button').disabled",
        ),
      '单文件底部入口未就绪',
    );
    assert.equal(
      await evaluate("document.querySelectorAll('.file-detail .file-actions').length"),
      0,
    );
    const footer = await evaluate(`(()=>{
      const open=document.querySelector('.single-file-actions button').getBoundingClientRect();
      const paste=document.querySelector('.paste-primary').getBoundingClientRect();
      return {right:open.left>=paste.right,sameRow:Math.abs(open.top-paste.top)<1,smaller:open.width<paste.width&&open.width<=84};
    })()`);
    assert(footer.right && footer.sameRow && footer.smaller, JSON.stringify(footer));
    await click('.single-file-actions button');
    await pause(40);
    assert.deepEqual(await evaluate("window.calls.filter(c=>c.cmd==='file_action').at(-1).args"), {
      id: 'row-1',
      index: 0,
      action: 'open',
    });
    await capture('r17-single-file-bottom');
    assert.equal(
      await evaluate("document.querySelector('.single-file-actions button').textContent.trim()"),
      '打开文件',
    );
    await evaluate(
      "window.singleDirectory=true;document.querySelector('.preview-refresh').click()",
    );
    await until(
      () =>
        evaluate(
          "document.querySelector('.single-file-actions button').textContent.trim()==='打开目录'",
        ),
      '目录按钮文案未更新',
    );
    assert.equal(
      await evaluate(
        "document.querySelector('.single-file-actions button').getAttribute('aria-label')",
      ),
      '打开目录',
    );
    await evaluate(
      "window.singleDirectory=false;document.querySelector('.preview-refresh').click()",
    );
    await until(
      () =>
        evaluate(
          "document.querySelector('.single-file-actions button').textContent.trim()==='打开文件'",
        ),
      '文件按钮文案未恢复',
    );

    await evaluate(
      "window.singleFileExists=false;document.querySelector('.preview-refresh').click()",
    );
    await until(
      () =>
        evaluate(
          "document.querySelector('.file-status')?.textContent.includes('已失效') && document.querySelector('.single-file-actions button')?.disabled===true",
        ),
      '失效单文件未禁用',
    );
    await evaluate(
      "window.rows[1]={...window.rows[1],sensitive:true};window.emitEvent('history-changed')",
    );
    await pause(80);
    await click('[data-id="row-1"]');
    await until(
      () => evaluate("!document.querySelector('.single-file-actions')"),
      '敏感文件仍显示底部打开',
    );
    await evaluate(
      "window.singleFileExists=true;window.rows[1]={...window.rows[1],sensitive:false};window.emitEvent('history-changed')",
    );
    await pause(80);
    await click('[data-id="row-1"]');
    await until(
      () => evaluate("document.querySelector('.single-file-actions button')?.disabled===false"),
      '单文件恢复后未启用',
    );
    const context = async (id) => {
      await evaluate(
        `document.querySelector('[data-id="${id}"]').dispatchEvent(new MouseEvent('contextmenu',{bubbles:true,cancelable:true,clientX:280,clientY:250}))`,
      );
      await pause(40);
    };
    await context('row-1');
    assert(
      await evaluate(
        "document.querySelector('.context-menu').textContent.includes('用默认应用打开')",
      ),
    );
    await evaluate(
      "[...document.querySelectorAll('.context-menu button')].find(n=>n.textContent==='在资源管理器中定位').click()",
    );
    await pause(50);
    assert.equal(
      await evaluate("window.calls.filter(c=>c.cmd==='file_action').at(-1).args.action"),
      'reveal',
    );
    await context('row-0');
    assert(
      !(await evaluate(
        "document.querySelector('.context-menu').textContent.includes('用默认应用打开')",
      )),
    );
    await key('Escape', 27);
    await evaluate(
      "const old=window.__TAURI_INTERNALS__.invoke;window.__TAURI_INTERNALS__.invoke=(cmd,args)=>cmd==='file_action'?Promise.reject('源文件已不存在'):old(cmd,args)",
    );
    await context('row-1');
    await evaluate(
      "[...document.querySelectorAll('.context-menu button')].find(n=>n.textContent==='用默认应用打开').click()",
    );
    await pause(60);
    assert(
      await evaluate(
        "document.querySelector('[role=alert]').textContent.includes('源文件已不存在')",
      ),
    );
  });

  for (const compact of [false, true])
    await test(
      'R17 ' + (compact ? '紧凑' : '普通') + ' 多文件菜单仅一个定位入口，不显示打开或文件子菜单',
      async () => {
        await metrics(compact ? 390 : 1000);
        await nav();
        await appearance({ compact });
        await evaluate(
          "window.rows[1].files=['C:/synthetic/第一.txt','C:/synthetic/第二.png'];window.rows[1].file_status='available';window.emitEvent('history-changed')",
        );
        await pause(70);
        await evaluate(
          "document.querySelector('[data-id=\"row-1\"]').dispatchEvent(new MouseEvent('contextmenu',{bubbles:true,cancelable:true,clientX:140,clientY:220}))",
        );
        await pause(40);
        assert.equal(await evaluate("document.querySelector('.file-submenu')"), null);
        const labels = await evaluate(
          "[...document.querySelectorAll('.context-menu button')].map(n=>n.textContent.trim())",
        );
        assert.equal(labels.filter((s) => s.includes('定位')).length, 1);
        assert(!labels.some((s) => s.includes('打开')));
        assert(
          !labels.some((s) => s.includes('第一.txt') || s.includes('第二.png')),
          JSON.stringify(labels),
        );
        assert(
          await evaluate(
            "(()=>{const r=document.querySelector('.context-menu').getBoundingClientRect();return r.left>=0&&r.right<=innerWidth&&r.top>=0&&r.bottom<=innerHeight})()",
          ),
        );
        await evaluate(
          "[...document.querySelectorAll('.context-menu button')].find(n=>n.textContent==='在资源管理器中定位').focus()",
        );
        await capture('r17-' + (compact ? 'compact' : 'list') + '-files-menu');
        await key('Enter', 13);
        await pause(50);
        assert.deepEqual(
          await evaluate("window.calls.filter(c=>c.cmd==='file_action').map(c=>c.args)"),
          [{ id: 'row-1', index: 0, action: 'reveal' }],
        );
        assert.equal(await evaluate("window.calls.filter(c=>c.cmd==='paste_item').length"), 0);
      },
    );

  await test('R19 唤出位置仅保留记忆和鼠标、草稿取消与保存跨布局保留', async () => {
    await metrics();
    await nav();
    assert.equal(await evaluate('window.settings.main_position'), 'remember');
    await click('.settings-toggle');
    await pause(40);
    assert.deepEqual(
      await evaluate(
        "[...document.querySelector('select[aria-label=主窗口唤出位置]').options].map(n=>n.value)",
      ),
      ['remember', 'mouse'],
    );
    await change('select[aria-label=主窗口唤出位置]', 'mouse');
    await click('.settings-back');
    await pause(40);
    assert.equal(await evaluate('window.settings.main_position'), 'remember');
    for (const mode of ['mouse', 'remember']) {
      await click('.settings-toggle');
      await pause(40);
      await change('select[aria-label=主窗口唤出位置]', mode);
      await click('.settings-view button[type=submit]');
      await pause(50);
      assert.equal(await evaluate('window.settings.main_position'), mode);
      await click('.settings-back');
      await pause(30);
      await appearance({ compact: true });
      await click('.settings-toggle');
      await pause(40);
      assert.equal(
        await evaluate("document.querySelector('select[aria-label=主窗口唤出位置]').value"),
        mode,
      );
      await click('.settings-back');
      await appearance({ compact: false });
    }
  });

  await test('R21 保留快捷键录制和保存，移除合并入口', async () => {
    await metrics();
    await nav();
    await click('.settings-toggle');
    await pause(60);
    await click('.settings-nav button:nth-child(5)');
    const button = () =>
      evaluate("document.querySelector('[data-hotkey-record=main_hotkey]').click()");
    await button();
    await pause(40);
    assert.equal(
      await evaluate("window.calls.filter(c=>c.cmd==='set_hotkey_recording').at(-1).args.active"),
      true,
    );
    await evaluate("window.emitEvent('hotkey-recorded',{session:1,hotkey:null})");
    await pause(50);
    assert.equal(
      await evaluate("document.querySelector('select[aria-label=主窗口快捷键]').value"),
      'alt_v',
    );
    await button();
    await pause(30);
    await evaluate("window.emitEvent('hotkey-recorded',{session:1,hotkey:'custom:1:67'})");
    await pause(40);
    assert.equal(
      await evaluate("document.querySelector('select[aria-label=主窗口快捷键]').value"),
      'alt_v',
    );
    assert(
      await evaluate(
        "document.querySelector('[data-hotkey-record=main_hotkey]').title.includes('保留给系统')",
      ),
    );
    await button();
    await pause(30);
    await evaluate("window.emitEvent('hotkey-recorded',{session:0,hotkey:'custom:4:88'})");
    await pause(30);
    assert.equal(
      await evaluate("document.querySelector('select[aria-label=主窗口快捷键]').value"),
      'alt_v',
    );
    await evaluate("window.emitEvent('hotkey-recorded',{session:1,hotkey:'custom:4:67'})");
    await pause(50);
    assert.equal(
      await evaluate(
        "document.querySelector('select[aria-label=主窗口快捷键]').selectedOptions[0].textContent",
      ),
      'Alt + C',
    );
    assert.equal(
      await evaluate("document.querySelector('.settings-view').textContent.includes('合并主窗口')"),
      false,
    );
    assert.equal(
      await evaluate("document.querySelector('select[aria-label=快速面板快捷键]').disabled"),
      false,
    );
    await pause(350);
    assert.equal(await evaluate("!!document.querySelector('.hotkey-check')"), false);
    await click('.settings-view button[type=submit]');
    await pause(70);
    assert.equal(await evaluate('window.settings.main_hotkey'), 'custom:4:67');
    assert.equal(await evaluate("'unified_hotkey' in window.settings"), false);
    await click('.settings-back');
    assert(
      await evaluate("document.querySelector('header .brand p').textContent.includes('Alt + C')"),
    );
  });
  await test('K2.4 忽略内容规则限制、正则校验与保存', async () => {
    await metrics();
    await nav();
    await click('.settings-toggle');
    await pause(50);
    await click('.settings-nav button:nth-child(4)');
    await click('button.add-content-ignore-rule');
    assert(await evaluate("!!document.querySelector('.settings-feedback .error')"));
    const input =
      "[...document.querySelectorAll('[data-category=privacy] input')].find(node=>node.getAttribute('aria-label')==='第 1 条规则内容')";
    await evaluate(
      `(()=>{const control=${input};control.value='secret';control.dispatchEvent(new Event('input',{bubbles:true}))})()`,
    );
    await click('.settings-view button[type=submit]');
    await pause(50);
    assert.deepEqual(await evaluate('window.settings.content_ignore_rules'), [
      { mode: 'keyword', value: 'secret' },
    ]);
    await click('.settings-back');
    await click('.settings-toggle');
    await pause(50);
    await click('.settings-nav button:nth-child(4)');
    await evaluate(
      "(()=>{const mode=document.querySelector('[aria-label=\"第 1 条规则类型\"]');mode.value='regex';mode.dispatchEvent(new Event('change',{bubbles:true}));const input=document.querySelector('[aria-label=\"第 1 条规则内容\"]');input.value='(';input.dispatchEvent(new Event('input',{bubbles:true}))})()",
    );
    assert(
      await evaluate(
        "document.querySelector('.settings-feedback .error')?.textContent.includes('正则表达式无效')",
      ),
    );
  });
  await test('Fluent 错误、确认与状态提示使用主题化信息条样式', async () => {
    await metrics();
    await nav();
    await appearance({ skin: 'fluent', theme: 'light' });
    await click('.settings-toggle');
    await until(
      () => evaluate("!!document.querySelector('.settings-feedback')"),
      'Fluent 设置未打开',
    );
    for (const theme of ['light', 'dark']) {
      await appearance({ skin: 'fluent', theme });
      await evaluate(`(()=>{
        document.querySelector('#fluent-message-fixtures')?.remove();
        const main=document.querySelector('.main-window');
        main.insertAdjacentHTML('beforeend', '<div id="fluent-message-fixtures" style="position:fixed;z-index:10000;inset:132px 16px auto 172px;display:grid;gap:8px"><div id="fluent-alert" class="error" role="alert">示例错误提示</div><div id="fluent-confirm" class="delete-confirm" role="alertdialog"><span>确认删除示例记录？</span><button class="confirm-delete">确认</button><button class="cancel-delete">取消</button></div></div>');
        document.querySelector('.settings-feedback').insertAdjacentHTML('beforeend','<p id="fluent-status" role="status">设置已保存</p>');
      })()`);
      const styles = await evaluate(`(()=>{
        const get=(selector,pseudo)=>getComputedStyle(document.querySelector(selector),pseudo);
        const alert=get('#fluent-alert');
        const confirm=get('#fluent-confirm');
        const status=get('#fluent-status');
        return {
          alertBackground:alert.backgroundColor,
          alertBorder:alert.borderInlineStartColor,
          alertIcon:get('#fluent-alert','::before').content,
          confirmBackground:confirm.backgroundColor,
          confirmAction:get('#fluent-confirm .confirm-delete').backgroundColor,
          cancelAction:get('#fluent-confirm .cancel-delete').backgroundColor,
          statusBackground:status.backgroundColor,
          statusIcon:get('#fluent-status','::before').content,
          radius:confirm.borderRadius
        };
      })()`);
      assert.notEqual(
        styles.alertBackground,
        await evaluate(
          "getComputedStyle(document.documentElement).getPropertyValue('--error-bg').trim()",
        ),
      );
      assert.equal(styles.alertIcon, '"!"');
      assert.equal(styles.statusIcon, '"i"');
      assert.notEqual(styles.confirmAction, styles.cancelAction);
      assert.equal(styles.radius, '4px');
      if (theme === 'dark') await capture('fluent-message-dark');
      await evaluate("document.querySelector('#fluent-status')?.remove()");
    }
    await evaluate("document.querySelector('#fluent-message-fixtures')?.remove()");
  });
  await test('R19 启动录制的迟到响应在退出设置后再次清理，不遗留录制', async () => {
    await metrics();
    await nav();
    await click('.settings-toggle');
    await pause(60);
    await click('.settings-nav button:nth-child(5)');
    await evaluate(
      "window.holdRecording=true;document.querySelector('[data-hotkey-record=main_hotkey]').click()",
    );
    await pause(30);
    assert.equal(
      await evaluate(
        "window.calls.filter(c=>c.cmd==='set_hotkey_recording'&&c.args.active).length",
      ),
      1,
    );
    await click('.settings-back');
    await pause(30);
    const stopped = await evaluate(
      "window.calls.filter(c=>c.cmd==='set_hotkey_recording'&&!c.args.active).length",
    );
    await evaluate('window.releaseRecording()');
    await pause(50);
    assert(
      (await evaluate(
        "window.calls.filter(c=>c.cmd==='set_hotkey_recording'&&!c.args.active).length",
      )) > stopped,
    );
    assert.equal(
      await evaluate("window.calls.filter(c=>c.cmd==='set_hotkey_recording').at(-1).args.active"),
      false,
    );
    await click('.settings-toggle');
    await pause(50);
    assert.equal(
      await evaluate("document.querySelector('select[aria-label=主窗口快捷键]').value"),
      'alt_v',
    );
  });
  for (const compact of [false, true])
    await test(
      'R20 ' + (compact ? '紧凑' : '普通') + ' 实际键盘事件录制与Esc只取消当前录制',
      async () => {
        await metrics(compact ? 390 : 1000);
        await nav();
        await appearance({ compact });
        await click('.settings-toggle');
        await pause(60);
        await click('.settings-nav button:nth-child(5)');
        const begin = () =>
          evaluate("document.querySelector('[data-hotkey-record=main_hotkey]').click()");
        for (const chord of [
          { key: 'c', code: 'KeyC', vk: 67, mods: 1, value: 'custom:4:67' },
          { key: 'C', code: 'KeyC', vk: 67, mods: 10, value: 'custom:3:67' },
        ]) {
          await begin();
          await pause(30);
          await send('Input.dispatchKeyEvent', {
            type: 'keyDown',
            key: chord.key,
            code: chord.code,
            windowsVirtualKeyCode: chord.vk,
            modifiers: chord.mods,
          });
          await send('Input.dispatchKeyEvent', {
            type: 'keyUp',
            key: chord.key,
            code: chord.code,
            windowsVirtualKeyCode: chord.vk,
            modifiers: chord.mods,
          });
          await pause(50);
          assert.equal(
            await evaluate("document.querySelector('select[aria-label=主窗口快捷键]').value"),
            chord.value,
          );
          assert.equal(await evaluate('window.settings.main_hotkey'), 'alt_v');
          assert.equal(await evaluate('window.settingUpdates.length'), 0);
          await evaluate("window.emitEvent('hotkey-recorded',{session:1,hotkey:'custom:4:88'})");
          await pause(30);
          assert.equal(
            await evaluate("document.querySelector('select[aria-label=主窗口快捷键]').value"),
            chord.value,
          );
        }
        await begin();
        await pause(30);
        await key('Escape', 27);
        await pause(40);
        assert(await evaluate("!!document.querySelector('.settings-view')"));
        assert.equal(
          await evaluate("document.querySelector('select[aria-label=主窗口快捷键]').value"),
          'custom:3:67',
        );
        assert.equal(
          await evaluate(
            "window.calls.filter(c=>c.cmd==='set_hotkey_recording').at(-1).args.active",
          ),
          false,
        );
        assert.equal(await evaluate("window.calls.filter(c=>c.cmd==='paste_item').length"), 0);
        await key('Escape', 27);
        await pause(40);
        assert.equal(await evaluate("!!document.querySelector('.settings-view')"), false);
      },
    );
  await test('R20 底部取消与行内取消均只停止录制，保留设置草稿', async () => {
    await metrics();
    await nav();
    await click('.settings-toggle');
    await pause(60);
    await click('.settings-nav button:nth-child(5)');
    const begin = () =>
      evaluate("document.querySelector('[data-hotkey-record=main_hotkey]').click()");
    await change('select[aria-label=主窗口快捷键]', 'ctrl_alt_v');
    for (const selector of [
      "document.querySelector('.settings-cancel')",
      "[...document.querySelectorAll('.settings-content button')].find(n=>n.textContent==='取消录制')",
    ]) {
      await begin();
      await pause(30);
      await evaluate(selector + '.click()');
      await pause(40);
      assert(await evaluate("!!document.querySelector('.settings-view')"));
      assert.equal(
        await evaluate("document.querySelector('select[aria-label=主窗口快捷键]').value"),
        'ctrl_alt_v',
      );
    }
    await click('.settings-cancel');
    await pause(40);
    assert.equal(await evaluate("!!document.querySelector('.settings-view')"), false);
    assert.equal(await evaluate('window.settings.main_hotkey'), 'alt_v');
  });
  await test('R20 启动响应在途时Esc先取消录制，迟到响应再次清理', async () => {
    await metrics();
    await nav();
    await click('.settings-toggle');
    await pause(60);
    await click('.settings-nav button:nth-child(5)');
    await evaluate(
      "window.holdRecording=true;document.querySelector('[data-hotkey-record=main_hotkey]').click()",
    );
    await pause(30);
    await key('Escape', 27);
    await pause(30);
    assert(await evaluate("!!document.querySelector('.settings-view')"));
    const stopped = await evaluate(
      "window.calls.filter(c=>c.cmd==='set_hotkey_recording'&&!c.args.active).length",
    );
    await evaluate('window.releaseRecording()');
    await pause(40);
    assert(
      (await evaluate(
        "window.calls.filter(c=>c.cmd==='set_hotkey_recording'&&!c.args.active).length",
      )) > stopped,
    );
    assert.equal(await evaluate('window.settingUpdates.length'), 0);
  });
  await test('R20 录制期间Enter和Space不激活按钮或退出设置', async () => {
    await metrics();
    await nav();
    await click('.settings-toggle');
    await pause(60);
    await click('.settings-nav button:nth-child(5)');
    for (const chord of [
      { key: 'Enter', code: 'Enter', vk: 13 },
      { key: ' ', code: 'Space', vk: 32 },
    ]) {
      await evaluate("document.querySelector('[data-hotkey-record=main_hotkey]').click()");
      await pause(30);
      await evaluate("document.querySelector('.settings-cancel').focus()");
      await send('Input.dispatchKeyEvent', {
        type: 'keyDown',
        key: chord.key,
        code: chord.code,
        windowsVirtualKeyCode: chord.vk,
      });
      await send('Input.dispatchKeyEvent', {
        type: 'keyUp',
        key: chord.key,
        code: chord.code,
        windowsVirtualKeyCode: chord.vk,
      });
      await pause(40);
      assert(await evaluate("!!document.querySelector('.settings-view')"));
      assert.equal(await evaluate('window.settingUpdates.length'), 0);
      assert(
        await evaluate(
          "!!document.querySelector('[data-hotkey-record=main_hotkey][aria-invalid=true]')",
        ),
      );
    }
  });
  await test('R20 原生录制结果早于启动响应时按会话缓冲，不被迟到旧会话覆盖', async () => {
    await metrics();
    await nav();
    await click('.settings-toggle');
    await pause(60);
    await click('.settings-nav button:nth-child(5)');
    await evaluate(
      "window.holdRecording=true;document.querySelector('[data-hotkey-record=main_hotkey]').click()",
    );
    await pause(30);
    await evaluate(
      "window.emitEvent('hotkey-recorded',{session:1,hotkey:'custom:4:67'});window.emitEvent('hotkey-recorded',{session:0,hotkey:'custom:4:88'})",
    );
    await pause(30);
    assert.equal(
      await evaluate("document.querySelector('select[aria-label=主窗口快捷键]').value"),
      'alt_v',
    );
    await evaluate('window.releaseRecording()');
    await pause(50);
    assert.equal(
      await evaluate("document.querySelector('select[aria-label=主窗口快捷键]').value"),
      'custom:4:67',
    );
    assert.equal(
      await evaluate("window.calls.filter(c=>c.cmd==='set_hotkey_recording').at(-1).args.active"),
      false,
    );
    assert.equal(await evaluate('window.settingUpdates.length'), 0);
    assert(await evaluate("!!document.querySelector('.settings-view')"));
  });
  await test('R22 更改组合后自动检测，已知冲突在对应行提示并阻止保存', async () => {
    await metrics();
    await nav();
    await click('.settings-toggle');
    await pause(60);
    await click('.settings-nav button:nth-child(5)');
    await evaluate(
      "window.hotkeyCheckResult={main:{status:'conflict',message:'此组合已被系统或其他程序注册'},quick:{status:'available',message:'未发现系统注册冲突'}}",
    );
    await change('select[aria-label=主窗口快捷键]', 'ctrl_alt_v');
    await pause(400);
    assert.equal(
      await evaluate("document.querySelector('[data-hotkey-warning=main_hotkey]').textContent"),
      '检测到快捷键冲突',
    );
    assert(
      await evaluate(
        "document.querySelector('[data-hotkey-warning=main_hotkey]').title.includes('其他程序注册')",
      ),
    );
    assert.equal(
      await evaluate("!!document.querySelector('[data-hotkey-warning=quick_hotkey]')"),
      false,
    );
    assert(await evaluate("document.querySelector('.settings-view button[type=submit]').disabled"));
    assert.equal(await evaluate('window.settingUpdates.length'), 0);
    await evaluate('window.hotkeyCheckResult=undefined');
    await change('select[aria-label=主窗口快捷键]', 'win_alt_v');
    await pause(350);
    assert.equal(
      await evaluate("document.querySelector('.settings-view button[type=submit]').disabled"),
      false,
    );
    await click('.settings-view button[type=submit]');
    await pause(50);
    assert.equal(await evaluate('window.settings.main_hotkey'), 'win_alt_v');
    assert(await evaluate('window.hotkeyChecks.length>=3'));
  });
  await test('R21 检测失败可重试，保存时复检，失败不写设置', async () => {
    await metrics();
    await nav();
    await click('.settings-toggle');
    await pause(60);
    await click('.settings-nav button:nth-child(5)');
    await change('select[aria-label=主窗口快捷键]', 'ctrl_alt_v');
    await pause(350);
    await evaluate('window.failHotkeyCheck=true');
    await click('.settings-view button[type=submit]');
    await pause(50);
    assert.equal(await evaluate('window.settingUpdates.length'), 0);
    assert(
      await evaluate(
        "[...document.querySelectorAll('[data-hotkey-warning]')].length===2&&[...document.querySelectorAll('[data-hotkey-warning]')].every(n=>n.textContent==='检测失败'&&n.title.includes('合成检测失败'))",
      ),
    );
    await evaluate('window.failHotkeyCheck=false');
    await click('.settings-view button[type=submit]');
    await pause(50);
    assert.equal(await evaluate('window.settings.main_hotkey'), 'ctrl_alt_v');
  });
  await test('R21 检测迟到结果不能覆盖新快捷键的结果', async () => {
    await metrics();
    await nav();
    await click('.settings-toggle');
    await pause(60);
    await click('.settings-nav button:nth-child(5)');
    await evaluate('window.holdHotkeyCheck=true');
    await change('select[aria-label=主窗口快捷键]', 'ctrl_alt_v');
    await pause(350);
    assert(await evaluate("typeof window.releaseHotkeyCheck==='function'"));
    await evaluate('window.holdHotkeyCheck=false');
    await change('select[aria-label=主窗口快捷键]', 'win_alt_v');
    await pause(350);
    await evaluate(
      "window.releaseHotkeyCheck({main:{status:'conflict',message:'旧组合占用'},quick:{status:'available',message:'旧结果'}})",
    );
    await pause(40);
    assert.equal(await evaluate("document.querySelectorAll('[data-hotkey-warning]').length"), 0);
    assert.equal(
      await evaluate("document.querySelector('.settings-view button[type=submit]').disabled"),
      false,
    );
  });
  await test('R21 相同修饰键与系统保留F12在本地拒绝，不提交检测', async () => {
    await metrics();
    await nav();
    await click('.settings-toggle');
    await pause(60);
    await click('.settings-nav button:nth-child(5)');
    await evaluate("document.querySelector('[data-hotkey-record=main_hotkey]').click()");
    await pause(30);
    await evaluate("window.emitEvent('hotkey-recorded',{session:1,hotkey:'custom:4:123'})");
    await pause(40);
    assert(
      await evaluate(
        "document.querySelector('[data-hotkey-record=main_hotkey]').title.includes('保留给系统')",
      ),
    );
    assert.equal(
      await evaluate("document.querySelector('select[aria-label=主窗口快捷键]').value"),
      'alt_v',
    );
    await evaluate("document.querySelector('[data-hotkey-record=quick_hotkey]').click()");
    await pause(30);
    await evaluate("window.emitEvent('hotkey-recorded',{session:1,hotkey:'custom:4:67'})");
    await pause(40);
    const count = await evaluate('window.hotkeyChecks.length');
    await pause(350);
    assert.equal(await evaluate('window.hotkeyChecks.length'), count);
    assert(
      await evaluate(
        "document.querySelectorAll('[data-hotkey-warning]').length===2&&[...document.querySelectorAll('[data-hotkey-warning]')].every(n=>n.textContent==='检测到快捷键冲突'&&n.title.includes('相同的修饰键'))",
      ),
    );
    assert(await evaluate("document.querySelector('.settings-view button[type=submit]').disabled"));
  });
  await test('R22 打开设置检测一次，改键再次检测，保存其他设置及取消录制不重复检测', async () => {
    await metrics();
    await nav();
    await click('.settings-toggle');
    await pause(400);
    assert.equal(await evaluate('window.hotkeyChecks.length'), 1);
    await click('.settings-nav button:nth-child(5)');
    await pause(350);
    assert.equal(await evaluate('window.hotkeyChecks.length'), 1);
    assert.equal(
      await evaluate(
        "!!document.querySelector('.hotkey-check')||document.querySelector('.settings-view').textContent.includes('检测冲突')||document.querySelector('.settings-view').textContent.includes('未发现系统注册冲突')||document.querySelector('.settings-view').textContent.includes('仅检查系统登记')",
      ),
      false,
    );
    await evaluate(
      "const n=document.querySelector('[data-category=hotkeys] input[type=number]');n.value='6';n.dispatchEvent(new Event('input',{bubbles:true}))",
    );
    await click('.settings-view button[type=submit]');
    await pause(350);
    assert.equal(await evaluate('window.hotkeyChecks.length'), 1);
    await click('.settings-back');
    await click('.settings-toggle');
    await pause(350);
    assert.equal(await evaluate('window.hotkeyChecks.length'), 2);
    await click('.settings-nav button:nth-child(5)');
    await evaluate("document.querySelector('[data-hotkey-record=main_hotkey]').click()");
    await pause(30);
    await evaluate("window.emitEvent('hotkey-recorded',{session:1,hotkey:'custom:4:67'})");
    await pause(350);
    assert.equal(await evaluate('window.hotkeyChecks.length'), 3);
    assert.equal(await evaluate('window.hotkeyChecks.at(-1).mainHotkey'), 'custom:4:67');
    await evaluate("document.querySelector('[data-hotkey-record=main_hotkey]').click()");
    await pause(30);
    await key('Escape', 27);
    await pause(350);
    assert.equal(await evaluate('window.hotkeyChecks.length'), 3);
    await evaluate("document.querySelector('[data-hotkey-record=main_hotkey]').click()");
    await pause(30);
    await evaluate("window.emitEvent('hotkey-recorded',{session:1,hotkey:'custom:4:67'})");
    await pause(350);
    assert.equal(await evaluate('window.hotkeyChecks.length'), 4);
  });
  await test('R23 两主题普通与紧凑下，冲突直接显示在红色录制按钮中且无溢出', async () => {
    for (const theme of ['light', 'dark'])
      for (const compact of [false, true]) {
        await metrics(compact ? 390 : 1000);
        await nav();
        await appearance({ theme, compact });
        await evaluate(
          "window.hotkeyCheckResult={main:{status:'owned',message:'当前由本应用注册'},quick:{status:'conflict',message:'此组合已被系统或其他程序注册'}}",
        );
        await click('.settings-toggle');
        await pause(60);
        if (compact) await change('select[aria-label=设置分类]', 'hotkeys');
        else await click('.settings-nav button:nth-child(5)');
        await until(
          () => evaluate("!!document.querySelector('[data-hotkey-warning=quick_hotkey]')"),
          '冲突行提示缺失',
        );
        assert.equal(
          await evaluate("!!document.querySelector('[data-hotkey-warning=main_hotkey]')"),
          false,
        );
        assert(
          await evaluate(
            "(()=>{const n=document.querySelector('[data-hotkey-warning=quick_hotkey]'),r=n.getBoundingClientRect(),c=getComputedStyle(n).color.match(/\\d+/g).map(Number),bg=getComputedStyle(n).backgroundColor.match(/\\d+/g).map(Number);return n.tagName==='BUTTON'&&n.classList.contains('hotkey-problem')&&n.textContent==='检测到快捷键冲突'&&n.scrollWidth<=n.clientWidth+1&&r.right<=innerWidth&&c[0]>c[1]&&c[0]>c[2]&&bg[0]>bg[1]&&bg[0]>bg[2]&&document.documentElement.scrollWidth<=innerWidth})()",
          ),
        );
        if (!compact && theme === 'light') await capture('r23-conflict-button');
        if (compact && theme === 'dark') await capture('r23-compact-dark-conflict-button');
      }
  });
  await test('R23 非法录制错误在恢复预设、重选原组合和成功保存后清除', async () => {
    await metrics();
    await nav();
    await click('.settings-toggle');
    await pause(60);
    await click('.settings-nav button:nth-child(5)');
    const invalid = async (field) => {
      await evaluate(
        "(()=>{const b=document.querySelector('[data-hotkey-record=" +
          field +
          "]');b.focus();b.click()})()",
      );
      await pause(30);
      await evaluate("window.emitEvent('hotkey-recorded',{session:1,hotkey:'custom:1:67'})");
      await pause(40);
      assert.equal(
        await evaluate("document.querySelector('[data-hotkey-record=" + field + "]').textContent"),
        '检测到快捷键冲突',
      );
    };
    await invalid('main_hotkey');
    assert.equal(
      await evaluate("!!document.querySelector('[data-category=hotkeys] .error')"),
      false,
    );
    assert.equal(
      await evaluate("document.querySelector('select[aria-label=主窗口快捷键]').value"),
      'alt_v',
    );
    await change('select[aria-label=主窗口快捷键]', 'ctrl_alt_v');
    await pause(350);
    assert.equal(
      await evaluate("document.querySelector('[data-hotkey-record=main_hotkey]').textContent"),
      '录制主窗口快捷键',
    );
    await invalid('main_hotkey');
    await change('select[aria-label=主窗口快捷键]', 'ctrl_alt_v');
    await pause(350);
    assert.equal(
      await evaluate("document.querySelectorAll('.hotkey-problem').length"),
      0,
      '重选相同预设应清除录制错误',
    );
    await invalid('main_hotkey');
    await send('Emulation.setFocusEmulationEnabled', { enabled: true });
    try {
      await evaluate("document.querySelector('select[aria-label=主窗口快捷键]').focus()");
      await pause(30);
      assert.equal(
        await evaluate("document.querySelectorAll('.hotkey-problem').length"),
        0,
        '聚焦当前预设应清除旧录制错误',
      );
    } finally {
      await send('Emulation.setFocusEmulationEnabled', { enabled: false });
    }
    await invalid('quick_hotkey');
    assert.equal(
      await evaluate("document.querySelector('[data-hotkey-record=main_hotkey]').textContent"),
      '录制主窗口快捷键',
    );
    await click('.settings-view button[type=submit]');
    await pause(80);
    assert.equal(await evaluate('window.settings.main_hotkey'), 'ctrl_alt_v');
    assert.equal(
      await evaluate("document.querySelectorAll('.hotkey-problem').length"),
      0,
      '成功保存应清除各行录制错误',
    );
    assert.equal(
      await evaluate("!!document.querySelector('[data-category=hotkeys] .error')"),
      false,
    );
  });
  await test('R23 当前占用组合改回原组合后按钮恢复，冲突按钮仍可直接录制', async () => {
    await metrics();
    await nav();
    await click('.settings-toggle');
    await pause(60);
    await click('.settings-nav button:nth-child(5)');
    await evaluate(
      "window.hotkeyCheckResult={main:{status:'conflict',message:'合成系统占用'},quick:{status:'available',message:'可注册'}}",
    );
    await change('select[aria-label=主窗口快捷键]', 'ctrl_alt_v');
    await pause(350);
    assert.equal(
      await evaluate("document.querySelector('[data-hotkey-record=main_hotkey]').textContent"),
      '检测到快捷键冲突',
    );
    await click('[data-hotkey-record=main_hotkey]');
    await pause(30);
    assert.equal(
      await evaluate("document.querySelector('[data-hotkey-record=main_hotkey]').textContent"),
      '取消录制',
    );
    await key('Escape', 27);
    await pause(30);
    await evaluate('window.hotkeyCheckResult=undefined');
    await change('select[aria-label=主窗口快捷键]', 'alt_v');
    await pause(350);
    assert.equal(
      await evaluate("document.querySelector('[data-hotkey-record=main_hotkey]').textContent"),
      '录制主窗口快捷键',
    );
    assert.equal(await evaluate("document.querySelectorAll('.hotkey-problem').length"), 0);
    assert.equal(
      await evaluate("document.querySelector('.settings-view button[type=submit]').disabled"),
      false,
    );
  });
  await test('R23 录制启动或停止失败归属本行，不误报冲突或写入迟到结果', async () => {
    await metrics();
    await nav();
    await click('.settings-toggle');
    await pause(60);
    await click('.settings-nav button:nth-child(5)');
    await evaluate(
      "window.recordInvoke=window.__TAURI_INTERNALS__.invoke;window.failStart=true;window.failStop=false;window.__TAURI_INTERNALS__.invoke=(cmd,args)=>cmd==='set_hotkey_recording'&&((args.active&&window.failStart)||(!args.active&&window.failStop))?Promise.reject('合成录制通信失败'):window.recordInvoke(cmd,args)",
    );
    await click('[data-hotkey-record=main_hotkey]');
    await pause(40);
    assert.equal(
      await evaluate("document.querySelector('[data-hotkey-record=main_hotkey]').textContent"),
      '录制失败',
    );
    assert(
      await evaluate(
        "document.querySelector('[data-hotkey-record=main_hotkey]').title.includes('合成录制通信失败')",
      ),
    );
    await evaluate('window.failStart=false;window.failStop=true');
    await click('[data-hotkey-record=main_hotkey]');
    await pause(30);
    await evaluate("window.emitEvent('hotkey-recorded',{session:1,hotkey:'custom:4:67'})");
    await pause(40);
    assert.equal(
      await evaluate("document.querySelector('[data-hotkey-record=main_hotkey]').textContent"),
      '录制失败',
    );
    assert.equal(
      await evaluate("document.querySelector('select[aria-label=主窗口快捷键]').value"),
      'alt_v',
    );
    await evaluate('window.failStop=false');
    await click('[data-hotkey-record=main_hotkey]');
    await pause(30);
    await evaluate("window.emitEvent('hotkey-recorded',{session:1,hotkey:'custom:4:67'})");
    await pause(50);
    assert.equal(
      await evaluate("document.querySelector('select[aria-label=主窗口快捷键]').value"),
      'custom:4:67',
    );
    assert.equal(await evaluate("document.querySelectorAll('.hotkey-problem').length"), 0);
    assert.equal(
      await evaluate("!!document.querySelector('[data-category=hotkeys] .error')"),
      false,
    );
  });
  await test('R23 停止录制在途时改回预设，迟到结果不写入且自动检测不遗漏', async () => {
    await metrics();
    await nav();
    await click('.settings-toggle');
    await pause(60);
    await click('.settings-nav button:nth-child(5)');
    await evaluate(
      "window.recordInvoke=window.__TAURI_INTERNALS__.invoke;window.holdStop=false;window.__TAURI_INTERNALS__.invoke=(cmd,args)=>cmd==='set_hotkey_recording'&&!args.active&&window.holdStop?new Promise(resolve=>window.releaseStop=resolve):window.recordInvoke(cmd,args)",
    );
    for (const row of [
      { payload: 'custom:1:67', preset: 'ctrl_alt_v' },
      { payload: 'custom:4:67', preset: 'win_alt_v' },
    ]) {
      const before = await evaluate('window.hotkeyChecks.length');
      await evaluate('window.holdStop=true');
      await click('[data-hotkey-record=main_hotkey]');
      await pause(30);
      await evaluate(
        "void window.emitEvent('hotkey-recorded',{session:1,hotkey:" +
          JSON.stringify(row.payload) +
          '})',
      );
      await until(() => evaluate("typeof window.releaseStop==='function'"), '停止请求未开始');
      await change('select[aria-label=主窗口快捷键]', row.preset);
      await pause(30);
      assert.equal(await evaluate('window.hotkeyChecks.length'), before);
      await evaluate('window.holdStop=false;window.releaseStop(1);window.releaseStop=undefined');
      await pause(350);
      assert.equal(
        await evaluate("document.querySelector('select[aria-label=主窗口快捷键]').value"),
        row.preset,
      );
      assert.equal(await evaluate('window.hotkeyChecks.length'), before + 1);
      assert.equal(await evaluate('window.hotkeyChecks.at(-1).mainHotkey'), row.preset);
      assert.equal(await evaluate("document.querySelectorAll('.hotkey-problem').length"), 0);
    }
  });
  const k1Key = async (name, modifiers = 0) => {
    const virtualKeys = {
      ArrowUp: 38,
      ArrowDown: 40,
      ArrowLeft: 37,
      ArrowRight: 39,
      Home: 36,
      End: 35,
      PageUp: 33,
      PageDown: 34,
      Enter: 13,
      Escape: 27,
      f: 70,
      Tab: 9,
    };
    for (const type of ['keyDown', 'keyUp']) {
      await send('Input.dispatchKeyEvent', {
        type,
        key: name,
        code: name === 'f' ? 'KeyF' : name,
        windowsVirtualKeyCode: virtualKeys[name],
        modifiers,
      });
    }
  };
  const k1Kind = () =>
    evaluate(
      "document.querySelector('.kind-filters button[aria-pressed=true]')?.textContent.trim()",
    );
  const k1Group = () =>
    evaluate(
      "document.querySelector('.compact-groups select')?.value ?? document.querySelector('.group-sidebar button[aria-pressed=true]')?.dataset.groupView",
    );
  const k1Setup = async (mode) => {
    await metrics(mode === 'compact' ? 390 : 1000);
    await nav();
    await appearance({ compact: mode === 'compact', layout: mode === 'cards' ? 'cards' : 'list' });
    await evaluate(`window.rows=[
      {id:'k1-text-a',kind:'text',summary:'测试文本甲',formats:['CF_UNICODETEXT'],files:[],group_id:7},
      {id:'k1-text-b',kind:'text',summary:'测试文本乙',formats:['CF_UNICODETEXT'],files:[],group_id:7},
      {id:'k1-link',kind:'text',is_link:true,summary:'https://example.invalid/',formats:['CF_UNICODETEXT'],files:[],group_id:null},
      {id:'k1-image',kind:'image',summary:'合成图片',formats:['PNG'],files:[],group_id:8},
      {id:'k1-file',kind:'files',summary:'合成文件',formats:['CF_HDROP'],files:['C:/synthetic.txt'],group_id:null}
    ];window.k1BaseInvoke=window.__TAURI_INTERNALS__.invoke;const base=window.k1BaseInvoke;window.__TAURI_INTERNALS__.invoke=async(cmd,args={})=>{
      if(cmd!=='list_items')return base(cmd,args);
      window.calls.push({cmd,args});const f=args.filter??{};
      const rows=window.rows.filter(r=>(!f.kind||(f.kind==='link'?r.is_link:r.kind===f.kind))&&(!f.query||r.summary.includes(f.query))&&(f.group_id==null||(f.group_id===-1?r.group_id==null:r.group_id===f.group_id))).slice(args.offset,args.offset+args.limit);
      if(window.k1HoldLists)return new Promise(resolve=>window.k1Requests.push({filter:f,release:()=>resolve(rows)}));
      return rows;
    };window.emitEvent('history-changed')`);
    await until(async () => (await selected()) === 'k1-text-a', 'K1 合成列表未就绪');
    await evaluate("document.querySelector('.item').focus()");
  };
  for (const mode of ['list', 'cards', 'compact']) {
    await test('BP1 依次粘贴图片、混合记录和 Ctrl+Enter ' + mode, async () => {
      await k1Setup(mode);
      await evaluate(
        `window.rows.push({id:'k1-image-second',kind:'image',summary:'第二张合成图片',formats:['PNG'],files:[]});window.emitEvent('history-changed')`,
      );
      await until(
        () => evaluate("!!document.querySelector('[data-id=k1-image-second]')"),
        '第二张图片未出现',
      );
      const choose = (id, ctrlKey = false) =>
        evaluate(
          `document.querySelector('[data-id="${id}"]').dispatchEvent(new MouseEvent('click',{bubbles:true,ctrlKey:${ctrlKey}}))`,
        );
      const batches = () =>
        evaluate("window.calls.filter(c=>c.cmd==='paste_sequence').map(c=>c.args.ids)");
      await choose('k1-image-second');
      await evaluate("document.querySelector('[data-id=k1-image-second]').focus()");
      await k1Key('Enter', 2);
      assert.deepEqual(await batches(), []);
      await choose('k1-image', true);
      await k1Key('Enter', 2);
      assert.deepEqual(await batches(), [['k1-image-second', 'k1-image']]);
      await choose('k1-link', true);
      await choose('k1-file', true);
      await pause(100);
      await evaluate(
        "document.querySelector('[data-id=k1-image]').dispatchEvent(new MouseEvent('contextmenu',{bubbles:true,cancelable:true,clientX:180,clientY:190}))",
      );
      await until(
        () => evaluate('!!document.querySelector(\'[aria-keyshortcuts="Control+Enter"]\')'),
        '依次粘贴菜单未打开',
      );
      await click('[aria-keyshortcuts="Control+Enter"]');
      assert.deepEqual(await batches(), [
        ['k1-image-second', 'k1-image'],
        ['k1-image-second', 'k1-image', 'k1-link', 'k1-file'],
      ]);
      assert.equal(await evaluate("window.calls.filter(c=>c.cmd==='paste_item').length"), 0);
    });
    await test('BP1 多选跨分组、类型及搜索并保留原顺序 ' + mode, async () => {
      await k1Setup(mode);
      const choose = (id, ctrlKey = false) =>
        evaluate(
          `document.querySelector('[data-id="${id}"]').dispatchEvent(new MouseEvent('click',{bubbles:true,ctrlKey:${ctrlKey}}))`,
        );
      const selectedWithOrder = () =>
        evaluate(
          "Object.fromEntries([...document.querySelectorAll('.selection-order')].map(b=>[b.closest('.item').dataset.id,b.textContent]))",
        );
      const setSearch = (value) =>
        evaluate(
          `(()=>{const input=document.querySelector('.searchbox input');input.value=${JSON.stringify(value)};input.dispatchEvent(new Event('input',{bubbles:true}));input.focus()})()`,
        );
      await choose('k1-text-b');
      await choose('k1-text-a', true);
      if (mode === 'compact') await change('.compact-groups select', '8');
      else await click('[data-group-view="8"]');
      await until(
        () =>
          evaluate(
            "!!document.querySelector('[data-id=k1-image]')&&!document.querySelector('[data-id=k1-text-a]')",
          ),
        '分组未切换',
      );
      await click('.kind-filters button:nth-child(3)');
      await until(
        () => evaluate("!!document.querySelector('[data-id=k1-image]')"),
        '图片筛选未加载',
      );
      await choose('k1-image', true);
      assert.deepEqual(await selectedWithOrder(), { 'k1-image': '3' });
      await evaluate(
        "document.querySelector('[data-id=k1-image]').dispatchEvent(new MouseEvent('contextmenu',{bubbles:true,cancelable:true,clientX:180,clientY:190}))",
      );
      assert.equal(
        await evaluate("document.querySelector('[aria-keyshortcuts]').textContent.trim()"),
        '依次粘贴',
      );
      assert(await evaluate("document.querySelector('.merge-paste').disabled"));
      await k1Key('Escape');
      await setSearch('不存在的记录');
      await until(
        () => evaluate("document.querySelector('.empty h2')?.textContent==='没有匹配的记录'"),
        '空搜索未完成',
      );
      assert(
        await evaluate(
          "document.querySelector('.selection-count').textContent.includes('3 条未显示')",
        ),
      );
      await k1Key('Enter', 2);
      assert.deepEqual(
        await evaluate("window.calls.filter(c=>c.cmd==='paste_sequence').at(-1).args.ids"),
        ['k1-text-b', 'k1-text-a', 'k1-image'],
      );
      await evaluate(
        "window.rows=window.rows.filter(i=>i.id!=='k1-text-a');window.emitEvent('history-changed')",
      );
      await until(
        () =>
          evaluate(
            "document.querySelector('.selection-count')?.textContent.includes('2 条未显示')",
          ),
        '已删除记录仍在选中',
      );
      await setSearch('');
      await click('.kind-filters button:first-child');
      if (mode === 'compact') await change('.compact-groups select', 'all');
      else await click('[data-group-view="all"]');
      await until(
        () => evaluate("!!document.querySelector('[data-id=k1-text-b]')"),
        '原列表未恢复',
      );
      assert.deepEqual(await selectedWithOrder(), { 'k1-text-b': '1', 'k1-image': '2' });
      await choose('k1-text-b');
      assert.deepEqual(await selectedWithOrder(), {});
    });
    await test('BP1 合并粘贴顺序、菜单及单条 Enter ' + mode, async () => {
      await k1Setup(mode);
      await appearance({ merge_paste_separator: 'tab' });
      const choose = (id, ctrlKey = false) =>
        evaluate(
          `document.querySelector('[data-id="${id}"]').dispatchEvent(new MouseEvent('click',{bubbles:true,ctrlKey:${ctrlKey}}))`,
        );
      const openMenu = () =>
        evaluate(
          `document.querySelector('[data-id="k1-link"]').dispatchEvent(new MouseEvent('contextmenu',{bubbles:true,cancelable:true,clientX:180,clientY:190}))`,
        );
      const badges = () =>
        evaluate(
          `Object.fromEntries([...document.querySelectorAll('.selection-order')].map(b=>[b.closest('.item').dataset.id,b.textContent]))`,
        );
      await choose('k1-link');
      await openMenu();
      assert.equal(
        await evaluate("document.querySelector('.context-menu').textContent.includes('合并粘贴')"),
        false,
      );
      await k1Key('Escape');
      await choose('k1-text-a', true);
      await choose('k1-image', true);
      await openMenu();
      assert(
        await evaluate(
          "[...document.querySelectorAll('.context-menu button')].some(b=>b.disabled&&b.textContent.includes('含 1 条非文本记录'))",
        ),
      );
      await k1Key('Escape');
      await choose('k1-image', true);
      await choose('k1-text-a', true);
      await choose('k1-text-a', true);
      await evaluate("window.rows.reverse();window.emitEvent('history-changed')");
      await until(
        () => evaluate("document.querySelector('.item')?.dataset.id==='k1-file'"),
        '列表未刷新',
      );
      assert.deepEqual(await badges(), { 'k1-text-a': '2', 'k1-link': '1' });
      assert.equal(
        await evaluate(
          "[...document.querySelectorAll('.selection-order')].filter(b=>b.getBoundingClientRect().width>0).length",
        ),
        2,
      );
      await openMenu();
      await evaluate(
        "[...document.querySelectorAll('.context-menu button')].find(b=>b.textContent.includes('合并粘贴')).click()",
      );
      assert.equal(await evaluate('document.activeElement.dataset.separator'), 'tab');
      assert.equal(await evaluate("document.querySelectorAll('.merge-submenu button').length"), 5);
      await click('[data-separator="comma"]');
      assert.deepEqual(
        await evaluate("window.calls.filter(c=>c.cmd==='paste_merged').map(c=>c.args)"),
        [{ ids: ['k1-link', 'k1-text-a'], separator: 'comma' }],
      );
      await evaluate('document.querySelector(\'[data-id="k1-link"]\').focus()');
      await k1Key('Enter');
      assert.deepEqual(
        await evaluate("window.calls.filter(c=>c.cmd==='paste_item').map(c=>c.args)"),
        [{ id: 'k1-link', plain: false }],
      );
    });
    await test('K2.1 三布局首尾停止、翻页与跨页 Home/End ' + mode, async () => {
      await k1Setup(mode);
      await evaluate(
        `window.rows=Array.from({length:205},(_,index)=>({id:'record-'+index,kind:'text',summary:'记录 '+index,formats:['CF_UNICODETEXT'],files:[],group_id:7}));window.emitEvent('history-changed')`,
      );
      await until(async () => (await selected()) === 'record-0', 'K2.1 首页面未就绪');
      const firstPageCalls = await evaluate("window.calls.filter(c=>c.cmd==='list_items').length");
      await k1Key('ArrowUp');
      assert.equal(await selected(), 'record-0');
      assert.equal(
        await evaluate("window.calls.filter(c=>c.cmd==='list_items').length"),
        firstPageCalls,
      );
      const pageStep = await evaluate(
        `(()=>{const list=document.querySelector('.list'),bounds=list.getBoundingClientRect(),tops=new Set([...list.querySelectorAll('.item')].filter(row=>{const rect=row.getBoundingClientRect();return rect.bottom>bounds.top&&rect.top<bounds.bottom}).map(row=>Math.round(row.getBoundingClientRect().top)));return tops.size*(getComputedStyle(list).gridTemplateColumns.split(' ').length)})()`,
      );
      await k1Key('PageDown');
      await until(
        async () => (await selected()) === 'record-' + pageStep,
        'K2.1 PageDown 步长错误',
      );
      await k1Key('PageUp');
      await until(async () => (await selected()) === 'record-0', 'K2.1 PageUp 步长错误');
      await k1Key('End');
      await until(async () => (await selected()) === 'record-204', 'K2.1 End 未加载全部分页');
      assert(await evaluate("document.activeElement.dataset.id==='record-204'"));
      await k1Key('ArrowDown');
      assert.equal(await selected(), 'record-204');
      await k1Key('Home');
      await until(async () => (await selected()) === 'record-0', 'K2.1 Home 未回到首条');
      assert(await evaluate("document.activeElement.dataset.id==='record-0'"));
    });
    await test('K1 三布局左右类型循环及空态继续切换 ' + mode, async () => {
      await k1Setup(mode);
      const labels = ['文本', '图片', '文件', '链接', mode === 'compact' ? '全部类型' : '全部'];
      for (const label of labels) {
        await k1Key('ArrowRight');
        await until(async () => (await k1Kind()) === label, '类型未切换 ' + label);
        await pause(35);
        assert(await evaluate("document.activeElement.matches('.item, .empty')"));
        assert.equal(await evaluate("window.calls.filter(c=>c.cmd==='paste_item').length"), 0);
      }
      await k1Key('ArrowLeft');
      await pause(40);
      assert.equal(await k1Kind(), '链接');
      assert.equal(await selected(), 'k1-link');
    });
    await test('K1 三布局分组循环与类型条件保留 ' + mode, async () => {
      await k1Setup(mode);
      await k1Key('ArrowRight');
      await pause(40);
      for (const view of ['ungrouped', '7', '8', 'all']) {
        await k1Key('ArrowDown', 2);
        await until(async () => (await k1Group()) === view, '分组未切换 ' + view);
        await pause(35);
        assert.equal(await k1Kind(), '文本');
        if (view === '8')
          assert.equal(await evaluate("document.querySelectorAll('.item').length"), 0);
      }
      await k1Key('ArrowUp', 2);
      await pause(40);
      assert.equal(await k1Group(), '8');
      await k1Key('ArrowRight');
      await pause(40);
      assert.equal(await selected(), 'k1-image');
      assert.equal(await k1Group(), '8');
    });
    await test('K1 三布局搜索光标保护与进入列表返回搜索 ' + mode, async () => {
      await k1Setup(mode);
      await k1Key('f', 2);
      await send('Input.insertText', { text: '测试文本' });
      await pause(190);
      await evaluate("document.querySelector('.searchbox input').setSelectionRange(3,3)");
      await k1Key('Home');
      assert.equal(await evaluate("document.querySelector('.searchbox input').selectionStart"), 0);
      await k1Key('End');
      assert.equal(
        await evaluate("document.querySelector('.searchbox input').selectionStart"),
        '测试文本'.length,
      );
      await evaluate("document.querySelector('.searchbox input').setSelectionRange(1,1)");
      await k1Key('ArrowRight');
      assert.equal(await evaluate("document.querySelector('.searchbox input').selectionStart"), 2);
      assert.equal(await k1Kind(), mode === 'compact' ? '全部类型' : '全部');
      await evaluate("document.querySelector('.searchbox input').setSelectionRange(0,4)");
      await k1Key('ArrowLeft');
      assert.equal(await evaluate("document.querySelector('.searchbox input').selectionStart"), 0);
      await k1Key('ArrowDown');
      await pause(35);
      assert.equal(await selected(), 'k1-text-b');
      assert(await evaluate("document.activeElement.matches('.item')"));
      await k1Key('ArrowRight');
      await pause(35);
      assert.equal(await k1Kind(), '文本');
      assert.equal(await evaluate("document.querySelector('.searchbox input').value"), '测试文本');
      assert.equal(await selected(), 'k1-text-a');
      await k1Key('f', 2);
      assert(await evaluate("document.activeElement.matches('input[type=search]')"));
      await k1Key('ArrowDown', 2);
      await pause(35);
      assert.equal(await k1Group(), 'ungrouped');
      assert.equal(await evaluate("document.querySelector('.searchbox input').value"), '测试文本');
    });
  }
  await test('K2.1 End加载期间筛选变更取消旧跳转且分页失败停在已加载末尾', async () => {
    await k1Setup('list');
    await appearance({ select_first_on_show: true });
    await evaluate(
      `window.rows=Array.from({length:205},(_,index)=>({id:'record-'+index,kind:'text',summary:'记录 '+index,formats:['CF_UNICODETEXT'],files:[],group_id:7}));window.emitEvent('main-shown')`,
    );
    await until(
      async () =>
        (await selected()) === 'record-0' &&
        (await evaluate("document.querySelectorAll('.item').length")) === 100,
      'K2.1 取消用列表未就绪',
    );
    await until(
      () => evaluate("document.querySelector('.list').getAttribute('aria-busy')==='false'"),
      'K2.1 取消用列表仍在加载',
    );
    await evaluate("document.querySelector('.item').focus()");
    await evaluate('window.k1Requests=[];window.k1HoldLists=true');
    await k1Key('End');
    await until(() => evaluate('window.k1Requests.length>0'), 'K2.1 End 分页请求未挂起');
    await evaluate(
      "(()=>{const input=document.querySelector('.searchbox input');input.focus();input.value='记录 1';input.dispatchEvent(new Event('input',{bubbles:true}))})()",
    );
    await pause(30);
    await evaluate('window.k1HoldLists=false;window.k1Requests[0].release()');
    await until(async () => (await selected()) === 'record-1', '筛选后的新列表未加载');
    assert(await evaluate("document.activeElement.matches('.searchbox input')"));
    await evaluate(
      "(()=>{const input=document.querySelector('.searchbox input');input.value='';input.dispatchEvent(new Event('input',{bubbles:true}))})()",
    );
    await pause(190);
    await evaluate(
      `window.rows=Array.from({length:205},(_,index)=>({id:'record-'+index,kind:'text',summary:'记录 '+index,formats:['CF_UNICODETEXT'],files:[],group_id:7}));window.emitEvent('history-changed')`,
    );
    await until(async () => (await selected()) === 'record-0', '分页失败用列表未就绪');
    await evaluate(
      "(()=>{window.k1FailureBase=window.__TAURI_INTERNALS__.invoke;window.__TAURI_INTERNALS__.invoke=async(cmd,args={})=>{if(cmd==='list_items'&&args.offset>=100)throw Error('合成分页失败');return window.k1FailureBase(cmd,args)}})()",
    );
    await evaluate("document.querySelector('.item').focus()");
    await k1Key('End');
    await until(async () => (await selected()) === 'record-99', 'End 分页失败后未停在已加载末尾');
    assert(
      await evaluate(
        "document.querySelector('.error[role=alert]')?.textContent.includes('加载下一页失败')",
      ),
    );
    await appearance({ select_first_on_show: false });
  });
  await test('K1 空搜索左右切换，焦点控件保留与类型按钮跟随', async () => {
    await k1Setup('list');
    await k1Key('f', 2);
    await k1Key('ArrowRight');
    await pause(40);
    assert.equal(await k1Kind(), '文本');
    assert(await evaluate("document.activeElement.matches('input[type=search]')"));
    await evaluate("document.querySelector('.kind-filters button[aria-pressed=true]').focus()");
    await k1Key('ArrowRight');
    await pause(40);
    assert.equal(await k1Kind(), '图片');
    assert.equal(await evaluate("document.activeElement.getAttribute('aria-pressed')"), 'true');
    assert.equal(await evaluate('document.activeElement.textContent.trim()'), '图片');
    await evaluate("document.querySelector('.group-sidebar button[aria-pressed=true]').focus()");
    await k1Key('ArrowDown', 2);
    await pause(40);
    assert.equal(await k1Group(), 'ungrouped');
    assert.equal(await evaluate("document.activeElement.getAttribute('aria-pressed')"), 'true');
  });
  await test('K1 额外修饰键与工具按钮不触发筛选或选记录', async () => {
    await k1Setup('list');
    for (const modifiers of [1, 8, 4, 10]) {
      await k1Key('ArrowRight', modifiers);
      await k1Key('ArrowDown', modifiers);
    }
    assert.equal(await k1Kind(), '全部');
    assert.equal(await k1Group(), 'all');
    assert.equal(await selected(), 'k1-text-a');
    await evaluate("document.querySelector('.compact-toggle').focus()");
    await k1Key('ArrowRight');
    await k1Key('ArrowDown', 2);
    assert.equal(await k1Kind(), '全部');
    assert.equal(await k1Group(), 'all');
  });
  await test('K1 无用户分组仍循环全部和未分组，搜索结果为空可继续', async () => {
    await k1Setup('compact');
    await evaluate(
      "(()=>{const groupInvoke=window.__TAURI_INTERNALS__.invoke;window.__TAURI_INTERNALS__.invoke=(cmd,args)=>cmd==='list_groups'?Promise.resolve([]):groupInvoke(cmd,args);window.emitEvent('history-changed')})()",
    );
    await until(
      () => evaluate("document.querySelector('.compact-groups select').options.length===2"),
      '分组未清空',
    );
    await k1Key('ArrowUp', 2);
    await pause(40);
    assert.equal(await k1Group(), 'ungrouped');
    await k1Key('ArrowDown', 2);
    await pause(40);
    assert.equal(await k1Group(), 'all');
    await k1Key('f', 2);
    await send('Input.insertText', { text: '无结果关键词' });
    await pause(190);
    await k1Key('ArrowDown');
    await pause(30);
    assert(await evaluate("document.activeElement.matches('.empty')"));
    await k1Key('ArrowRight');
    await pause(40);
    assert.equal(await k1Kind(), '文本');
    assert.equal(
      await evaluate("document.querySelector('.searchbox input').value"),
      '无结果关键词',
    );
  });
  await test('K1 原生分组下拉未提交时保留方向键不拦截', async () => {
    await k1Setup('compact');
    const prevented = await evaluate(
      "(()=>{const n=document.querySelector('.compact-groups select');n.focus();return ['ArrowRight','ArrowUp','ArrowDown'].map(key=>!n.dispatchEvent(new KeyboardEvent('keydown',{key,ctrlKey:true,bubbles:true,cancelable:true})))})()",
    );
    assert.deepEqual(prevented, [false, false, false]);
    assert.equal(await k1Group(), 'all');
    assert.equal(await selected(), 'k1-text-a');
  });
  await test('K1 输入法、设置、菜单和分组编辑期间保留原交互', async () => {
    await k1Setup('list');
    await evaluate(
      "document.activeElement.dispatchEvent(new KeyboardEvent('keydown',{key:'ArrowRight',isComposing:true,bubbles:true,cancelable:true}))",
    );
    assert.equal(await k1Kind(), '全部');
    await evaluate(
      "document.querySelector('.item').dispatchEvent(new MouseEvent('contextmenu',{bubbles:true,cancelable:true,clientX:250,clientY:200}))",
    );
    await k1Key('ArrowDown', 2);
    assert.equal(await k1Group(), 'all');
    await k1Key('Escape');
    await click('.settings-toggle');
    await pause(60);
    await k1Key('ArrowRight');
    await k1Key('ArrowDown', 2);
    await k1Key('Escape');
    await pause(40);
    assert.equal(await k1Kind(), '全部');
    assert.equal(await k1Group(), 'all');
    await evaluate(
      "Array.from(document.querySelectorAll('.group-sidebar button')).find(button=>button.textContent.includes('新建分组')).click()",
    );
    await pause(30);
    await k1Key('ArrowDown', 2);
    await k1Key('ArrowRight');
    assert.equal(await k1Kind(), '全部');
    assert.equal(await k1Group(), 'all');
  });
  await test('K1 连续筛选旧响应丢弃且加载中Enter不粘贴旧记录', async () => {
    await k1Setup('list');
    await evaluate('window.k1Requests=[];window.k1HoldLists=true');
    await k1Key('ArrowRight');
    await until(() => evaluate('window.k1Requests.length===1'), '首个筛选未开始');
    await k1Key('Enter');
    assert.equal(await evaluate("window.calls.filter(call=>call.cmd==='paste_item').length"), 0);
    await k1Key('ArrowRight');
    await until(() => evaluate('window.k1Requests.length===2'), '第二个筛选未开始');
    await evaluate('window.k1HoldLists=false;window.k1Requests[1].release()');
    await until(async () => (await selected()) === 'k1-image', '新筛选未显示');
    await evaluate('window.k1Requests[0].release()');
    await pause(40);
    assert.equal(await selected(), 'k1-image');
    assert.equal(await k1Kind(), '图片');
    await k1Key('Enter');
    await pause(30);
    assert.equal(
      await evaluate("window.calls.filter(call=>call.cmd==='paste_item').at(-1).args.id"),
      'k1-image',
    );
  });
  await test('K1 切换保留多选并清除敏感明文预览', async () => {
    await k1Setup('list');
    await evaluate("window.rows[0].sensitive=true;window.emitEvent('history-changed')");
    await pause(70);
    await click('.item');
    await evaluate(
      "const b=Array.from(document.querySelectorAll('.detail button')).find(button=>button.textContent.includes('显示'));b?.click()",
    );
    await pause(40);
    assert(
      await evaluate("document.querySelector('main').textContent.includes('SYNTHETIC_SECRET_RAW')"),
    );
    await evaluate("document.querySelector('.item').focus()");
    await k1Key('ArrowRight');
    await pause(45);
    assert.equal(
      await evaluate("document.querySelector('main').textContent.includes('SYNTHETIC_SECRET_RAW')"),
      false,
    );
    await evaluate(
      "document.querySelectorAll('.item')[1].dispatchEvent(new MouseEvent('click',{ctrlKey:true,bubbles:true}));document.querySelector('.item').focus()",
    );
    assert.equal(
      await evaluate("document.querySelectorAll('.item[aria-selected=true]').length"),
      2,
    );
    await k1Key('ArrowDown', 2);
    await pause(45);
    assert(
      await evaluate(
        "document.querySelector('.selection-count')?.textContent.includes('已选 2 条')",
      ),
    );
    assert.equal(
      await evaluate("document.querySelector('main').textContent.includes('SYNTHETIC_SECRET_RAW')"),
      false,
    );
  });
  await test('K1 删除确认期间不切换筛选，取消后恢复', async () => {
    await k1Setup('list');
    await evaluate(
      "document.querySelectorAll('.item')[1].dispatchEvent(new MouseEvent('click',{ctrlKey:true,bubbles:true}));document.querySelector('.item').focus()",
    );
    await evaluate(
      "document.activeElement.dispatchEvent(new KeyboardEvent('keydown',{key:'Delete',bubbles:true,cancelable:true}))",
    );
    await pause(30);
    assert(await evaluate("!!document.querySelector('.delete-confirm')"));
    await k1Key('ArrowRight');
    await k1Key('ArrowDown', 2);
    assert.equal(await k1Kind(), '全部');
    assert.equal(await k1Group(), 'all');
    await k1Key('Escape');
    await k1Key('ArrowRight');
    await pause(40);
    assert.equal(await k1Kind(), '文本');
  });
  await test('K1 筛选等待期间Ctrl+F保留搜索焦点，迟到结果不抢焦点', async () => {
    await k1Setup('compact');
    await evaluate('window.k1Requests=[];window.k1HoldLists=true');
    await k1Key('ArrowRight');
    await until(() => evaluate('window.k1Requests.length===1'), '请求未开始');
    await k1Key('f', 2);
    await evaluate('window.k1HoldLists=false;window.k1Requests[0].release()');
    await pause(50);
    assert(await evaluate("document.activeElement.matches('input[type=search]')"));
    assert.equal(await k1Kind(), '文本');
  });
  await test('K1 类型控件上Ctrl方向切分组与侧栏左右切类型', async () => {
    await k1Setup('list');
    await evaluate("document.querySelector('.kind-filters button').focus()");
    await k1Key('ArrowDown', 2);
    await pause(40);
    assert.equal(await k1Group(), 'ungrouped');
    await evaluate("document.querySelector('.group-sidebar button[aria-pressed=true]').focus()");
    await k1Key('ArrowRight');
    await pause(40);
    assert.equal(await k1Kind(), '文本');
    assert.equal(await k1Group(), 'ungrouped');
  });
  await test('K1 搜索框边界与选区不偷切类型，空搜索则可切换', async () => {
    await k1Setup('compact');
    await k1Key('f', 2);
    await send('Input.insertText', { text: '测试' });
    await pause(190);
    await evaluate("document.querySelector('.searchbox input').setSelectionRange(2,2)");
    await k1Key('ArrowRight');
    assert.equal(await k1Kind(), '全部类型');
    await evaluate("document.querySelector('.searchbox input').setSelectionRange(0,0)");
    await k1Key('ArrowLeft');
    assert.equal(await k1Kind(), '全部类型');
    await evaluate(
      "const input=document.querySelector('.searchbox input');input.value='';input.dispatchEvent(new Event('input',{bubbles:true}))",
    );
    await k1Key('ArrowLeft');
    await pause(40);
    assert.equal(await k1Kind(), '链接');
  });
  await test('K1 筛选加载后不抢用户新选择的搜索或工具焦点', async () => {
    for (const selector of ['.searchbox input', '.compact-toggle']) {
      await k1Setup('list');
      await evaluate('window.k1Requests=[];window.k1HoldLists=true');
      await k1Key('ArrowRight');
      await until(() => evaluate('window.k1Requests.length===1'), '筛选请求未开始');
      await evaluate(
        `document.querySelector(${JSON.stringify(selector)}).focus();window.k1HoldLists=false;window.k1Requests[0].release()`,
      );
      await pause(50);
      assert(await evaluate(`document.activeElement.matches(${JSON.stringify(selector)})`));
    }
  });
  for (const mode of ['list', 'cards', 'compact']) {
    await test('K1 说明与类型分隔布局 ' + mode, async () => {
      await k1Setup(mode);
      await k1Key('ArrowRight');
      await k1Key('ArrowRight');
      await pause(50);
      const geometry = await evaluate(
        `(()=>{const label=document.querySelector('.filter-label').getBoundingClientRect(),types=document.querySelector('.kind-filters').getBoundingClientRect(),group=document.querySelector('.compact-groups select')?.getBoundingClientRect();return {gap:types.left-label.right,aligned:!group||Math.abs(group.left-types.left)<=1,border:getComputedStyle(document.querySelector('.kind-filters')).borderTopWidth,divider:getComputedStyle(document.querySelector('.kind-filters')).borderLeftWidth,background:getComputedStyle(document.querySelector('.kind-filters')).backgroundColor,overflow:document.documentElement.scrollWidth>innerWidth,hint:document.querySelector('.keyboard-hint').textContent}})()`,
      );
      assert.equal(geometry.gap, 10);
      assert.equal(geometry.aligned, true);
      assert.equal(geometry.border, '1px');
      assert.equal(geometry.divider, '1px');
      assert.notEqual(geometry.background, 'rgba(0, 0, 0, 0)');
      assert.equal(geometry.overflow, false);
      assert.equal(geometry.hint.includes('Shift+Enter 粘贴纯文本'), mode !== 'compact');
      assert.equal(geometry.hint.includes('Ctrl+Enter 依次粘贴'), mode !== 'compact');
      assert(geometry.hint.includes('Enter 粘贴') && !geometry.hint.includes('Esc'));
      assert(
        geometry.hint.includes('Ctrl+↑↓') &&
          geometry.hint.includes('←→') &&
          !geometry.hint.includes('Ctrl+F'),
      );
      await capture('k1-filter-help-' + mode);
      await click(mode === 'compact' ? 'footer .settings-toggle' : '.settings-toggle');
      await pause(60);
      assert(
        (
          await evaluate("document.querySelector('[data-panel-keyboard-help]').textContent")
        ).includes('紧凑'),
      );
    });
  }
  for (const theme of ['light', 'dark']) {
    for (const mode of ['list', 'cards', 'compact']) {
      await test('K1 Tab与侧栏统一选中态 ' + theme + ' ' + mode, async () => {
        await k1Setup(mode);
        await appearance({ theme });
        for (const index of [0, 1, 2, 3, 4]) {
          await evaluate(`document.querySelectorAll('.kind-filters button')[${index}].click()`);
          await pause(30);
          const state = await evaluate(
            `(()=>{const bar=document.querySelector('.kind-filters'),tabs=[...bar.querySelectorAll('button')],selected=bar.querySelector('[aria-pressed=true]'),style=getComputedStyle(selected),r=bar.getBoundingClientRect();return {count:tabs.filter(tab=>tab.getAttribute('aria-pressed')==='true').length,index:tabs.indexOf(selected),background:style.backgroundColor,page:getComputedStyle(document.querySelector('main')).backgroundColor,border:style.borderTopWidth,weight:style.fontWeight,shadow:style.boxShadow,sidebar:getComputedStyle(document.querySelector('.group-sidebar button[aria-pressed=true]')).backgroundColor,selectedColor:style.color,sidebarColor:getComputedStyle(document.querySelector('.group-sidebar button[aria-pressed=true]')).color,inside:tabs.every(tab=>{const b=tab.getBoundingClientRect();return b.left>=r.left&&b.right<=r.right}),overflow:document.documentElement.scrollWidth>innerWidth}})()`,
          );
          assert.equal(state.count, 1);
          assert.equal(state.index, index);
          assert.equal(state.border, '1px');
          assert.equal(state.weight, '600');
          assert.notEqual(state.background, 'rgba(0, 0, 0, 0)');
          assert.equal(state.shadow, 'none');
          assert.equal(state.background, state.sidebar);
          assert.equal(state.selectedColor, state.sidebarColor);
          assert.equal(state.inside, true);
          assert.equal(state.overflow, false);
        }
        await evaluate("document.querySelectorAll('.kind-filters button')[2].click()");
        await pause(30);
        await capture('k1-tabs-a-' + theme + '-' + mode);
        if (mode === 'compact') {
          await metrics(320);
          assert.equal(await evaluate('document.documentElement.scrollWidth>innerWidth'), false);
        }
      });
    }
  }
  const richSetup = async (mode = 'list') => {
    await metrics(mode === 'compact' ? 390 : 1000);
    await nav();
    await appearance({ compact: mode === 'compact', layout: mode === 'cards' ? 'cards' : 'list' });
    await evaluate(
      `(()=>{const canvas=document.createElement('canvas');canvas.width=160;canvas.height=220;const context=canvas.getContext('2d');context.fillStyle='#277d77';context.fillRect(0,0,160,220);const image=canvas.toDataURL('image/png');window.b10Rich={blocks:[{kind:'text',text:'合成第一段 <script>不会执行</script>'},{kind:'image',src:image,message:null},{kind:'text',text:'合成第二段'},{kind:'image',src:image,message:null}],truncated:false,linked_images:true};window.rows=[{id:'rich',kind:'text',summary:'合成图文',formats:['CF_UNICODETEXT','HTML Format'],files:[]},{id:'plain',kind:'text',summary:'普通文本',formats:['CF_UNICODETEXT'],files:[]}];const invoke=window.__TAURI_INTERNALS__.invoke;window.__TAURI_INTERNALS__.invoke=async(cmd,args)=>{if(cmd==='item_detail'&&args.id==='rich'){if(window.b10Hold)return new Promise(resolve=>window.b10Release=()=>resolve({text:'纯文本回退',files:[],truncated:false,rich:window.b10Rich}));return {text:'纯文本回退',files:[],truncated:false,...(!window.rows[0].sensitive?{rich:window.b10Rich}:{})};}if(cmd==='reveal_item'&&args.id==='rich')return {text:'纯文本回退',files:[],truncated:false,rich:window.b10Rich};return invoke(cmd,args)};window.emitEvent('history-changed')})()`,
    );
    await until(
      () => evaluate("document.querySelector('.item')?.dataset.id==='rich'"),
      '图文记录未显示',
    );
  };
  for (const mode of ['list', 'cards']) {
    await test('B10 图文顺序与实际图片加载 ' + mode, async () => {
      await richSetup(mode);
      await until(
        () =>
          evaluate(
            "document.querySelectorAll('.rich-preview img').length===2&&Array.from(document.querySelectorAll('.rich-preview img')).every(image=>image.complete&&image.naturalWidth>0)",
          ),
        '图文图片未渲染',
      );
      assert.deepEqual(
        await evaluate(
          "Array.from(document.querySelector('.rich-preview').children).slice(0,4).map(element=>element.tagName)",
        ),
        ['P', 'FIGURE', 'P', 'FIGURE'],
      );
      assert.equal(await evaluate("document.querySelectorAll('.rich-preview script').length"), 0);
      assert(
        (await evaluate("document.querySelector('.rich-preview').textContent")).includes(
          '<script>不会执行</script>',
        ),
      );
      assert.equal(await evaluate("document.querySelector('.text-detail')!==null"), false);
      assert.equal(
        await evaluate("document.querySelector('.detail-header').textContent.includes('图文预览')"),
        true,
      );
      assert.equal(
        await evaluate(
          "Array.from(document.querySelectorAll('.rich-preview img')).every(image=>image.getBoundingClientRect().right<=document.querySelector('.detail').getBoundingClientRect().right)",
        ),
        true,
      );
      await capture('b10-rich-' + mode);
    });
  }
  await test('B10 敏感图文未解锁不展示，显式解锁后展示并可重新隐藏', async () => {
    await richSetup();
    await evaluate("window.rows[0].sensitive=true;window.emitEvent('history-changed')");
    await pause(100);
    assert.equal(await evaluate("document.querySelectorAll('.rich-preview img').length"), 0);
    await evaluate("document.querySelector('.sensitive-actions button').click()");
    await until(
      () => evaluate("document.querySelectorAll('.rich-preview img').length===2"),
      '显式解锁未显示',
    );
    await evaluate("document.querySelector('.sensitive-actions button').click()");
    await pause(30);
    assert.equal(await evaluate("document.querySelectorAll('.rich-preview img').length"), 0);
  });
  await test('B10 失效和外链占位不产生浏览器资源请求', async () => {
    await richSetup();
    await evaluate(
      "window.b10Rich={blocks:[{kind:'text',text:'保留文字'},{kind:'image',src:null,message:'原图片已失效'},{kind:'image',src:'https://invalid/remote.jpg',message:'外部图片未加载'}],truncated:true,linked_images:false};window.emitEvent('history-changed')",
    );
    await pause(90);
    assert.equal(await evaluate("document.querySelectorAll('.rich-preview img').length"), 0);
    assert.equal(await evaluate("document.querySelectorAll('.rich-preview figcaption').length"), 2);
    assert(
      (await evaluate("document.querySelector('.rich-preview').textContent")).includes(
        '仅显示部分',
      ),
    );
  });
  await test('B10 切换记录清除图文，迟到详情不恢复旧图', async () => {
    await richSetup();
    await evaluate("window.b10Hold=true;window.emitEvent('history-changed')");
    await until(() => evaluate("typeof window.b10Release==='function'"), '延迟详情未开始');
    await click('[data-id="plain"]');
    await evaluate('window.b10Release()');
    await pause(70);
    assert.equal(await evaluate("document.querySelectorAll('.rich-preview').length"), 0);
  });
  await test('B10 紧凑浮窗图文与图片加载尺寸回报', async () => {
    await richSetup('compact');
    const rich = await evaluate('window.b10Rich');
    await send('Page.navigate', { url: origin + '/?panel=preview' });
    await until(() => evaluate("!!document.querySelector('.floating-preview')"), '浮窗未创建');
    await pause(100);
    await evaluate(
      `window.emitEvent('compact-preview-changed',{generation:1,item:{id:'rich',kind:'text',summary:'合成图文',formats:['HTML Format'],files:[]},detail:{text:'纯文本回退',files:[],truncated:false,rich:${JSON.stringify(rich)}}})`,
    );
    await until(
      () =>
        evaluate(
          "document.querySelectorAll('.rich-preview img').length===2&&Array.from(document.querySelectorAll('.rich-preview img')).every(image=>image.naturalWidth>0)",
        ),
      '浮窗图文未加载',
    );
    assert.equal(await evaluate("document.querySelectorAll('.floating-text').length"), 0);
    await until(
      () => evaluate("window.calls.some(call=>call.cmd==='resize_compact_preview')"),
      '浮窗图片加载后未报告尺寸',
    );
    await capture('b10-floating');
    await evaluate("window.emitEvent('compact-preview-changed',null)");
    await pause(35);
    assert.equal(await evaluate("document.querySelectorAll('.rich-preview img').length"), 0);
  });
  if (process.env.B10_REAL_FIXTURE_DIR) {
    await test('B10 真实记录隔离副本实际渲染', async () => {
      const content = JSON.parse(
        await readFile(
          path.join(process.env.B10_REAL_FIXTURE_DIR, 'resolved-preview.json'),
          'utf8',
        ),
      );
      await richSetup();
      await evaluate(
        `window.b10Rich=${JSON.stringify(content)};window.emitEvent('history-changed')`,
      );
      await until(
        () =>
          evaluate(
            "document.querySelectorAll('.rich-preview img').length===2&&Array.from(document.querySelectorAll('.rich-preview img')).every(image=>image.complete&&image.naturalHeight>0)",
          ),
        '真实记录图片未完成渲染',
      );
      assert.equal(
        await evaluate("document.querySelectorAll('.rich-preview .rich-text').length"),
        2,
      );
      assert.equal(
        await evaluate("document.querySelectorAll('.rich-preview figcaption').length"),
        0,
      );
      assert.equal(
        await evaluate(
          "Array.from(document.querySelectorAll('.rich-preview img')).every(image=>image.getBoundingClientRect().right<=document.querySelector('.detail').getBoundingClientRect().right)",
        ),
        true,
      );
      await capture('b10-real-record');
    });
  }
  const storagePage = async (patch = {}) => {
    await nav();
    await evaluate(
      `window.directoryStatus={...window.directoryStatus,enabled:true,...${JSON.stringify(patch)}};window.directoryPlan={ticket:'synthetic-ticket',source:'D:/synthetic/current',target:'E:/合成 空目录/片筏',bytes:2097152}`,
    );
    await click('.settings-toggle');
    await pause(80);
    await click('.settings-nav button:nth-child(6)');
  };
  for (const mode of ['list', 'cards', 'compact']) {
    await test('S1 数据存储三布局显示与窄窗不溢出 ' + mode, async () => {
      await metrics(mode === 'compact' ? 390 : 1000);
      await nav();
      await appearance({
        layout: mode === 'cards' ? 'cards' : 'list',
        compact: mode === 'compact',
        theme: mode === 'compact' ? 'dark' : 'light',
      });
      await evaluate(
        "window.directoryStatus={...window.directoryStatus,enabled:true,previous:'C:/合成旧目录/仍可能含敏感记录'}",
      );
      await click('.settings-toggle');
      await pause(80);
      if (mode === 'compact') await change('select[aria-label=设置分类]', 'storage');
      else await click('.settings-nav button:nth-child(6)');
      assert(
        (await evaluate("document.querySelector('[data-current-directory]').textContent")).includes(
          'synthetic/current',
        ),
      );
      assert(
        (
          await evaluate("document.querySelector('[data-previous-directory]').textContent")
        ).includes('合成旧目录'),
      );
      assert.equal(await evaluate('document.documentElement.scrollWidth<=innerWidth'), true);
      if (mode !== 'cards') await capture('s1-storage-' + mode);
    });
  }
  await test('S1 路径只移除展示用扩展前缀，迁移数据保持原值', async () => {
    await metrics();
    await nav();
    const prefix = String.fromCharCode(92, 92, 63, 92);
    const slash = String.fromCharCode(92);
    const paths = {
      current: prefix + 'C:' + slash + '合成数据',
      previous: prefix + 'UNC' + slash + 'server' + slash + 'share',
      source: prefix + 'D:' + slash + '源目录',
      target: prefix + 'E:' + slash + '目标目录',
    };
    await evaluate(
      `window.directoryStatus={...window.directoryStatus,enabled:true,current:${JSON.stringify(paths.current)},previous:${JSON.stringify(paths.previous)}};window.directoryPlan={ticket:'prefix-ticket',source:${JSON.stringify(paths.source)},target:${JSON.stringify(paths.target)},bytes:1}`,
    );
    await click('.settings-toggle');
    await pause(80);
    await click('.settings-nav button:nth-child(6)');
    await click('.data-directory-settings button');
    await pause(30);
    for (const [field, expected] of Object.entries({
      current: 'C:' + slash + '合成数据',
      previous: slash + slash + 'server' + slash + 'share',
      source: 'D:' + slash + '源目录',
      target: 'E:' + slash + '目标目录',
    })) {
      assert.equal(
        await evaluate(`document.querySelector('[data-${field}-directory]').textContent`),
        expected,
      );
    }
    assert.equal(await evaluate('window.directoryStatus.current'), paths.current);
    assert.equal(await evaluate('window.directoryPlan.target'), paths.target);
    await click('.directory-confirm button');
    await pause(30);
    assert.deepEqual(await evaluate('window.directoryRequests.at(-1)'), {
      cmd: 'migrate_data_directory',
      args: { ticket: 'prefix-ticket' },
    });
  });
  await test('S1 隔离环境禁止更改与恢复目录', async () => {
    await metrics();
    await storagePage({ enabled: false });
    assert.equal(
      await evaluate(
        "Array.from(document.querySelectorAll('.data-directory-settings button')).every(button=>button.disabled)",
      ),
      true,
    );
    assert.equal(await evaluate('window.directoryRequests.length'), 0);
  });
  await test('S1 选择器取消与预检确认条取消均不执行迁移', async () => {
    await storagePage();
    await evaluate('window.directoryPlan=null');
    await click('.data-directory-settings button');
    await pause(30);
    assert.equal(await evaluate("!!document.querySelector('.directory-confirm')"), false);
    await evaluate(
      "window.directoryPlan={ticket:'ticket',source:'D:/old',target:'E:/new',bytes:1}",
    );
    await click('.data-directory-settings button');
    await pause(30);
    assert.equal(await evaluate("!!document.querySelector('.directory-confirm')"), true);
    await click('.directory-confirm button:nth-child(2)');
    assert.equal(await evaluate("!!document.querySelector('.directory-confirm')"), false);
    assert.equal(
      await evaluate(
        "window.directoryRequests.some(request=>request.cmd==='migrate_data_directory')",
      ),
      false,
    );
  });
  await test('S1 未保存设置阻止迁移，保存后允许确认', async () => {
    await storagePage();
    await evaluate("document.querySelector('input[type=checkbox]').click()");
    await click('.data-directory-settings button');
    await pause(30);
    assert.equal(
      await evaluate("document.querySelector('.directory-confirm button').disabled"),
      true,
    );
    await click('.settings-buttons button[type=submit]');
    await pause(80);
    assert.equal(
      await evaluate("document.querySelector('.directory-confirm button').disabled"),
      false,
    );
    assert.equal(
      await evaluate(
        "window.directoryRequests.some(request=>request.cmd==='migrate_data_directory')",
      ),
      false,
    );
  });
  await test('S1 预检失败保留当前路径且不执行迁移', async () => {
    await storagePage();
    await evaluate('window.failDirectoryChoice=true');
    await click('.data-directory-settings button');
    await pause(30);
    assert.equal(await evaluate("!!document.querySelector('.directory-confirm')"), false);
    assert(
      (
        await evaluate(
          "document.querySelector('.data-directory-settings [role=alert]').textContent",
        )
      ).includes('非空'),
    );
    assert.equal(
      await evaluate("document.querySelector('[data-current-directory]').textContent"),
      'D:/synthetic/current',
    );
  });
  await test('S1 确认失败可重试，成功只发送预检票据并保持重启锁定', async () => {
    await storagePage();
    await click('.data-directory-settings button');
    await pause(30);
    await evaluate('window.failDirectoryMigration=true');
    await click('.directory-confirm button');
    await pause(30);
    assert.equal(
      await evaluate("document.querySelector('.directory-confirm button').disabled"),
      false,
    );
    await evaluate('window.failDirectoryMigration=false');
    await click('.directory-confirm button');
    await pause(30);
    assert.deepEqual(
      await evaluate(
        "window.directoryRequests.filter(request=>request.cmd==='migrate_data_directory').at(-1).args",
      ),
      { ticket: 'synthetic-ticket' },
    );
    assert.equal(
      await evaluate("document.querySelector('.directory-confirm button').disabled"),
      true,
    );
    assert(
      (await evaluate("document.querySelector('.data-directory-settings').textContent")).includes(
        '正在重启',
      ),
    );
  });
  await test('S1 恢复默认也先预检，选择期间锁定设置操作', async () => {
    await storagePage();
    await evaluate('window.holdDirectoryChoice=true');
    await click('.data-directory-settings .directory-actions button:nth-child(2)');
    await pause(30);
    assert.equal(await evaluate("document.querySelector('.settings-back').disabled"), true);
    await evaluate('window.releaseDirectoryChoice()');
    await pause(30);
    assert.equal(await evaluate('window.directoryRequests.at(-1).cmd'), 'restore_data_directory');
    assert.equal(await evaluate("!!document.querySelector('.directory-confirm')"), true);
    assert.equal(
      await evaluate(
        "window.directoryRequests.some(request=>request.cmd==='migrate_data_directory')",
      ),
      false,
    );
  });
  for (const compact of [false, true]) {
    await test('S1 恢复默认显示旧数据保留位置且取消不迁移 ' + compact, async () => {
      await metrics(compact ? 390 : 1000);
      await nav();
      await appearance({ compact, theme: compact ? 'dark' : 'light' });
      await evaluate(
        "window.directoryStatus={...window.directoryStatus,enabled:true,displaced:'C:/合成已保留旧数据'};window.directoryPlan={ticket:'restore-ticket',source:'D:/current',target:'C:/default',bytes:1024,displaced:'C:/合成待保留旧数据'}",
      );
      await click('.settings-toggle');
      await pause(80);
      if (compact) await change('select[aria-label=设置分类]', 'storage');
      else await click('.settings-nav button:nth-child(6)');
      await click('.data-directory-settings .directory-actions button:nth-child(2)');
      await pause(30);
      assert.equal(
        await evaluate("document.querySelector('[data-displaced-directory]').textContent"),
        'C:/合成已保留旧数据',
      );
      assert.equal(
        await evaluate("document.querySelector('[data-planned-backup]').textContent"),
        'C:/合成待保留旧数据',
      );
      assert(
        (await evaluate("document.querySelector('.directory-confirm').textContent")).includes(
          '无需手工清空',
        ),
      );
      assert.equal(await evaluate('document.documentElement.scrollWidth<=innerWidth'), true);
      await capture('s1-restore-' + (compact ? 'compact' : 'normal'));
      await click('.directory-confirm button:nth-child(2)');
      assert.equal(
        await evaluate(
          "window.directoryRequests.some(request=>request.cmd==='migrate_data_directory')",
        ),
        false,
      );
    });
  }
  await test('U4 移除面板背景设置，旧值不恢复入口', async () => {
    await metrics(1000);
    await nav();
    await appearance({ quick_background: 'native' });
    await click('.settings-toggle');
    await until(
      () => evaluate("!!document.querySelector('select[aria-label=主题]')"),
      '设置未加载',
    );
    await click('.settings-nav button:nth-child(2)');
    assert.equal(await evaluate("document.querySelector('select[aria-label=快速面板背景]')"), null);
    await capture('card-shadow-settings');
  });
  await test('K1 类型标签恢复普通样式且保留类型Tab', async () => {
    for (const theme of ['light', 'dark']) {
      for (const mode of ['list', 'cards', 'compact']) {
        await k1Setup(mode);
        await appearance({ theme });
        const label = await evaluate(
          "(()=>{const n=document.querySelector('.filter-label'),s=getComputedStyle(n),group=document.querySelector('.compact-groups');return {weight:s.fontWeight,color:s.color,groupColor:group?getComputedStyle(group).color:s.color,size:s.fontSize,tabs:document.querySelectorAll('.kind-filters button').length,selected:document.querySelectorAll('.kind-filters [aria-pressed=true]').length};})()",
        );
        assert.equal(label.weight, '400');
        assert.equal(label.size, '12px');
        assert.equal(label.color, label.groupColor);
        assert.equal(label.tabs, 5);
        assert.equal(label.selected, 1);
        if (mode === 'compact') await capture('plain-type-label-' + theme);
      }
    }
  });
  await test('U7 皮肤草稿取消、保存与六种布局配色', async () => {
    await metrics();
    await nav('?fluent-fixture=1');
    const editSkin = async () => {
      await click('.settings-toggle');
      await until(() => evaluate("!!document.querySelector('.settings-view form')"), '设置未打开');
      await click('.settings-nav button:nth-child(2)');
      await change('select[aria-label=皮肤]', 'fluent');
    };
    await editSkin();
    await click('.settings-cancel');
    assert.equal(await evaluate('window.settings.skin'), 'clipraft');
    await editSkin();
    assert.equal(await evaluate('document.documentElement.dataset.skin'), 'clipraft');
    await click('.settings-view button[type=submit]');
    await until(() => evaluate("document.documentElement.dataset.skin==='fluent'"), '皮肤未生效');
    assert.equal(
      await evaluate(
        "document.querySelector('[aria-label=皮肤] option[value=clipraft]').textContent",
      ),
      '素笺',
    );
    assert.equal(await evaluate('window.settings.skin'), 'fluent');
    assert.deepEqual(
      await evaluate(
        "[...document.querySelectorAll('.appearance-section h3')].map(n=>n.textContent)",
      ),
      ['主题', '布局'],
    );
    assert.deepEqual(
      await evaluate(
        "[...document.querySelectorAll('.settings-buttons button')].map(n=>n.textContent.trim())",
      ),
      ['取消', '保存'],
    );
    assert(await evaluate("document.querySelector('.settings-buttons [type=submit]').disabled"));
    assert(
      await evaluate(
        "document.querySelector('.appearance-section small').getBoundingClientRect().height > 0",
      ),
    );
    const switchStyle = () =>
      evaluate(`(() => {
        const control = document.querySelector('.compact-setting [role=switch]');
        const track = getComputedStyle(control);
        const thumb = getComputedStyle(control, '::before');
        return {
          startsAtLeadingEdge: track.flexDirection === 'row' &&
            ['flex-start', 'flow-start'].includes(track.justifyContent),
          travel: new DOMMatrix(thumb.transform === 'none' ? '' : thumb.transform).m41,
          available: control.clientWidth - parseFloat(track.paddingLeft) -
            parseFloat(track.paddingRight) - parseFloat(thumb.width)
        };
      })()`);
    assert.deepEqual(await switchStyle(), { startsAtLeadingEdge: true, travel: 0, available: 19 });
    await capture('fluent-settings-switch-off');
    await click('.compact-setting [role=switch]');
    assert(await evaluate("document.querySelector('.compact-setting [role=switch]').checked"));
    assert(!(await evaluate("document.querySelector('.settings-buttons [type=submit]').disabled")));
    await until(async () => (await switchStyle()).travel === 19, '开关滑块未停在轨道内');
    await capture('fluent-settings-switch-on');
    await click('.compact-setting [role=switch]');
    const openPicker = async () => {
      const point = await evaluate(
        "(()=>{const r=document.querySelector('select[aria-label=明暗]').getBoundingClientRect();return {x:r.x+r.width/2,y:r.y+r.height/2}})()",
      );
      for (const type of ['mousePressed', 'mouseReleased'])
        await send('Input.dispatchMouseEvent', { type, ...point, button: 'left', clickCount: 1 });
    };
    await openPicker();
    await until(() => evaluate("!!document.querySelector('select:open')"), '下拉浮层未打开');
    await pause(180);
    await capture('fluent-settings-picker');
    await key('ArrowDown', 40);
    await key('Escape', 27);
    assert.equal(await evaluate('document.activeElement.value'), 'light');
    await openPicker();
    await key('ArrowDown', 40);
    await key('Enter', 13);
    assert.equal(await evaluate('document.activeElement.value'), 'dark');
    await click('.settings-title-back');
    await click('[data-id="row-2"]');
    for (const theme of ['light', 'dark']) {
      for (const mode of ['list', 'cards', 'compact']) {
        await metrics(mode === 'compact' ? 390 : 1000);
        await appearance({
          theme,
          layout: mode === 'cards' ? 'cards' : 'list',
          compact: mode === 'compact',
        });
        assert.equal(await selected(), 'row-2');
        assert.deepEqual(
          await evaluate(
            "(()=>{const style=getComputedStyle(document.querySelector('.item.selected .summary'));return [style.fontSize,style.fontWeight];})()",
          ),
          [mode === 'compact' ? '12px' : '13px', '400'],
          `${theme}/${mode} 摘要字号或字重`,
        );
        assert.equal(
          await evaluate("document.querySelector('.item').getBoundingClientRect().height"),
          { list: 64, cards: 104, compact: 58 }[mode],
          mode,
        );
        assert.deepEqual(
          await evaluate(
            "(()=>{const {width,height}=document.querySelector('.item.selected .record-thumbnail').getBoundingClientRect();return [width,height];})()",
          ),
          { list: [52, 42], cards: [40, 40], compact: [44, 36] }[mode],
          `${theme}/${mode} 缩略图尺寸`,
        );
        assert.equal(
          await evaluate("!!document.querySelector('.main-statusbar .settings-toggle')"),
          mode === 'compact',
        );
        assert(
          await evaluate(`(() => {
            const panel = document.querySelector('.main-content');
            const frame = panel.getBoundingClientRect();
            const body = document.querySelector('.app-body').getBoundingClientRect();
            const footer = document.querySelector('.main-statusbar').getBoundingClientRect();
            return !document.querySelector('.search-tools .settings-toggle') &&
              getComputedStyle(panel).borderRadius === '8px' &&
              body.right - frame.right === 8 && footer.top - frame.bottom === 8 &&
              (${mode !== 'compact'} || frame.left - body.left === 8);
          })()`),
          `${theme}/${mode} 内容框未与窗口边缘和底栏留出间距`,
        );
        assert(
          await evaluate(
            "/^\\d{2}:\\d{2}$/.test(document.querySelector('.record-time').textContent.trim())",
          ),
        );
        assert(await evaluate("!document.querySelector('[data-id=row-59] .record-time')"));
        const selectedFill = theme === 'dark' ? 'rgb(48, 72, 67)' : 'rgb(227, 240, 236)';
        await until(
          () =>
            evaluate(
              `getComputedStyle(document.querySelector('.item.selected')).backgroundColor===${JSON.stringify(selectedFill)}`,
            ),
          '选中填充未达到目标颜色',
        );
        await capture('fluent-' + theme + '-' + mode);
      }
    }
  });
  await test('U7 Fluent 左侧分组圆点与选中样式', async () => {
    await metrics();
    await nav();
    for (const theme of ['light', 'dark']) {
      await appearance({ skin: 'fluent', theme });
      assert.equal(await evaluate("document.querySelector('.group-sidebar h2')"), null);
      const dots = () =>
        evaluate(`[...document.querySelectorAll('.group-views [data-group-view] .group-dot')].map(dot => {
          const style = getComputedStyle(dot);
          return {size: [style.width, style.height], radius: style.borderRadius, color: style.backgroundColor};
        })`);
      const markers = await dots();
      assert.equal(markers.length, 3);
      for (const marker of markers) {
        assert.deepEqual(marker.size, ['8px', '8px']);
        assert.equal(marker.radius, '50%');
      }
      assert.equal(new Set(markers.map(({ color }) => color)).size, 3);
      assert(await evaluate("!!document.querySelector('[data-group-view=all] > svg')"));
      assert.deepEqual(
        await evaluate(`(() => {
          const icon = document.querySelector('[data-group-view="all"] > svg');
          return [
            icon.querySelector('path').getAttribute('d'),
            getComputedStyle(icon).width,
            getComputedStyle(icon).height,
            getComputedStyle(icon).strokeWidth,
          ];
        })()`),
        ['m12 3 10 5-10 5L2 8z M2 12l10 5 10-5 M2 16l10 5 10-5', '16px', '16px', '1.5px'],
      );
      assert.equal(
        await evaluate("document.querySelector('[data-group-view=all]').textContent.trim()"),
        '全部分组',
      );
      const group = '[data-group-view="7"]';
      await click(group);
      const selectedFill = theme === 'dark' ? 'rgb(48, 72, 67)' : 'rgb(227, 240, 236)';
      const accent = theme === 'dark' ? 'rgb(131, 213, 195)' : 'rgb(23, 107, 100)';
      await until(
        () =>
          evaluate(
            `getComputedStyle(document.querySelector('${group}')).backgroundColor===${JSON.stringify(selectedFill)}`,
          ),
        '分组未显示选中底色',
      );
      assert.equal(
        await evaluate(
          `getComputedStyle(document.querySelector('${group}[aria-pressed=true]'),'::before').backgroundColor`,
        ),
        accent,
      );
      assert.deepEqual(await dots(), markers);
      await capture('fluent-' + theme + '-groups');
    }
  });
  await test('U7 Fluent 开关与两套皮肤的设置控件', async () => {
    for (const [skin, width, theme] of [
      ['fluent', 1000, 'light'],
      ['fluent', 390, 'dark'],
      ['clipraft', 1000, 'light'],
      ['clipraft', 390, 'dark'],
    ]) {
      await metrics(width);
      await nav();
      await appearance({ skin, theme, compact: width === 390 });
      await click('.settings-toggle');
      await until(() => evaluate("!!document.querySelector('.settings-view form')"), '设置未打开');
      if (skin === 'fluent')
        assert(
          await evaluate(`(() => {
          const select = document.querySelector('select[aria-label=主窗口唤出位置]');
          const center = element => {
            const bounds = element.getBoundingClientRect();
            return bounds.y + bounds.height / 2;
          };
          return Math.abs(center(select) - center(select.previousElementSibling)) < 1;
        })()`),
          `${theme} 主窗口唤出位置与下拉框未垂直居中`,
        );
      assert.deepEqual(
        await evaluate(`[...document.querySelectorAll('.settings-content label.check > input')].map(control => {
          const style = getComputedStyle(control);
          const aligned = control.role === 'switch'
            ? style.flexDirection === 'row' && ['flex-start', 'flow-start'].includes(style.justifyContent)
            : style.justifyContent === 'center';
          return [control.getAttribute('role'), style.width, style.height, style.borderRadius,
            getComputedStyle(control, '::before').borderRadius, aligned];
        })`),
        Array.from({ length: 8 }, () =>
          skin === 'fluent'
            ? ['switch', '39px', '20px', '20px', '50%', true]
            : [null, '15px', '15px', '4px', '0px', true],
        ),
      );
      const focused =
        skin === 'fluent'
          ? '[data-category=general] [role=switch]'
          : '[data-category=general] input[type=checkbox]';
      const before = await evaluate(
        `document.querySelector('${focused}').focus();document.activeElement.checked`,
      );
      const space = { key: ' ', code: 'Space', windowsVirtualKeyCode: 32 };
      await send('Input.dispatchKeyEvent', { type: 'keyDown', ...space, text: ' ' });
      await send('Input.dispatchKeyEvent', { type: 'keyUp', ...space });
      assert.equal(await evaluate('document.activeElement.checked'), !before);
      assert.equal(await evaluate('window.settingUpdates.length'), 0);
      await pause(120);
      if (skin === 'fluent')
        assert.equal(
          await evaluate(
            "new DOMMatrix(getComputedStyle(document.activeElement, '::before').transform).m41",
          ),
          19,
        );
      await capture(`${skin}-settings-${theme}-switches`);
      for (const [category, label] of [
        ['general', '通用'],
        ['privacy', '隐私'],
        ['appearance', '外观'],
        ['about', '关于'],
      ]) {
        if (skin !== 'fluent') continue;
        if (width === 390) await change('select[aria-label=设置分类]', category);
        else
          await evaluate(
            `[...document.querySelectorAll('.settings-nav button')].find(n => n.textContent.trim() === ${JSON.stringify(label)}).click()`,
          );
        assert(
          await evaluate(`(() => {
            const section = document.querySelector('[data-category=${category}]');
            const controls = [...section.querySelectorAll('[role=switch]')];
            return controls.length > 0 && controls.every(control => {
              const row = control.closest('label');
              const style = getComputedStyle(row);
              const frame = row.getBoundingClientRect();
              const bounds = control.getBoundingClientRect();
              const right = frame.right - parseFloat(style.borderRightWidth) - parseFloat(style.paddingRight);
              return bounds.width > 0 && Math.abs(right - bounds.right) < 1;
            });
          })()`),
          `${skin}/${theme}/${category} 的开关未对齐控件列`,
        );
        if (category !== 'appearance') continue;
        const labels = await evaluate(`[...document.querySelectorAll(
          '[data-category="appearance"] .appearance-label-copy',
        )].map((label) => {
          const title = document.createRange();
          title.selectNode(label.firstChild);
          const bounds = title.getBoundingClientRect();
          const description = label.querySelector('small').getBoundingClientRect();
          return {
            title: label.firstChild.textContent.trim(),
            lines: title.getClientRects().length,
            stacked: bounds.bottom <= description.top + 1 &&
              Math.abs(bounds.left - description.left) < 1,
          };
        })`);
        assert.equal(labels.length, 5);
        for (const label of labels) {
          assert.equal(label.lines, 1, `${width}px ${label.title}标题发生折行`);
          assert(label.stacked, `${width}px ${label.title}说明未与标题上下对齐`);
        }
        await capture(`u7-fluent-appearance-labels-${width}`);
      }
    }
  });
  await test('U7 Fluent 材质回退、亚克力表面与减少动效', async () => {
    await metrics();
    await nav();
    await appearance({ skin: 'fluent' });
    await until(
      () => evaluate("document.documentElement.dataset.nativeMaterial==='solid'"),
      '材质回退状态未同步',
    );
    const acrylic = await evaluate(`(() => {
      const menu = document.createElement('div');
      menu.className = 'context-menu';
      document.querySelector('.main-window').append(menu);
      const filter = getComputedStyle(menu).backdropFilter;
      menu.remove();
      return filter;
    })()`);
    assert(acrylic.includes('blur(18px)'), acrylic);
    await evaluate('window.nativeBackdrop = true');
    await appearance({ backdrop_strength: 40 });
    await until(
      () => evaluate("document.documentElement.dataset.nativeMaterial==='host-backdrop'"),
      '主窗材质状态未同步',
    );
    const backdrop = await evaluate(`(() => {
      const frame = getComputedStyle(document.querySelector('.main-window'));
      const pixel = document.createElement('canvas').getContext('2d');
      pixel.fillStyle = frame.backgroundColor;
      pixel.fillRect(0, 0, 1, 1);
      const frameAlpha = pixel.getImageData(0, 0, 1, 1).data[3];
      const content = getComputedStyle(document.querySelector('.main-content'));
      pixel.fillStyle = content.backgroundColor;
      pixel.fillRect(0, 0, 1, 1);
      return {
        root: getComputedStyle(document.documentElement).backgroundColor,
        frameAlpha,
        contentAlpha: pixel.getImageData(0, 0, 1, 1).data[3],
        windowBorder: frame.borderTopColor,
        content: content.backgroundColor,
        dividers: [
          getComputedStyle(document.querySelector('.main-content')).borderTopColor,
          getComputedStyle(document.querySelector('.workspace')).borderTopColor,
          getComputedStyle(document.querySelector('.history-pane')).borderRightColor,
        ],
        structural: [...document.querySelectorAll('.main-window > header, .app-body, .group-sidebar, .main-statusbar, .list, .detail')]
          .map(element => getComputedStyle(element).backgroundColor),
      };
    })()`);
    assert.equal(backdrop.root, 'rgba(0, 0, 0, 0)');
    assert(Math.abs(backdrop.frameAlpha - 218) <= 1);
    assert(Math.abs((255 - backdrop.contentAlpha) * 2 - (255 - backdrop.frameAlpha)) <= 1);
    assert.equal(backdrop.windowBorder, 'rgba(0, 0, 0, 0)');
    assert.deepEqual(backdrop.dividers, Array(3).fill('rgba(0, 0, 0, 0)'));
    assert.deepEqual(backdrop.structural, Array(6).fill('rgba(0, 0, 0, 0)'));
    // 无头浏览器用合成背景演示内容区与外侧的透出差异。
    await evaluate(`document.body.style.backgroundImage =
      'radial-gradient(ellipse at 0% 20%, #5d7782, transparent 68%), linear-gradient(120deg, #c4cdd1, #a5b2a8 70%, #586964)'`);
    await capture('u7-soft-backdrop-main');
    await click('.settings-toggle');
    const settingsBackdrop = await evaluate(`(() => ({
      wrapper: getComputedStyle(document.querySelector('.main-content')).backgroundColor,
      header: getComputedStyle(document.querySelector('.main-window > header')).backgroundColor,
      nav: getComputedStyle(document.querySelector('.settings-nav')).backgroundColor,
      pane: getComputedStyle(document.querySelector('.settings-pane')).backgroundColor,
      paneBorder: getComputedStyle(document.querySelector('.settings-pane')).borderTopColor,
      content: getComputedStyle(document.querySelector('.settings-content')).backgroundColor,
      fieldset: getComputedStyle(document.querySelector('.settings-content fieldset')).backgroundColor,
      slider: document.querySelector('input[type=range][aria-label="背景透出"]')?.value,
      sliderMax: document.querySelector('input[type=range][aria-label="背景透出"]')?.max,
      appearanceIcons: [...document.querySelectorAll('[data-category="appearance"] .appearance-icon-slot')]
        .map((icon) => getComputedStyle(icon).display),
      appearanceIconSlotWidth: getComputedStyle(
        document.querySelector('[data-category="appearance"]'),
      ).getPropertyValue('--appearance-icon-slot-width'),
    }))()`);
    assert.equal(settingsBackdrop.wrapper, 'rgba(0, 0, 0, 0)');
    assert.equal(settingsBackdrop.nav, settingsBackdrop.header);
    assert.equal(settingsBackdrop.nav, 'rgba(0, 0, 0, 0)');
    assert.equal(settingsBackdrop.pane, backdrop.content);
    assert.equal(settingsBackdrop.paneBorder, 'rgba(0, 0, 0, 0)');
    assert.equal(settingsBackdrop.content, 'rgba(0, 0, 0, 0)');
    assert.equal(settingsBackdrop.fieldset, 'rgba(0, 0, 0, 0)');
    assert.equal(settingsBackdrop.slider, '40');
    assert.equal(settingsBackdrop.sliderMax, '100');
    assert.equal(settingsBackdrop.appearanceIcons.length, 4);
    assert(settingsBackdrop.appearanceIcons.every((display) => display === 'none'));
    assert.equal(settingsBackdrop.appearanceIconSlotWidth, '0px');
    await capture('u7-settings-unified-backdrop');
    await evaluate("document.body.style.removeProperty('background-image')");
    await evaluate(
      "[...document.querySelectorAll('.settings-nav button')].find(button => button.textContent.trim() === '外观').click()",
    );
    await evaluate(`(() => {
      const slider = document.querySelector('input[type=range][aria-label="背景透出"]');
      slider.value = '31';
      slider.dispatchEvent(new Event('input', {bubbles:true}));
      slider.dispatchEvent(new Event('change', {bubbles:true}));
    })()`);
    await until(
      () =>
        evaluate(
          "getComputedStyle(document.querySelector('input[type=range][aria-label=\"背景透出\"]')).getPropertyValue('--range-progress') === '31%'",
        ),
      '滑块轨道进度未按百分比同步',
    );
    await evaluate(`(() => {
      const slider = document.querySelector('input[type=range][aria-label="背景透出"]');
      slider.value = '100';
      slider.dispatchEvent(new Event('input', {bubbles:true}));
      slider.dispatchEvent(new Event('change', {bubbles:true}));
    })()`);
    await click('.settings-view button[type=submit]');
    await until(() => evaluate('window.settings.backdrop_strength === 100'), '背景透出未保存');
    await until(
      () =>
        evaluate(
          "document.documentElement.style.getPropertyValue('--fluent-backdrop-opacity') === '36%'",
        ),
      '背景透出未同步到窗口材质',
    );
    assert(
      await evaluate(`(() => {
        const pixel = document.createElement('canvas').getContext('2d');
        pixel.fillStyle = getComputedStyle(document.querySelector('.main-window')).backgroundColor;
        pixel.fillRect(0, 0, 1, 1);
        return pixel.getImageData(0, 0, 1, 1).data[3] === 163;
      })()`),
      '100% 时主窗未保留旧 60 档的底色',
    );
    await evaluate(
      "[...document.querySelectorAll('.settings-nav button')].find(button => button.textContent.trim() === '外观').click()",
    );
    const sliderLayout = await evaluate(`(() => {
      const label = document.querySelector('.backdrop-setting > span:first-child');
      const range = document.querySelector('.backdrop-control input[type=range]');
      const row = document.querySelector('.backdrop-setting').getBoundingClientRect();
      const control = document.querySelector('.backdrop-control').getBoundingClientRect();
      const slider = range.getBoundingClientRect();
      return {
        labelWidth: label.getBoundingClientRect().width,
        labelWhiteSpace: getComputedStyle(label).whiteSpace,
        labelAlign: getComputedStyle(label).alignItems,
        labelTextAlign: getComputedStyle(label).textAlign,
        rowCenter: row.top + row.height / 2,
        controlCenter: control.top + control.height / 2,
        sliderCenter: slider.top + slider.height / 2,
        rangeWidth: range.getBoundingClientRect().width,
        rangeAppearance: getComputedStyle(range).appearance,
        rangeHeight: getComputedStyle(range).height,
        rangePaddingBottom: getComputedStyle(range).paddingBottom,
        rangeBackground: getComputedStyle(range).backgroundColor,
        rangeBorder: getComputedStyle(range).borderWidth,
        appearanceLabelWidths: [
          ...document.querySelectorAll(
            '[data-category="appearance"] .settings-select > span, [data-category="appearance"] .compact-setting > span',
          ),
        ].map((label) => label.getBoundingClientRect().width),
        iconSlot: (() => {
          const appearance = document.querySelector('[data-category="appearance"]');
          const label = appearance.querySelector('.settings-select > .appearance-label');
          const icon = label.querySelector('.appearance-icon-slot');
          appearance.style.setProperty('--appearance-icon-slot-width', '20px');
          appearance.style.setProperty('--appearance-icon-slot-gap', '8px');
          appearance.style.setProperty('--appearance-icon-display', 'block');
          const result = {
            display: getComputedStyle(icon).display,
            columns: getComputedStyle(label).gridTemplateColumns,
            gap: getComputedStyle(label).columnGap,
          };
          appearance.style.removeProperty('--appearance-icon-slot-width');
          appearance.style.removeProperty('--appearance-icon-slot-gap');
          appearance.style.removeProperty('--appearance-icon-display');
          return result;
        })(),
      };
    })()`);
    assert(sliderLayout.labelWidth > 200, JSON.stringify(sliderLayout));
    assert.equal(sliderLayout.labelWhiteSpace, 'nowrap');
    assert.equal(sliderLayout.labelAlign, 'center');
    assert.equal(sliderLayout.labelTextAlign, 'left');
    assert(
      Math.abs(sliderLayout.rowCenter - sliderLayout.controlCenter) < 1,
      JSON.stringify(sliderLayout),
    );
    assert(
      Math.abs(sliderLayout.controlCenter - sliderLayout.sliderCenter) < 1,
      JSON.stringify(sliderLayout),
    );
    assert(sliderLayout.rangeWidth >= 240, JSON.stringify(sliderLayout));
    assert.equal(sliderLayout.rangeAppearance, 'none', JSON.stringify(sliderLayout));
    assert.equal(sliderLayout.rangeHeight, '24px', JSON.stringify(sliderLayout));
    assert.equal(sliderLayout.rangePaddingBottom, '0px', JSON.stringify(sliderLayout));
    assert.equal(sliderLayout.rangeBackground, 'rgba(0, 0, 0, 0)', JSON.stringify(sliderLayout));
    assert.equal(sliderLayout.rangeBorder, '0px', JSON.stringify(sliderLayout));
    assert(
      sliderLayout.appearanceLabelWidths.every((width) => width > 200),
      JSON.stringify(sliderLayout.appearanceLabelWidths),
    );
    assert.equal(sliderLayout.iconSlot.display, 'block');
    assert(
      sliderLayout.iconSlot.columns.startsWith('20px '),
      JSON.stringify(sliderLayout.iconSlot),
    );
    assert.equal(sliderLayout.iconSlot.gap, '8px');
    await capture('u7-fluent-backdrop-slider');
    await evaluate("document.documentElement.dataset.nativeMaterial = 'solid'");
    assert.deepEqual(
      await evaluate(`[
        getComputedStyle(document.querySelector('.main-content')).animationName,
        getComputedStyle(document.querySelector('.settings-view')).animationName,
        getComputedStyle(document.querySelector('.settings-content fieldset')).animationName,
        getComputedStyle(document.querySelector('.settings-content fieldset')).transitionDuration
      ]`),
      ['none', 'none', 'none', '0s'],
    );
    await send('Emulation.setEmulatedMedia', {
      features: [{ name: 'prefers-reduced-motion', value: 'reduce' }],
    });
    assert.equal(
      await evaluate(
        "getComputedStyle(document.querySelector('.settings-content fieldset')).transitionDuration",
      ),
      '0s',
    );
    await send('Emulation.setEmulatedMedia', {
      features: [{ name: 'prefers-reduced-motion', value: 'no-preference' }],
    });
  });
  await test('U7 Fluent 主窗图片文件详情可滚动查看长图', async () => {
    await metrics();
    await nav('?fluent-fixture');
    await appearance({ skin: 'fluent' });
    await evaluate(`(() => {
      const original = window.__TAURI_INTERNALS__.convertFileSrc;
      window.__TAURI_INTERNALS__.convertFileSrc = hash => hash === 'tall'
        ? 'data:image/svg+xml,' + encodeURIComponent('<svg xmlns="http://www.w3.org/2000/svg" width="160" height="800"><rect width="160" height="800" fill="#277d77"/></svg>')
        : original(hash);
      window.rows[2] = {...window.rows[2], thumbnail: 'tall', preview: 'tall'};
      window.rows[1] = {
        ...window.rows[1],
        kind: 'files',
        summary: '甲 乙,丙.txt, 第二张.png',
        files: ['C:/synthetic/甲 乙,丙.txt', 'C:/synthetic/第二张.png'],
        thumbnail: 'tall',
        preview: 'tall'
      };
      const originalInvoke = window.__TAURI_INTERNALS__.invoke;
      window.__TAURI_INTERNALS__.invoke = (cmd, args) => cmd === 'item_detail'
        ? Promise.resolve({text:'',files:window.rows[1].files.map(path => ({path,exists:true})),truncated:false})
        : originalInvoke(cmd, args);
      window.emitEvent('history-changed');
    })()`);
    await click('[data-id="row-1"]');
    await until(
      () => evaluate("document.querySelector('.detail img')?.naturalHeight === 800"),
      '高图片预览未解码',
    );
    const preview = await evaluate(`(() => ({
      title: document.querySelector('.files-preview-title')?.textContent.trim(),
      fileNames: [...document.querySelectorAll('.file-copy strong')].map(node => node.textContent),
      parents: [...document.querySelectorAll('.file-parent')].map(node => node.textContent),
      detailImage: document.querySelector('.detail .large-preview img')?.getBoundingClientRect().height,
      previewCardBottom: document.querySelector('.files-preview-content').getBoundingClientRect().bottom,
      lastFileBottom: document.querySelector('.file-entry:last-child').getBoundingClientRect().bottom,
      detailOverflow: getComputedStyle(document.querySelector('.detail')).overflow,
      scrollOverflow: getComputedStyle(document.querySelector('.detail-scroll')).overflow,
      imageMaxHeight: getComputedStyle(document.querySelector('.detail .large-preview img')).maxHeight,
      frameMaxHeight: getComputedStyle(document.querySelector('.detail .large-preview')).maxHeight,
      scrollHeight: document.querySelector('.detail-scroll').scrollHeight,
      clientHeight: document.querySelector('.detail-scroll').clientHeight,
      cardOverflow: getComputedStyle(document.querySelector('.preview-content')).overflow
    }))()`);
    assert.equal(preview.title, '2 个项目');
    assert.deepEqual(preview.fileNames, ['甲 乙,丙.txt', '第二张.png']);
    assert.deepEqual(preview.parents, ['C:/synthetic', 'C:/synthetic']);
    assert(preview.detailImage > 320, JSON.stringify(preview));
    assert(preview.previewCardBottom >= preview.lastFileBottom, JSON.stringify(preview));
    assert.equal(preview.detailOverflow, 'hidden');
    assert.equal(preview.scrollOverflow, 'auto');
    assert.equal(preview.imageMaxHeight, 'none');
    assert.equal(preview.frameMaxHeight, 'none');
    assert(preview.scrollHeight > preview.clientHeight, JSON.stringify(preview));
    assert.equal(preview.cardOverflow, 'visible');
    await capture('u7-fluent-files-preview');
    await click('[data-id="row-2"]');
    await until(
      () => evaluate("document.querySelector('.detail img')?.naturalHeight === 800"),
      '图片记录的长图未解码',
    );
    const image = await evaluate(`(() => {
      const scroll = document.querySelector('.detail-scroll');
      scroll.scrollTop = scroll.scrollHeight;
      const card = document.querySelector('.preview-content').getBoundingClientRect();
      const frame = document.querySelector('.detail .large-preview').getBoundingClientRect();
      return {cardBottom: card.bottom, frameBottom: frame.bottom, scrollBottom: scroll.getBoundingClientRect().bottom, scrolled: scroll.scrollTop};
    })()`);
    assert(image.scrolled > 0, JSON.stringify(image));
    assert(image.cardBottom > image.frameBottom, JSON.stringify(image));
    assert(image.cardBottom <= image.scrollBottom, JSON.stringify(image));
    await capture('u7-fluent-long-image-bottom');
  });
  await test('U7 其余设置分类适配宽窄布局与表单控件', async () => {
    const categories = [
      ['general', '通用', '.check'],
      ['history', '历史记录', 'input[type=number]'],
      ['privacy', '隐私', '.add-app input'],
      ['hotkeys', '快捷键', '.hotkey-record button'],
      ['storage', '数据存储', '.archive-field input'],
    ];
    for (const [width, theme] of [
      [1000, 'light'],
      [390, 'dark'],
    ]) {
      await metrics(width);
      await nav();
      await appearance({ skin: 'fluent', theme, compact: width === 390 });
      await click('.settings-toggle');
      await until(() => evaluate("!!document.querySelector('.settings-view form')"), '设置未打开');
      assert.deepEqual(
        await evaluate(`(() => {
          const frame = document.querySelector('.settings-pane');
          const bounds = frame.getBoundingClientRect();
          const body = document.querySelector('.app-body').getBoundingClientRect();
          const back = document.querySelector('.main-window > header .settings-title-back');
          const style = getComputedStyle(frame);
          return {
            back: !!back?.querySelector('svg') && back.textContent.trim() === '设置',
            sidebarBack: !!document.querySelector('.settings-nav .settings-back'),
            radius: style.borderRadius,
            clipping: style.overflow,
            right: body.right - bounds.right,
            bottom: body.bottom - bounds.bottom,
            left: ${width <= 600} ? bounds.left - body.left : null
          };
        })()`),
        {
          back: true,
          sidebarBack: false,
          radius: '8px',
          clipping: 'hidden',
          right: 8,
          bottom: 8,
          left: width <= 600 ? 8 : null,
        },
        `${theme} 设置标题或内容框不符合预期`,
      );
      for (const [category, label, control] of categories) {
        if (width === 390) await change('select[aria-label=设置分类]', category);
        else
          await evaluate(
            `[...document.querySelectorAll('.settings-nav button')].find(n => n.textContent.trim() === ${JSON.stringify(label)}).click()`,
          );
        assert(
          await evaluate(`(() => {
            const page = document.querySelector('[data-category=${category}]');
            const field = page.querySelector(${JSON.stringify(control)});
            const card = page.querySelector(':scope > label, :scope > section');
            const content = document.querySelector('.settings-content');
            const frame = content.getBoundingClientRect();
            const bounds = field.getBoundingClientRect();
            return bounds.width > 0 && getComputedStyle(card).borderRadius === '4px' &&
              bounds.left >= frame.left && bounds.right <= frame.right &&
              content.scrollWidth <= content.clientWidth;
          })()`),
          `${theme}/${category} 控件溢出或卡片样式未生效`,
        );
        await pause(150);
        await capture(`fluent-settings-${theme}-${category}`);
      }
    }
  });
  await test('U7 设置字号、忽略程序卡片与导出悬停', async () => {
    for (const [width, theme, hover] of [
      [1000, 'light', 'rgb(18, 91, 85)'],
      [390, 'dark', 'rgb(150, 223, 207)'],
    ]) {
      await metrics(width);
      await nav();
      await appearance({ skin: 'fluent', theme, compact: width === 390 });
      await click('.settings-toggle');
      await until(() => evaluate("!!document.querySelector('.settings-view form')"), '设置未打开');
      assert.deepEqual(
        await evaluate(`[
          '[data-category=general] .check > span',
          '[data-category=general] .number-field > span',
          '.compact-setting > span',
          '.appearance-section .number-field > span'
        ].map(selector => {
          const style = getComputedStyle(document.querySelector(selector));
          return [style.fontSize, style.fontWeight];
        })`),
        Array.from({ length: 4 }, () => ['14px', '400']),
      );
      const selectCategory = (category, label) =>
        width === 390
          ? change('select[aria-label=设置分类]', category)
          : evaluate(
              `[...document.querySelectorAll('.settings-nav button')].find(n => n.textContent.trim() === ${JSON.stringify(label)}).click()`,
            );
      await selectCategory('privacy', '隐私');
      assert(
        await evaluate(`(() => {
          const rows = [...document.querySelectorAll('.ignored-apps > li')];
          return rows.length > 0 && rows.every(row => {
            const style = getComputedStyle(row);
            return style.borderRadius === '4px' && style.borderTopWidth === '1px' &&
              style.borderBottomWidth === '1px';
          });
        })()`),
      );
      await selectCategory('storage', '数据存储');
      await pause(150);
      const point = await evaluate(`(() => {
        const button = document.querySelector('[data-start-archive]');
        button.scrollIntoView({block:'center'});
        const rect = button.getBoundingClientRect();
        return {x: rect.left + rect.width / 2, y: rect.top + rect.height / 2};
      })()`);
      const foreground = await evaluate(
        "getComputedStyle(document.querySelector('[data-start-archive]')).color",
      );
      await send('Input.dispatchMouseEvent', { type: 'mouseMoved', ...point });
      await until(
        () =>
          evaluate(
            `getComputedStyle(document.querySelector('[data-start-archive]:hover')).backgroundColor===${JSON.stringify(hover)}`,
          ),
        `${theme} 开始导出的悬停背景未使用主按钮色`,
      );
      assert.equal(
        await evaluate("getComputedStyle(document.querySelector('[data-start-archive]')).color"),
        foreground,
      );
      await capture(`fluent-settings-${theme}-archive-hover`);
      await selectCategory('general', '通用');
      await click('[data-category=general] .check input');
      await click('.settings-title-back');
      await until(() => evaluate("!document.querySelector('.settings-view')"), '未返回列表');
      assert.equal(await evaluate('window.settingUpdates.length'), 0);
    }
  });
  await test('U7 删除确认焦点与只读浮窗表面', async () => {
    await metrics();
    await nav();
    await appearance({ skin: 'fluent' });
    await evaluate(
      "document.querySelectorAll('.item')[1].dispatchEvent(new MouseEvent('click',{ctrlKey:true,bubbles:true}));document.querySelector('.item').focus()",
    );
    await evaluate(
      "document.activeElement.dispatchEvent(new MouseEvent('contextmenu',{bubbles:true,cancelable:true,clientX:260,clientY:180}))",
    );
    assert(
      await evaluate(
        "document.querySelector('.context-menu button:last-child').matches('.delete-record')",
      ),
    );
    await click('.delete-record');
    await until(
      () => evaluate("document.activeElement.matches('.cancel-delete')"),
      '确认未聚焦取消',
    );
    await send('Input.dispatchKeyEvent', {
      type: 'keyDown',
      key: 'Enter',
      code: 'Enter',
      windowsVirtualKeyCode: 13,
      text: '\r',
    });
    await send('Input.dispatchKeyEvent', {
      type: 'keyUp',
      key: 'Enter',
      code: 'Enter',
      windowsVirtualKeyCode: 13,
    });
    await until(() => evaluate("!document.querySelector('.delete-confirm')"), 'Enter 未激活取消');
    assert(await evaluate("document.activeElement.matches('.item')"));
    await floatingNav();
    await appearance({ skin: 'fluent', theme: 'dark' });
    await evaluate(
      `window.emitEvent('compact-preview-changed',${JSON.stringify(floatingContent())})`,
    );
    await until(() => evaluate("!!document.querySelector('.floating-title')"), '浮窗标题未显示');
    assert.equal(
      await evaluate(
        "getComputedStyle(document.querySelector('.floating-preview')).backgroundColor",
      ),
      'rgb(44, 55, 53)',
    );
    const floating = await evaluate(`({
      margins: [document.documentElement, document.body, document.querySelector('#app'), document.querySelector('.floating-shell')]
        .map(element => getComputedStyle(element).backgroundColor),
      materialCalls: window.calls.filter(call => call.cmd === 'sync_window_material').length,
    })`);
    assert.deepEqual(floating.margins, Array(4).fill('rgba(0, 0, 0, 0)'));
    assert.equal(floating.materialCalls, 0);
    await pause(180);
    await capture('fluent-dark-floating');
  });
  await test('U6 三窗口迟到外观快照不覆盖已收到的新设置', async () => {
    for (const panel of ['', 'quick', 'preview']) {
      await send('Page.navigate', { url: origin + '/?panel=' + panel + '&holdappearance=1' });
      await until(() => evaluate("typeof window.releaseAppearance==='function'"), '未挂起外观快照');
      await evaluate(
        "window.emitEvent('settings-changed',{...window.settings,skin:'fluent',theme:'dark'});window.releaseAppearance()",
      );
      await pause(150);
      assert.equal(await evaluate('document.documentElement.dataset.theme'), 'dark', panel);
      assert.equal(await evaluate('document.documentElement.dataset.skin'), 'fluent', panel);
    }
  });
  await test('U6 角色令牌变更保留选择与滚动，禁用按钮不产生悬停反馈', async () => {
    await metrics();
    await nav();
    await click('[data-id="row-12"]');
    const id = await selected();
    const scroll = await evaluate('document.querySelector(".list").scrollTop');
    await evaluate(
      "document.documentElement.style.setProperty('--radius-search','13px');document.documentElement.style.setProperty('--selected-bg','rgb(11, 22, 33)')",
    );
    assert.equal(
      await evaluate("getComputedStyle(document.querySelector('.searchbox')).borderRadius"),
      '13px',
    );
    assert.equal(
      await evaluate("getComputedStyle(document.querySelector('.item.selected')).backgroundColor"),
      'rgb(11, 22, 33)',
    );
    assert.equal(await selected(), id);
    assert.equal(await evaluate('document.querySelector(".list").scrollTop'), scroll);
    await evaluate(
      "document.documentElement.removeAttribute('style');document.querySelector('.preview-refresh').disabled=true",
    );
    const style = () =>
      evaluate(
        "(()=>{const s=getComputedStyle(document.querySelector('.preview-refresh'));return [s.backgroundColor,s.borderColor,s.color]})()",
      );
    const before = await style();
    await send('DOM.enable');
    await send('CSS.enable');
    const { root } = await send('DOM.getDocument');
    const { nodeId } = await send('DOM.querySelector', {
      nodeId: root.nodeId,
      selector: '.preview-refresh',
    });
    await send('CSS.forcePseudoState', { nodeId, forcedPseudoClasses: ['hover'] });
    assert.deepEqual(await style(), before);
    await send('CSS.forcePseudoState', { nodeId, forcedPseudoClasses: [] });
  });
  const prepareWallpaper = (layout = 'regular') =>
    evaluate(`(() => {
      const compact = ${JSON.stringify(layout)} === 'compact';
      const canvas = document.createElement('canvas');
      canvas.width = compact ? 390 : 800;
      canvas.height = compact ? 700 : 560;
      const ctx = canvas.getContext('2d');
      ctx.scale(canvas.width / 800, canvas.height / 560);
      const gradient = ctx.createLinearGradient(0, 0, 800, 560);
      gradient.addColorStop(0, compact ? '#158568' : '#172038');
      gradient.addColorStop(.38, '#926ba7');
      gradient.addColorStop(.7, '#e28cba');
      gradient.addColorStop(1, '#faf1df');
      ctx.fillStyle = gradient;
      ctx.fillRect(0, 0, 800, 560);
      ctx.strokeStyle = '#22273e';
      ctx.lineWidth = 3;
      for (let x = -100; x < 900; x += 95) {
        ctx.beginPath();
        ctx.moveTo(x, 0);
        ctx.lineTo(x + 260, 560);
        ctx.stroke();
      }
      window.backgroundImport = {jpeg: canvas.toDataURL('image/jpeg').split(',')[1],
        options: {fit: 'cover', position: 'center', scrim: 78}};
    })()`);
  const backgroundSection = '.background-settings';
  const wallpaperButton = (label) =>
    evaluate(`Array.from(document.querySelectorAll('${backgroundSection} button'))
      .find(button => button.textContent.trim() === ${JSON.stringify(label)}).click()`);
  const openWallpaperSettings = async () => {
    await evaluate("window.emitEvent('open-settings')");
    await until(() => evaluate("!!document.querySelector('.settings-nav')"), '设置页未加载');
    await click('.settings-nav button:nth-child(2)');
  };
  const wallpaperStyle = (selector, property = 'backgroundImage') =>
    evaluate(
      `getComputedStyle(document.querySelector(${JSON.stringify(selector)}))[${JSON.stringify(property)}]`,
    );

  const assertWallpaperSettings = async () => {
    assert.equal((await wallpaperStyle('.main-window')) === 'none', true, '设置页仍绘制壁纸');
    assert.equal(await wallpaperStyle('.settings-nav', 'backgroundColor'), 'rgba(0, 0, 0, 0)');
    assert.equal(
      await wallpaperStyle('.settings-content', 'backgroundColor'),
      await wallpaperStyle('.settings-actions', 'backgroundColor'),
    );
    assert.equal(
      await wallpaperStyle('.settings-content fieldset', 'backgroundColor'),
      'rgba(0, 0, 0, 0)',
    );
  };

  await test('U8 背景图片导入、调整、失败回退和移除', async () => {
    await metrics();
    await nav();
    await prepareWallpaper();
    const section = backgroundSection;
    const preview = `${section} .background-preview`;
    await openWallpaperSettings();
    await wallpaperButton('选择图片');
    await until(
      () => evaluate(`!!document.querySelector('${preview}.has-wallpaper')`),
      '设置页未绘制背景预览',
    );
    assert.match(await wallpaperStyle(preview), /data:image\/jpeg/);
    await change(`${section} select:nth-of-type(1)`, 'contain');
    await change(`${section} label:nth-of-type(2) select`, 'bottom');
    await evaluate(`(() => {
      const slider = document.querySelector('${section} input[type=range]');
      slider.value = '71';
      slider.dispatchEvent(new Event('input', { bubbles: true }));
    })()`);
    await wallpaperButton('应用背景调整');
    await until(
      () => evaluate("window.backgroundAssets.regular.options.position === 'bottom'"),
      '调整未保存',
    );
    assert.equal(await wallpaperStyle(preview, 'backgroundSize'), 'auto, contain');
    assert.equal(await evaluate('window.backgroundAssets.regular.options.scrim'), 71);
    const previous = await wallpaperStyle(preview);
    await evaluate('window.backgroundImportError=true');
    await wallpaperButton('替换图片');
    await until(
      () => evaluate(`!!document.querySelector('${section} [role=alert]')`),
      '损坏图片无反馈',
    );
    assert.equal(await wallpaperStyle(preview), previous);
    await evaluate('window.backgroundImportError=false;window.backgroundImport=null');
    await wallpaperButton('替换图片');
    assert.equal(await wallpaperStyle(preview), previous);

    await evaluate('window.nativeBackdrop=true');
    await appearance({ skin: 'fluent', theme: 'dark' });
    assert.equal(await evaluate('document.documentElement.dataset.nativeMaterial'), 'solid');
    assert.equal(await evaluate('window.lastMaterialRequest.customBackground'), true);
    await evaluate(`document.querySelector('${section}').scrollIntoView()`);
    await capture('u8-background-settings');
    await send('Emulation.setEmulatedMedia', {
      features: [{ name: 'forced-colors', value: 'active' }],
    });
    try {
      assert.equal(await wallpaperStyle(preview), 'none');
    } finally {
      await send('Emulation.setEmulatedMedia', { features: [] });
    }
    await wallpaperButton('移除图片');
    await until(
      () => evaluate("document.documentElement.dataset.nativeMaterial === 'host-backdrop'"),
      'Fluent 桌面背景未恢复',
    );
    assert.equal(await wallpaperStyle('.main-window'), 'none');
    assert.equal(await evaluate('window.backgroundAssets.regular'), null);
  });
  await test('U8 壁纸与两套皮肤的设置底板、历史分层和三种布局', async () => {
    await metrics();
    await nav();
    await prepareWallpaper();
    await openWallpaperSettings();
    await wallpaperButton('选择图片');
    await until(
      () =>
        evaluate("document.querySelector('.main-window').hasAttribute('data-custom-background')"),
      '背景图片未生效',
    );
    await click('.background-target[data-layout=compact]');
    await prepareWallpaper('compact');
    await wallpaperButton('选择图片');
    await until(() => evaluate('!!window.backgroundAssets.compact'), '紧凑背景未保存');
    for (const skin of ['clipraft', 'fluent']) {
      await appearance({ skin, theme: 'light', compact: false });
      await click('.settings-nav button:nth-child(1)');
      await capture(`u8-wallpaper-${skin}-settings`);
      await assertWallpaperSettings();
      await evaluate("document.querySelector('.settings-buttons button[type=button]').click()");
      await until(() => evaluate("!!document.querySelector('.workspace')"), '未返回历史页');
      for (const theme of ['light', 'dark']) {
        for (const [layout, compact] of [
          ['list', false],
          ['cards', false],
          ['list', true],
        ]) {
          await metrics(compact ? 390 : 1000);
          await appearance({ theme, layout, compact });
          if (theme === 'light' && layout === 'list' && !compact) {
            await capture(`u8-wallpaper-${skin}-history`);
          }
          assert.match(await wallpaperStyle('.main-window'), /data:image\/jpeg/);
          assert(
            await evaluate(`(() => {
              const style = getComputedStyle(document.querySelector('.main-window'));
              return style.backgroundImage.includes(window.backgroundAssets.${compact ? 'compact' : 'regular'}.jpeg);
            })()`),
          );
          assert(await evaluate("!!document.querySelector('.item.selected')"));
          const structuralBorders = await evaluate(`(() => {
            const elements = document.querySelectorAll('.main-window, .main-window > header, .main-content, .workspace, .group-sidebar, .history-pane, .list, .detail-bottom, .preview-content, .main-statusbar');
            return [...elements].filter(element => {
              const style = getComputedStyle(element);
              return ['Top', 'Right', 'Bottom', 'Left'].some(side => parseFloat(style['border' + side + 'Width']) > 0
                && style['border' + side + 'Color'] !== 'rgba(0, 0, 0, 0)');
            }).map(element => element.className);
          })()`);
          assert.deepEqual(structuralBorders, []);
          const alpha = await evaluate(`(() => {
            const ctx = document.createElement('canvas').getContext('2d');
            const selectors = ['.main-content', '.item:not(.selected, :hover)', '.item.selected'];
            if (!${compact}) selectors.push(${JSON.stringify(skin === 'fluent' ? '.preview-content' : '.detail')});
            return selectors.map(selector => {
              ctx.clearRect(0, 0, 1, 1);
              ctx.fillStyle = getComputedStyle(document.querySelector(selector)).backgroundColor;
              ctx.fillRect(0, 0, 1, 1);
              return ctx.getImageData(0, 0, 1, 1).data[3];
            });
          })()`);
          assert.deepEqual(
            alpha,
            compact ? [71, 219, 255] : [71, 219, 255, 199],
            `${skin}/${theme}/${compact ? 'compact' : layout}`,
          );
        }
      }
      await send('Emulation.setEmulatedMedia', {
        features: [{ name: 'forced-colors', value: 'active' }],
      });
      try {
        assert.equal(await wallpaperStyle('.main-window'), 'none');
      } finally {
        await send('Emulation.setEmulatedMedia', { features: [] });
      }
      await metrics();
      await openWallpaperSettings();
      await click('.settings-nav button:nth-child(1)');
      await assertWallpaperSettings();
    }
  });
  await test('U8 普通和紧凑背景独立配置、显示和移除', async () => {
    await metrics();
    await nav();
    await appearance({ skin: 'fluent' });
    await prepareWallpaper();
    await openWallpaperSettings();
    assert(
      await evaluate(`(() => {
        const backgrounds = document.querySelector('.background-settings');
        return backgrounds.querySelector('h3').textContent === '自定义背景'
          && backgrounds.previousElementSibling.getAttribute('aria-labelledby') === 'appearance-theme-heading'
          && backgrounds.nextElementSibling.getAttribute('aria-labelledby') === 'appearance-layout-heading';
      })()`),
    );
    assert.equal(
      await wallpaperStyle('.background-controls', 'backgroundColor'),
      await wallpaperStyle('.appearance-section > label.settings-select', 'backgroundColor'),
    );
    assert(
      await evaluate(`(() => {
        const headings = [...document.querySelectorAll('[data-category=appearance] h3')];
        const fonts = headings.map(heading => {
          const {fontFamily, fontSize, fontWeight, fontStretch, letterSpacing} = getComputedStyle(heading);
          return [fontFamily, fontSize, fontWeight, fontStretch, letterSpacing].join('|');
        });
        return fonts.length === 3 && new Set(fonts).size === 1;
      })()`),
      '自定义背景标题的字号、字重和字宽应与主题、布局一致',
    );
    await wallpaperButton('选择图片');
    await until(() => evaluate('!!window.backgroundAssets.regular'), '普通背景未保存');
    await evaluate("document.querySelector('.settings-buttons button[type=button]').click()");
    await evaluate('window.nativeBackdrop=true');
    await appearance({ compact: true });
    assert.equal(await wallpaperStyle('.main-window'), 'none');
    assert.equal(await evaluate('window.lastMaterialRequest.customBackground'), false);
    await openWallpaperSettings();
    assert.equal(
      await evaluate(
        "document.querySelector('.background-target[aria-pressed=true]').dataset.layout",
      ),
      'compact',
    );
    await prepareWallpaper('compact');
    await wallpaperButton('选择图片');
    await until(
      () => evaluate("!!document.querySelector('[data-layout=compact] .has-wallpaper')"),
      '竖图未预览',
    );
    await change('.background-controls select', 'contain');
    await wallpaperButton('应用背景调整');
    await until(
      () => evaluate("window.backgroundAssets.compact.options.fit === 'contain'"),
      '紧凑布局调整未保存',
    );
    assert.equal(await evaluate('window.backgroundAssets.regular.options.fit'), 'cover');
    await evaluate("document.querySelector('.settings-content').scrollTop = 0");
    await capture('u8-background-layouts');
    await metrics(390);
    await capture('u8-background-layouts-narrow');
    assert.equal(
      await evaluate(`(() => {
        const pane = document.querySelector('.settings-content');
        return pane.scrollWidth > pane.clientWidth;
      })()`),
      false,
    );
    await click('.background-target[data-layout=regular]');
    await wallpaperButton('移除图片');
    await until(() => evaluate('window.backgroundAssets.regular === null'), '普通背景未移除');
    await evaluate("document.querySelector('.settings-buttons button[type=button]').click()");
    assert.match(await wallpaperStyle('.main-window'), /data:image\/jpeg/);
    assert.equal(await wallpaperStyle('.main-window', 'backgroundSize'), 'auto, contain');
    await capture('u8-wallpaper-portrait');
    await metrics();
    await appearance({ compact: false });
    assert.equal(await wallpaperStyle('.main-window'), 'none');
  });
  await test('U8 壁纸贴合窗口上下沿，无底色细线', async () => {
    await metrics();
    await nav();
    await evaluate(`(() => {
      const canvas = document.createElement('canvas');
      canvas.width = 320;
      canvas.height = 160;
      const ctx = canvas.getContext('2d');
      ctx.fillStyle = '#154886';
      ctx.fillRect(0, 0, canvas.width, canvas.height);
      window.backgroundImport = {jpeg: canvas.toDataURL('image/jpeg').split(',')[1],
        options: {fit: 'cover', position: 'center', scrim: 78}};
    })()`);
    await openWallpaperSettings();
    await wallpaperButton('选择图片');
    await until(() => evaluate('!!window.backgroundAssets.regular'), '普通背景未保存');
    await click('.background-target[data-layout=compact]');
    await wallpaperButton('选择图片');
    await until(() => evaluate('!!window.backgroundAssets.compact'), '紧凑背景未保存');
    await evaluate("document.querySelector('.settings-buttons button[type=button]').click()");
    await until(() => evaluate("!!document.querySelector('.workspace')"), '未返回历史页');

    const edgePixels = async (x, y) => {
      const shot = await send('Page.captureScreenshot', {
        format: 'png',
        clip: { x, y, width: 1, height: 5, scale: 1 },
      });
      return evaluate(`(async () => {
        const image = new Image();
        image.src = ${JSON.stringify('data:image/png;base64,' + shot.data)};
        await image.decode();
        const canvas = document.createElement('canvas');
        canvas.width = 1;
        canvas.height = 5;
        const ctx = canvas.getContext('2d');
        ctx.drawImage(image, 0, 0);
        const row = y => [...ctx.getImageData(0, y, 1, 1).data].slice(0, 3);
        return [row(0), row(4)];
      })()`);
    };
    const assertContinuousEdges = async (x, description) => {
      for (const y of [0, 695]) {
        const [first, last] = await edgePixels(x, y);
        assert(
          first.every((channel, index) => Math.abs(channel - last[index]) <= 2),
          `${description} y=${y}: ${first} 与 ${last}`,
        );
      }
    };
    for (const [width, compact] of [
      [1000, false],
      [390, true],
    ]) {
      await metrics(width);
      for (const skin of ['fluent', 'clipraft']) {
        for (const theme of ['light', 'dark']) {
          await appearance({ skin, theme, compact });
          await assertContinuousEdges(Math.floor(width / 2), `${skin}/${theme}/${width}`);
        }
      }
    }
    await metrics();
    await appearance({ skin: 'fluent', theme: 'dark', compact: false });
    await openWallpaperSettings();
    await assertContinuousEdges(500, 'Fluent 深色设置页');
  });
  const setSystemScheme = (value) =>
    send('Emulation.setEmulatedMedia', { features: [{ name: 'prefers-color-scheme', value }] });
  const assertColorScheme = (scheme) =>
    until(
      () => evaluate(`document.documentElement.dataset.theme === ${JSON.stringify(scheme)}`),
      '窗口未切换至' + scheme,
    );
  await test('0.8.1 系统明暗同步三个窗口与 Fluent 材质，固定配色仍优先', async () => {
    try {
      for (const panel of ['main', 'quick', 'preview']) {
        await setSystemScheme('light');
        await metrics();
        if (panel === 'preview') await floatingNav('&system-theme=1');
        if (panel !== 'preview') await nav(`?panel=${panel}&system-theme=1`);
        await assertColorScheme('light');
        await setSystemScheme('dark');
        await assertColorScheme('dark');
        if (panel === 'main') {
          await until(
            () => evaluate("window.lastMaterialRequest?.colorScheme === 'dark'"),
            'Fluent 原生材质未收到深色',
          );
        }
        await setSystemScheme('light');
        await assertColorScheme('light');
        await appearance({ theme: 'dark' });
        await assertColorScheme('dark');
        await setSystemScheme('dark');
        await setSystemScheme('light');
        await pause(100);
        assert.equal(await evaluate('document.documentElement.dataset.theme'), 'dark', panel);
      }
    } finally {
      await send('Emulation.setEmulatedMedia', { features: [] });
    }
  });
  const openCategory = async (label) => {
    await click('.settings-toggle');
    await until(() => evaluate("!!document.querySelector('.settings-nav')"), '设置分类未显示');
    await evaluate(
      `[...document.querySelectorAll('.settings-nav button')].find(button => button.textContent.trim() === ${JSON.stringify(label)}).click()`,
    );
  };
  const aboutStatus = (text) =>
    until(
      () =>
        evaluate(`(() => {
        const about = document.querySelector('[data-category=about]');
        return about?.checkVisibility() && about.innerText.includes(${JSON.stringify(text)});
      })()`),
      '关于页面未显示：' + text,
    );
  const assertAboutCard = async () =>
    assert(
      await evaluate(`(() => {
        const card = document.querySelector('.about-release-card');
        const name = card.querySelector('strong').getBoundingClientRect();
        const version = card.querySelector('.about-version span').getBoundingClientRect();
        const button = card.querySelector('.about-check').getBoundingClientRect();
        const bounds = card.getBoundingClientRect();
        const style = getComputedStyle(card);
        const right = bounds.right - parseFloat(style.paddingRight) - parseFloat(style.borderRightWidth);
        return style.borderRightStyle === 'solid' && version.top >= name.bottom &&
          button.left >= Math.max(name.right, version.right) && Math.abs(button.right - right) <= 1;
      })()`),
      '名称、版本和检查按钮未按卡片左右布局对齐',
    );
  const assertThemeChoices = async () => {
    const theme = await evaluate(`(() => {
      const select = document.querySelector('[data-category=appearance] select[aria-label=明暗], [data-category=appearance] select[aria-label=主题]');
      return {
        options: [...select.options].map(option => [option.value, option.textContent]),
        extra: [...select.childNodes].filter(node => node.nodeType === Node.TEXT_NODE)
          .map(node => node.textContent.trim()).filter(Boolean),
      };
    })()`);
    assert.deepEqual(theme.options, [
      ['light', '浅色'],
      ['dark', '深色'],
      ['system', '跟随系统'],
    ]);
    assert.deepEqual(theme.extra, []);
  };
  await test('0.8.1 旧设置默认不联网，保存后下次启动检查并提示新版', async () => {
    await metrics();
    await nav();
    assert.equal(await evaluate("!!document.querySelector('.settings-update-badge')"), false);
    await openCategory('外观');
    await assertThemeChoices();
    await change('[data-category=appearance] select[aria-label=主题]', 'system');
    await evaluate(
      "[...document.querySelectorAll('.settings-nav button')].find(button => button.textContent.trim() === '关于').click()",
    );
    await aboutStatus('当前版本 0.8.1');
    assert.equal(await evaluate('window.releaseChecks.length'), 0);
    await click('[data-category=about] input[type=checkbox]');
    await click('.settings-buttons button[type=submit]');
    await until(
      () =>
        evaluate(
          'window.settings.auto_check_updates === true && window.settings.theme === "system"',
        ),
      '设置未保存',
    );
    assert.equal(await evaluate('window.releaseChecks.length'), 0);
    await nav('?auto-update=1');
    await until(
      () =>
        evaluate(
          "!!document.querySelector('.sidebar-bottom .settings-toggle .settings-update-badge')",
        ),
      '启动时未提示新版',
    );
    assert.equal(
      await evaluate("document.querySelector('.settings-toggle').getAttribute('aria-description')"),
      '有新版本 0.8.10',
    );
    assert.equal(await evaluate("document.querySelectorAll('.main-statusbar button').length"), 0);
    for (const [skin, theme, shape, accent, ink] of [
      ['clipraft', 'light', 'ui-update-line', 'rgb(39, 125, 119)', 'rgb(39, 125, 119)'],
      ['clipraft', 'dark', 'ui-update-line', 'rgb(156, 189, 209)', 'rgb(156, 189, 209)'],
      ['fluent', 'light', 'ui-update-fluent', 'rgb(23, 107, 100)', 'rgb(255, 255, 255)'],
      ['fluent', 'dark', 'ui-update-fluent', 'rgb(131, 213, 195)', 'rgb(18, 42, 36)'],
    ]) {
      await appearance({ skin, theme });
      const badge = await evaluate(`(() => {
        const icon = document.querySelector('svg.settings-update-badge');
        const button = icon.closest('.settings-toggle');
        const shapes = [...icon.querySelectorAll('g')].filter(group => getComputedStyle(group).display !== 'none');
        const style = getComputedStyle(icon);
        const buttonStyle = getComputedStyle(button);
        const right = button.getBoundingClientRect().right - parseFloat(buttonStyle.paddingRight)
          - parseFloat(buttonStyle.borderRightWidth);
        return {
          size: [style.width, style.height], color: style.color, shapes: shapes.map(group => group.getAttribute('class')),
          stroke: getComputedStyle(shapes[0].querySelector('path')).stroke,
          alignedRight: Math.abs(icon.getBoundingClientRect().right - right) < 1,
        };
      })()`);
      assert.deepEqual(badge, {
        size: ['14px', '14px'],
        color: accent,
        shapes: [shape],
        stroke: ink,
        alignedRight: true,
      });
      if (theme === 'light') await capture(`v081-settings-update-right-${skin}`);
    }
    assert.deepEqual(await evaluate('window.releaseChecks'), [
      { credentials: 'omit', redirect: 'error', referrerPolicy: 'no-referrer' },
    ]);
    await openCategory('关于');
    await aboutStatus('发现新版本 0.8.10');
    assert.equal(
      await evaluate("document.querySelector('.release-notes').textContent"),
      '- 合成发布说明 <b>纯文本</b>\n### 细节\n保留子节',
    );
    assert.deepEqual(await evaluate('window.releaseNotesTags'), []);
    assert.equal(await evaluate("!!document.querySelector('.release-notes b')"), false);
    assert.equal(
      await evaluate("window.calls.filter(call=>call.cmd==='open_release_page').length"),
      0,
    );
    await click('.about-open-release');
    assert.equal(
      await evaluate("window.calls.filter(call=>call.cmd==='open_release_page').length"),
      1,
    );
  });
  await test('0.8.1 手动检查处理正式版、预发布和断网，允许重试', async () => {
    await metrics();
    await nav();
    await openCategory('关于');
    await evaluate('window.releasePageOffline = true');
    for (const [reply, message] of [
      [{ status: 200, body: { tag_name: 'v0.8.1' } }, '暂无可用更新'],
      [{ status: 200, body: { tag_name: 'v0.7.9' } }, '暂无可用更新'],
      [
        { status: 200, body: { tag_name: 'v9.0.0-beta.1', prerelease: true } },
        '暂无可用的正式版本',
      ],
      [{ status: 200, body: { tag_name: 'invalid' } }, '无法识别 GitHub 返回的版本号'],
      [{ status: 404 }, '暂无可用的正式版本'],
      [{ offline: true }, '请检查网络后重试'],
    ]) {
      await evaluate(`window.releaseReply=${JSON.stringify(reply)}`);
      await click('.about-check');
      await aboutStatus(message);
    }
    assert.equal(await evaluate('window.releaseChecks.length'), 6);
    assert.equal(await evaluate('window.releasePageChecks'), 1);
    assert.equal(
      await evaluate("document.querySelector('.about-open-release').textContent.trim()"),
      '打开 GitHub 发布页',
    );
  });
  await test('0.8.1 API 返回 403 或 429 时从发布页判断版本', async () => {
    await metrics();
    await nav();
    await appearance({ skin: 'fluent' });
    await openCategory('关于');
    await evaluate("window.releaseReply={status:403}; window.releasePageTag='v0.8.0'");
    await click('.about-check');
    await aboutStatus('暂无可用更新');
    await assertAboutCard();
    await capture('v081-about-fluent-light');
    await evaluate(
      "window.releaseReply={status:429}; window.releasePageTag='v0.8.10'; window.releasePageNotes='- 发布页回退已取到更新内容'",
    );
    await click('.about-check');
    await aboutStatus('发现新版本 0.8.10');
    await aboutStatus('发布页回退已取到更新内容');
    assert.equal(await evaluate('window.releasePageChecks'), 2);
    assert.deepEqual(await evaluate('window.releaseNotesTags'), ['v0.8.10']);
    assert.equal(
      await evaluate("document.querySelector('.about-open-release').textContent.trim()"),
      '前往 GitHub 下载',
    );
    await evaluate(
      "window.releaseReply={status:200,body:{tag_name:'v0.8.10',body:'没有更新标题的长正文'}}; window.releasePageNotes=''",
    );
    await click('.about-check');
    await aboutStatus('可前往 GitHub 查看完整发布说明');
    assert.equal(await evaluate("!!document.querySelector('.release-notes')"), false);
    assert.deepEqual(await evaluate('window.releaseNotesTags'), ['v0.8.10', 'v0.8.10']);
  });
  await test('0.8.1 慢请求期间禁止重复检查，超时后可再次发起', async () => {
    await metrics();
    await nav();
    await openCategory('关于');
    await evaluate(
      'window.releaseReply={pending:true};const originalTimeout = window.setTimeout;window.setTimeout=(callback, delay, ...args)=>originalTimeout(callback,delay===10000?900:delay,...args)',
    );
    await click('.about-check');
    await aboutStatus('正在连接 GitHub');
    assert(await evaluate("document.querySelector('.about-check').disabled"));
    await click('.about-check');
    assert.equal(await evaluate('window.releaseChecks.length'), 1);
    await aboutStatus('检查更新超时');
    await evaluate("window.releaseReply={status:200,body:{tag_name:'v0.8.1'}}");
    await click('.about-check');
    await aboutStatus('暂无可用更新');
    assert.equal(await evaluate('window.releaseChecks.length'), 2);
  });
  await test('0.8.1 跨版本更新内容按正式版本降序分组，历史失败时仍可下载', async () => {
    await metrics();
    await nav('?cross-updates=1');
    await openCategory('关于');
    await aboutStatus('当前版本 0.6.9');
    await evaluate(`window.releaseReply={status:429};
      window.releasePageTag='v0.8.0';
      window.releaseHistory={complete:true,releases:[
        {tag:'v0.6.9',notes:'当前版本不显示'},
        {tag:'v0.7.0',notes:'Fluent 主题'},
        {tag:'v0.7.5-beta.1',notes:'预发布不显示'},
        {tag:'v0.8.0',notes:'自定义背景'},
        {tag:'0.7.0',notes:'重复项不显示'},
        {tag:'v0.9.0',notes:'超出最新正式版不显示'},
      ]};`);
    await click('.about-check');
    await aboutStatus('自定义背景');
    const groups = () =>
      evaluate(`[...document.querySelectorAll('.about-release-notes')].map(node => [
        node.querySelector('h4').textContent.trim(), node.querySelector('.release-notes')?.textContent.trim()
      ])`);
    assert.deepEqual(await groups(), [
      ['0.8.0', '自定义背景'],
      ['0.7.0', 'Fluent 主题'],
    ]);
    assert.deepEqual(await evaluate('window.releaseNotesTags'), []);
    await evaluate("window.releaseHistoryOffline=true; window.releasePageNotes='自定义背景'");
    await click('.about-check');
    await aboutStatus('部分发布记录暂未读取');
    assert.deepEqual(await groups(), [['0.8.0', '自定义背景']]);
    assert.equal(
      await evaluate("document.querySelector('.about-open-release').textContent.trim()"),
      '前往 GitHub 下载',
    );
  });
  await test('0.8.1 紧凑 Fluent 关于页可操作且发布说明正常换行', async () => {
    await metrics(390);
    await nav('?compact-about=1&auto-update=1');
    await until(
      () =>
        evaluate(
          "!!document.querySelector('.main-statusbar .settings-toggle .settings-update-badge')",
        ),
      '紧凑设置入口未显示新版角标',
    );
    assert.equal(await evaluate("document.querySelectorAll('.main-statusbar button').length"), 2);
    await click('.main-statusbar .settings-toggle');
    await until(
      () => evaluate("!!document.querySelector('.settings-category select')"),
      '紧凑设置未显示',
    );
    await change('.settings-category select', 'appearance');
    await assertThemeChoices();
    await change('.settings-category select', 'about');
    await aboutStatus('当前版本 0.8.1');
    await evaluate(
      "window.releaseReply.body.body = '## 更新内容' + String.fromCharCode(10) + 'long-content-'.repeat(35)",
    );
    await click('.about-check');
    await aboutStatus('发现新版本 0.8.10');
    await assertAboutCard();
    assert(
      await evaluate(
        "document.querySelector('[data-category=about]').scrollWidth <= document.querySelector('[data-category=about]').clientWidth",
      ),
    );
    await capture('v081-about-compact-dark');
  });
  await writeFile(
    path.join(out, 'results.json'),
    JSON.stringify(
      {
        passed: checks.length,
        failed: failures.length,
        checks,
        failures,
        scope: 'Edge 无头实际渲染，合成 IPC；原生窗口/焦点/粘贴未测',
      },
      null,
      2,
    ),
  );
  console.log(
    JSON.stringify({ passed: checks.length, failed: failures.length, failures }, null, 2),
  );
  await appearanceRecorder.finish();
  await send('Browser.close').catch(() => {});
  if (failures.length) process.exitCode = 1;
} finally {
  ws?.close();
  browser.kill();
  await new Promise((resolve) => server.close(resolve));
}

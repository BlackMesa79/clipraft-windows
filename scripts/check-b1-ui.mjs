import { createAppearanceRecorder } from './appearance-baseline.mjs';
// 离屏渲染 + 合成 IPC 数据；不操作用户桌面，不代表原生粘贴验收。
import { createServer } from 'node:http';
import { spawn } from 'node:child_process';
import { readFile, writeFile, mkdir } from 'node:fs/promises';
import path from 'node:path';
import assert from 'node:assert/strict';
import { settingsMock } from './settings-ui-mock.mjs';
const root = process.cwd();
const out = path.join(root, '.tools/b1-ui');
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
  const appearanceRecorder = createAppearanceRecorder(evaluate, out);
  const mock = `(() => {
    const callbacks=new Map(),events=new Map();let id=0;
    window.calls=[];window.ready=false;window.failResize=false;
    window.rows=[
      {id:'safe',kind:'text',summary:'合成普通文本',formats:['CF_UNICODETEXT'],files:[]},
      {id:'secret',kind:'text',summary:'sk-S••••••••ef',sensitive:true,formats:['CF_UNICODETEXT'],files:[],thumbnail:null},
      {id:'file',kind:'files',summary:'文件',formats:['CF_HDROP'],files:['C:/synthetic/report.txt'],file_status:'missing'}
    ];
    window.emitEvent=(event,payload)=>callbacks.get(events.get(event))?.({event,id:1,payload});
    window.__TAURI_EVENT_PLUGIN_INTERNALS__={unregisterListener(){}};
    window.__TAURI_INTERNALS__={
      transformCallback(fn){callbacks.set(++id,fn);return id;},convertFileSrc:hash=>'/mock/'+hash,
      async invoke(cmd,args={}) {
        window.calls.push({cmd,args});
        if(cmd==='plugin:event|listen'){events.set(args.event,args.handler);return id;}
        if(cmd==='resize_quick'){if(window.failResize){window.failResize=false;window.resizeFailures=(window.resizeFailures??0)+1;throw Error('合成 resize 失败');}window.resizeSuccess=(window.resizeSuccess??0)+1;return;}
        if(cmd==='panel_state'){if(location.search.includes('fail=panel'))throw Error('合成 panel_state 失败');return {items:window.rows,selected:1,visible:true,error:null,serial:0,session:1,position:1,total:window.rows.length,loading:false};}
        if(cmd==='frontend_ready'){window.ready=true;return;}
        if(cmd==='list_items')return window.rows;
        if(cmd==='list_groups')return [];
        if(cmd==='last_error')return null;
        if(cmd==='item_detail')return {text:'合成详情',files:[],truncated:false};
        return 1;
      }
    };
  })();`;

  const archiveMock = `(() => {
    window.archiveReport=null; window.archiveStarts=[]; window.activations=[];
    const invoke=window.__TAURI_INTERNALS__.invoke;
    window.__TAURI_INTERNALS__.invoke=async(cmd,args={})=>{
      if(cmd==='archive_task_status')return structuredClone(window.archiveReport);
      if(cmd==='scan_archive_staging')return {candidates:window.stagingCandidates??[],truncated:false};
      if(cmd==='preview_staging_cleanup')return {ticket:'review-1',root:args.root,files:2,bytes:2048};
      if(cmd==='cancel_staging_cleanup'){window.stagingCancelled=(window.stagingCancelled??0)+1;return;}
      if(cmd==='confirm_staging_cleanup'){window.stagingConfirmed=(window.stagingConfirmed??0)+1;window.stagingCandidates=[];return;}
      if(cmd==='retry_migration_cleanup'){window.cleanupRetried=true;window.directoryStatus.cleanup.complete=true;window.directoryStatus.cleanup.error=null;return;}

      if(cmd==='choose_archive_path')return args.selection==='archive'?'D:/synthetic/history.clipraft':'D:/synthetic';
      if(cmd==='start_archive_export'||cmd==='start_archive_restore'){
        window.archiveStarts.push({cmd,passwordLength:args.input.password.length,destination:args.input.destination});
        window.archiveReport={id:1,kind:cmd==='start_archive_export'?'export':'restore',running:true,progress:{phase:'snapshotting',completed_bytes:0,total_bytes:0},outcome:null};
        return 1;
      }
      if(cmd==='activate_restored_archive'){
        window.activations.push(args.id); if(window.failActivation)throw Error('模拟切换失败，原库未变');return;
      }
      if(cmd==='cancel_archive_task'){
        window.archiveReport={...window.archiveReport,running:false,progress:{...window.archiveReport.progress,phase:'cancelled'},outcome:{state:'cancelled',message:'已取消并清理'}};return true;
      }
      return invoke(cmd,args);
    };
  })();`;
  await send('Page.enable');
  await send('Runtime.enable');
  await send('Emulation.setFocusEmulationEnabled', { enabled: true });
  await send('Page.addScriptToEvaluateOnNewDocument', {
    source: mock + settingsMock + archiveMock,
  });
  await send('Emulation.setDeviceMetricsOverride', {
    width: 1000,
    height: 900,
    deviceScaleFactor: 1,
    mobile: false,
  });
  await send('Page.navigate', { url: origin });
  await until(() => evaluate('window.ready===true'), '未ready');
  await pause(400);
  const focusChecks = [];
  for (const theme of ['light', 'dark']) {
    for (const layout of ['list', 'cards', 'compact']) {
      await evaluate(
        `window.emitEvent('settings-changed',{...window.settings,theme:${JSON.stringify(theme)},layout:${JSON.stringify(layout === 'cards' ? 'cards' : 'list')},compact:${layout === 'compact'}})`,
      );
      await pause(80);
      await send('Input.dispatchKeyEvent', {
        type: 'keyDown',
        key: 'Tab',
        code: 'Tab',
        windowsVirtualKeyCode: 9,
      });
      await send('Input.dispatchKeyEvent', {
        type: 'keyUp',
        key: 'Tab',
        code: 'Tab',
        windowsVirtualKeyCode: 9,
      });
      const selectors = [
        '.item:not(.selected)',
        '.window-controls button',
        '.kind-filters button',
        '.compact-toggle',
        '.settings-toggle',
      ];
      if (layout !== 'compact') selectors.push('.preview-refresh');
      for (const selector of selectors) {
        const result = await evaluate(
          `(()=>{const e=document.querySelector(${JSON.stringify(selector)});if(!e||!e.getBoundingClientRect().width||e.disabled)return null;e.focus();const s=getComputedStyle(e);return {offset:s.outlineOffset,style:s.outlineStyle,width:s.outlineWidth,visible:e.matches(':focus-visible')};})()`,
        );
        if (!result) continue;
        assert.equal(result.visible, true, theme + '/' + layout + '/' + selector);
        assert.equal(result.offset, '-2px', theme + '/' + layout + '/' + selector);
        assert.equal(result.style, 'solid');
        assert.equal(result.width, '2px');
        focusChecks.push(theme + '/' + layout + '/' + selector);
      }
    }
  }
  await evaluate(
    "window.emitEvent('settings-changed',{...window.settings,theme:'light',layout:'list',compact:false})",
  );
  await pause(80);
  for (const [name, selector] of [
    ['record', '.item:not(.selected)'],
    ['refresh', '.preview-refresh'],
    ['titlebar', '.window-controls button'],
  ]) {
    await evaluate(`document.querySelector(${JSON.stringify(selector)}).focus()`);
    const shot = await send('Page.captureScreenshot', { format: 'jpeg', quality: 65 });
    const bytes = Buffer.from(shot.data, 'base64');
    assert(bytes.length < 300 * 1024);
    await writeFile(path.join(out, 'focus-' + name + '.jpg'), bytes);
  }

  await evaluate(
    "Array.from(document.querySelectorAll('.group-sidebar button')).find(button=>button.textContent.includes('新建分组')).click()",
  );
  await until(
    () => evaluate("!!document.querySelector('.group-editor input')"),
    '新建分组输入未显示',
  );
  await until(
    () => evaluate("document.activeElement===document.querySelector('.group-editor input')"),
    '分组输入未聚焦',
  );
  assert.equal(
    await evaluate(
      "(()=>{const e=document.querySelector('.group-editor input');e.focus();return getComputedStyle(e).outlineOffset})()",
    ),
    '-2px',
  );
  await appearanceRecorder.capture('groupShot');
  const groupShot = await send('Page.captureScreenshot', { format: 'jpeg', quality: 65 });
  const groupPreview = Buffer.from(groupShot.data, 'base64');
  assert(groupPreview.length < 300 * 1024);
  await writeFile(path.join(out, 'group-focus.jpg'), groupPreview);
  await evaluate(
    "Array.from(document.querySelectorAll('.group-editor button')).find(button=>button.textContent==='取消').click()",
  );
  await send('Input.dispatchKeyEvent', {
    type: 'keyDown',
    key: 'Tab',
    code: 'Tab',
    windowsVirtualKeyCode: 9,
  });
  await send('Input.dispatchKeyEvent', {
    type: 'keyUp',
    key: 'Tab',
    code: 'Tab',
    windowsVirtualKeyCode: 9,
  });
  await evaluate("document.querySelector('[data-group-view=all]').focus()");
  assert.equal(
    await evaluate(
      "(()=>{const e=document.querySelector('[data-group-view=all]');e.focus();return getComputedStyle(e).outlineOffset})()",
    ),
    '-2px',
  );
  await evaluate("window.emitEvent('open-settings')");
  await until(() => evaluate("!!document.querySelector('.archive-settings')"), '缺少归档设置');
  await evaluate(
    "Array.from(document.querySelectorAll('button')).find(b=>b.textContent.trim()==='数据存储').click()",
  );
  await until(
    () => evaluate("!document.querySelector('[data-start-archive]').disabled"),
    '归档状态未就绪',
  );
  const checks = [
    '分组输入框和侧栏Tab焦点环向内绘制',
    `浅深主题与三布局焦点环检查${focusChecks.length}项`,
  ];
  const input = async (selector, value) =>
    evaluate(
      `(()=>{const input=document.querySelector(${JSON.stringify(selector)});input.value=${JSON.stringify(value)};input.dispatchEvent(new Event('input',{bubbles:true}));})()`,
    );
  const click = async (selector) =>
    evaluate(`document.querySelector(${JSON.stringify(selector)}).click()`);
  await input('[data-archive-destination]', 'D:/synthetic/history.clipraft');
  assert.equal(await evaluate("document.querySelectorAll('input[type=password]').length"), 0);
  checks.push('设置页不包含密码输入');
  await click('[data-start-archive]');
  await until(() => evaluate("!!document.querySelector('dialog[open]')"), '密码对话框未打开');
  assert.equal(
    await evaluate("document.activeElement.hasAttribute('data-archive-password')"),
    true,
  );
  for (let index = 0; index < 5; index++) {
    await send('Input.dispatchKeyEvent', {
      type: 'keyDown',
      key: 'Tab',
      code: 'Tab',
      windowsVirtualKeyCode: 9,
    });
    await send('Input.dispatchKeyEvent', {
      type: 'keyUp',
      key: 'Tab',
      code: 'Tab',
      windowsVirtualKeyCode: 9,
    });
    assert.equal(await evaluate("!!document.activeElement.closest('dialog')"), true);
  }
  checks.push('密码对话框Tab焦点不会进入背景');
  await input('[data-archive-password]', 'discard-on-close');
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
  await until(() => evaluate("!document.querySelector('dialog')"), 'Escape未关闭密码对话框');
  assert.equal(await evaluate("!!document.querySelector('.archive-settings')"), true);
  assert.equal(await evaluate('window.archiveStarts.length'), 0);
  await click('[data-start-archive]');
  await until(() => evaluate("!!document.querySelector('dialog[open]')"), '密码对话框未重开');
  assert.equal(await evaluate("document.querySelector('[data-archive-password]').value"), '');
  checks.push('密码弹窗焦点、Escape隔离和取消清空');
  await appearanceRecorder.capture('passwordShot');
  const passwordShot = await send('Page.captureScreenshot', { format: 'jpeg', quality: 65 });
  const passwordPreview = Buffer.from(passwordShot.data, 'base64');
  assert(passwordPreview.length < 300 * 1024);
  await writeFile(path.join(out, 'password-dialog.jpg'), passwordPreview);
  await input('[data-archive-password]', 'synthetic-password');
  await input('[data-archive-confirmation]', 'different');
  await click('[data-confirm-archive-dialog]');
  await until(
    () => evaluate("document.querySelector('dialog').textContent.includes('不一致')"),
    '未拒绝不一致密码',
  );
  assert.equal(await evaluate('window.archiveStarts.length'), 0);
  checks.push('不一致密码不启动');
  await input('[data-archive-confirmation]', 'synthetic-password');
  await click('[data-confirm-archive-dialog]');
  await until(() => evaluate('window.archiveStarts.length===1'), '未启动导出');
  assert.equal(await evaluate("document.querySelectorAll('input[type=password]').length"), 0);
  checks.push('提交后清空密码');
  await until(
    () =>
      evaluate(
        "document.querySelector('.archive-settings').textContent.includes('期间复制不会记录')",
      ),
    '缺少维护提示',
  );
  assert.equal(await evaluate("document.querySelector('[data-start-archive]').disabled"), true);
  checks.push('运行中禁止重复启动');
  assert.equal(
    await evaluate(
      "Array.from(document.querySelectorAll('.data-directory-settings button')).every(b=>b.disabled)",
    ),
    true,
  );
  checks.push('运行中禁用迁移');
  await evaluate(
    "Array.from(document.querySelectorAll('button')).find(b=>b.textContent.trim()==='取消').click()",
  );
  await until(() => evaluate("!document.querySelector('.archive-settings')"), '设置未关闭');
  assert.equal(await evaluate('window.archiveReport.running'), true);
  await evaluate("window.emitEvent('open-settings')");
  await until(() => evaluate("!!document.querySelector('.archive-settings')"), '设置未重新打开');
  await evaluate(
    "Array.from(document.querySelectorAll('button')).find(b=>b.textContent.trim()==='数据存储').click()",
  );
  await until(
    () => evaluate("!!document.querySelector('[data-cancel-archive]')"),
    '未恢复运行中任务',
  );
  checks.push('关闭再打开设置可继续管理后台任务');

  await evaluate(
    "window.archiveReport.progress={phase:'publishing',completed_bytes:100,total_bytes:100}",
  );
  await until(
    () => evaluate("document.querySelector('[data-cancel-archive]').disabled"),
    '发布期允许取消',
  );
  checks.push('发布时禁用取消');
  await evaluate("window.archiveReport.progress.phase='writing'");
  await until(
    () => evaluate("!document.querySelector('[data-cancel-archive]').disabled"),
    '写入期不能取消',
  );
  await click('[data-cancel-archive]');
  await until(() => evaluate("!document.querySelector('[data-cancel-archive]')"), '取消未结束');
  checks.push('取消后恢复操作');
  await evaluate(
    "Array.from(document.querySelectorAll('.archive-settings button')).find(b=>b.textContent==='恢复归档').click()",
  );
  await input('[data-archive-source]', 'D:/synthetic/history.clipraft');
  assert.equal(await evaluate("document.querySelector('[data-restore-target]').value"), 'current');
  assert.equal(await evaluate("!!document.querySelector('[data-archive-destination]')"), false);
  checks.push('恢复默认原目录替换，不要求选择新目录');
  await click('[data-start-archive]');
  await until(() => evaluate("!!document.querySelector('dialog[open]')"), '恢复密码弹窗未显示');
  assert.equal(await evaluate("!!document.querySelector('[data-archive-confirmation]')"), false);
  await input('[data-archive-password]', 'synthetic-password');
  await click('[data-confirm-archive-dialog]');
  await until(() => evaluate('window.archiveStarts.length===2'), '恢复未启动');
  assert.equal(await evaluate('window.archiveStarts[1].destination===undefined'), true);
  await evaluate(
    "window.archiveReport={id:2,kind:'restore',running:false,progress:{phase:'failed',completed_bytes:0,total_bytes:0},outcome:{state:'failed',message:'密码错误或归档损坏'}}",
  );
  await until(
    () =>
      evaluate(
        "document.querySelector('.archive-settings').textContent.includes('密码错误或归档损坏')",
      ),
    '恢复失败未显示',
  );
  assert.equal(await evaluate("!!document.querySelector('dialog')"), false);
  assert.equal(await evaluate('window.activations.length'), 0);
  checks.push('错误密码或坏包不会触发替换');
  await click('[data-start-archive]');
  await until(() => evaluate("!!document.querySelector('dialog[open]')"), '未重开恢复密码弹窗');
  await input('[data-archive-password]', 'synthetic-password');
  await click('[data-confirm-archive-dialog]');
  await until(() => evaluate('window.archiveStarts.length===3'), '恢复重试未开始');

  await evaluate(
    "window.archiveReport={id:2,kind:'restore',running:false,progress:{phase:'complete',completed_bytes:100,total_bytes:100},outcome:{state:'succeeded',restored_root:'D:/synthetic/staging/database',replace_current:true,cleanup_warning:'测试清理提示'}}",
  );
  await until(
    () =>
      evaluate("document.querySelector('.archive-settings').textContent.includes('归档已验证')"),
    '没有恢复结果',
  );
  assert.equal(
    await evaluate(
      "document.querySelector('.archive-settings').textContent.includes('当前数据目录保持不变')",
    ),
    true,
  );
  checks.push('恢复完成明确未切换');
  assert.equal(
    await evaluate(
      "document.querySelector('.archive-settings').textContent.includes('测试清理提示')",
    ),
    true,
  );
  checks.push('清理警告可见');
  await until(() => evaluate("!!document.querySelector('[data-replace-ack]')"), '未自动提示替换');
  assert.equal(await evaluate('window.activations.length'), 0);
  assert.equal(
    await evaluate("document.querySelector('[data-confirm-archive-dialog]').disabled"),
    true,
  );
  await appearanceRecorder.capture('replaceShot');
  const replaceShot = await send('Page.captureScreenshot', { format: 'jpeg', quality: 65 });
  const replacePreview = Buffer.from(replaceShot.data, 'base64');
  assert(replacePreview.length < 300 * 1024);
  await writeFile(path.join(out, 'replace-dialog.jpg'), replacePreview);
  await click('[data-dismiss-archive-dialog]');
  await until(() => evaluate("!document.querySelector('dialog')"), '暂不替换未关闭');
  assert.equal(await evaluate('window.activations.length'), 0);
  checks.push('验证成功后自动提示替换，未确认不切换');
  await evaluate(
    "Array.from(document.querySelectorAll('button')).find(b=>b.textContent.trim()==='取消').click()",
  );
  await until(
    () => evaluate("!document.querySelector('.archive-settings')"),
    '暂不替换后设置未关闭',
  );
  await evaluate("window.emitEvent('open-settings')");
  await until(
    () => evaluate("!!document.querySelector('[data-replace-current]')"),
    '重新打开未恢复成功任务',
  );
  await evaluate(
    "Array.from(document.querySelectorAll('button')).find(b=>b.textContent.trim()==='数据存储').click()",
  );
  assert.equal(await evaluate("!!document.querySelector('dialog')"), false);
  checks.push('暂不替换后关闭重开设置仍指向同一恢复任务');

  await click('[data-replace-current]');
  await until(() => evaluate("!!document.querySelector('[data-replace-ack]')"), '不能稍后替换');
  await click('[data-replace-ack]');
  await evaluate('window.failActivation=true');
  await click('[data-confirm-archive-dialog]');
  await until(
    () => evaluate("document.querySelector('dialog').textContent.includes('原库未变')"),
    '切换失败未显示',
  );
  assert.equal(await evaluate("!!document.querySelector('dialog[open]')"), true);
  checks.push('切换错误保留确认窗可重试');
  await evaluate('window.failActivation=false');
  await click('[data-confirm-archive-dialog]');
  await until(
    () => evaluate("document.querySelector('.archive-settings').textContent.includes('正在重启')"),
    '切换成功未提示重启',
  );
  assert.deepEqual(await evaluate('window.activations'), [2, 2]);
  checks.push('确认替换只提交成功任务编号并等待重启');
  await evaluate("document.querySelector('.archive-settings').scrollIntoView({block:'center'})");
  const shot = await send('Page.captureScreenshot', { format: 'jpeg', quality: 65 });
  const bytes = Buffer.from(shot.data, 'base64');
  assert(bytes.length < 300 * 1024);
  await writeFile(path.join(out, 'restore-result.jpg'), bytes);

  await send('Page.navigate', { url: origin });
  await until(() => evaluate('window.ready===true'), '另存回归页面未ready');
  await evaluate("window.emitEvent('open-settings')");
  await until(() => evaluate("!!document.querySelector('.archive-settings')"), '另存设置未显示');
  await evaluate(
    "Array.from(document.querySelectorAll('button')).find(b=>b.textContent.trim()==='数据存储').click()",
  );
  await until(
    () => evaluate("!document.querySelector('[data-start-archive]').disabled"),
    '另存状态未就绪',
  );
  await evaluate(
    "Array.from(document.querySelectorAll('.archive-settings button')).find(b=>b.textContent==='恢复归档').click()",
  );
  await evaluate(
    "const select=document.querySelector('[data-restore-target]');select.value='other';select.dispatchEvent(new Event('change',{bubbles:true}))",
  );
  await input('[data-archive-source]', 'D:/synthetic/history.clipraft');
  await input('[data-archive-destination]', 'D:/synthetic/other');
  await click('[data-start-archive]');
  await until(() => evaluate("!!document.querySelector('dialog[open]')"), '另存密码框未显示');
  await input('[data-archive-password]', 'synthetic-password');
  await click('[data-confirm-archive-dialog]');
  await until(() => evaluate('window.archiveStarts.length===1'), '另存未启动');
  assert.equal(await evaluate('window.archiveStarts[0].destination'), 'D:/synthetic/other');
  await evaluate(
    "window.archiveReport={id:1,kind:'restore',running:false,progress:{phase:'complete',completed_bytes:1,total_bytes:1},outcome:{state:'succeeded',restored_root:'D:/synthetic/other',replace_current:false,cleanup_warning:null}}",
  );
  await until(
    () =>
      evaluate(
        "document.querySelector('.archive-settings').textContent.includes('新库已验证并恢复')",
      ),
    '另存结果未显示',
  );
  assert.equal(await evaluate("!!document.querySelector('dialog')"), false);
  assert.equal(await evaluate("!!document.querySelector('[data-replace-current]')"), false);
  checks.push('可选恢复到其他目录，不提示替换或重启');
  await send('Page.navigate', { url: origin });
  await until(() => evaluate('window.ready===true'), '暂存回归页面未ready');
  await evaluate(
    "window.directoryStatus.cleanup={backup:'D:/synthetic/.clipraft-backup-safe',source:'D:/synthetic/old',initialized:true,complete:false,remaining:2,error:'旧文件被占用，等待重试'};window.stagingCandidates=[{root:'D:/synthetic/.clipraft-export-active',status:{state:'active'}},{root:'D:/synthetic/.clipraft-restore-incomplete',status:{state:'retained'}},{root:'D:/synthetic/.clipraft-export-legacy',status:{state:'unrecognized',reason:'缺少所有权标记'}}];window.emitEvent('open-settings')",
  );
  await until(() => evaluate("!!document.querySelector('.staging-settings')"), '暂存入口未显示');
  await evaluate(
    "Array.from(document.querySelectorAll('button')).find(b=>b.textContent.trim()==='数据存储').click()",
  );
  await until(
    () => evaluate("!document.querySelector('[data-scan-staging]').disabled"),
    '暂存入口不可用',
  );
  const extendedPaths = Array.from({ length: 40 }, (_, index) => ({
    root: String.raw`\\?\D:\synthetic\.clipraft-restore-` + index,
    status: { state: index === 0 ? 'active' : 'retained' },
  }));
  extendedPaths.push({
    root: String.raw`\\?\UNC\server\share\legacy`,
    status: { state: 'unrecognized', reason: '缺少所有权标记' },
  });
  await evaluate('window.stagingCandidates=' + JSON.stringify(extendedPaths));
  await click('[data-scan-staging]');
  await until(
    () => evaluate("document.querySelectorAll('.staging-settings li').length===41"),
    '暂存扫描结果不完整',
  );
  assert.equal(await evaluate("document.querySelector('[data-review-staging]').disabled"), true);
  assert.equal(await evaluate("document.querySelectorAll('[data-review-staging]').length"), 40);
  checks.push('活动暂存不可清理，未知标记不提供清理入口');
  await evaluate(
    "Array.from(document.querySelectorAll('[data-review-staging]')).find(button=>!button.disabled).click()",
  );
  await until(
    () => evaluate("!!document.querySelector('[data-confirm-staging]')"),
    '暂存未提供确认清单',
  );
  assert.equal(await evaluate('window.stagingConfirmed??0'), 0);
  assert.equal(await evaluate("!!document.querySelector('.staging-cleanup-dialog[open]')"), true);
  assert.equal(
    await evaluate(
      "document.querySelector('.staging-cleanup-dialog').textContent.includes(String.fromCharCode(92,92,63,92))",
    ),
    false,
  );
  assert.equal(
    await evaluate(
      "Array.from(document.querySelectorAll('.staging-settings .staging-path')).some(node=>node.textContent.startsWith(String.fromCharCode(92,92)+'server'))",
    ),
    true,
  );
  const visibleConfirm = await evaluate(
    "(()=>{const r=document.querySelector('[data-confirm-staging]').getBoundingClientRect();return r.top>=0&&r.bottom<=innerHeight})()",
  );
  assert.equal(visibleConfirm, true);
  for (let index = 0; index < 4; index++) {
    await send('Input.dispatchKeyEvent', {
      type: 'keyDown',
      key: 'Tab',
      code: 'Tab',
      windowsVirtualKeyCode: 9,
    });
    await send('Input.dispatchKeyEvent', {
      type: 'keyUp',
      key: 'Tab',
      code: 'Tab',
      windowsVirtualKeyCode: 9,
    });
    assert.equal(
      await evaluate("!!document.activeElement.closest('.staging-cleanup-dialog')"),
      true,
    );
  }
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
  await until(
    () => evaluate("!document.querySelector('.staging-cleanup-dialog')"),
    'Esc未关闭清理确认',
  );
  assert.equal(await evaluate('window.stagingConfirmed??0'), 0);
  assert.equal(await evaluate('window.stagingCancelled'), 1);
  await evaluate(
    "Array.from(document.querySelectorAll('[data-review-staging]')).find(button=>!button.disabled).click()",
  );
  await until(
    () => evaluate("!!document.querySelector('.staging-cleanup-dialog[open]')"),
    '清理确认未重新显示',
  );
  checks.push('长暂存列表确认弹窗可见、焦点受限、Esc取消不删除');
  checks.push('盘符及UNC扩展前缀仅在显示中移除');

  await evaluate("document.querySelector('.staging-settings').scrollIntoView({block:'center'})");
  await appearanceRecorder.capture('cleanupShot');
  const cleanupShot = await send('Page.captureScreenshot', { format: 'jpeg', quality: 65 });
  const cleanupPreview = Buffer.from(cleanupShot.data, 'base64');
  assert(cleanupPreview.length < 300 * 1024);
  await writeFile(path.join(out, 'staging-review.jpg'), cleanupPreview);
  await click('[data-confirm-staging]');
  await until(() => evaluate('window.stagingConfirmed===1'), '确认后未清理');
  await until(
    () => evaluate("document.querySelector('.staging-settings').textContent.includes('没有发现')"),
    '清理后未刷新',
  );
  checks.push('非空暂存先预检再确认，确认后刷新扫描结果');
  assert.equal(
    await evaluate(
      "document.querySelector('[data-migration-cleanup]').textContent.includes('.clipraft-backup-safe')",
    ),
    true,
  );
  assert.equal(
    await evaluate(
      "document.querySelector('[data-migration-cleanup]').textContent.includes('当前继续使用新库')",
    ),
    true,
  );
  await click('[data-retry-migration-cleanup]');
  await until(
    () =>
      evaluate(
        "document.querySelector('[data-migration-cleanup]').textContent.includes('源清单文件已清理')",
      ),
    '源清理重试未更新',
  );
  assert.equal(await evaluate('window.cleanupRetried'), true);
  checks.push('迁移备份位置与待清理状态可见，重试完成不回滚新库');
  await evaluate(
    "document.querySelector('[data-migration-cleanup]').scrollIntoView({block:'center'})",
  );
  await appearanceRecorder.capture('migrationShot');
  const migrationShot = await send('Page.captureScreenshot', { format: 'jpeg', quality: 65 });
  const migrationPreview = Buffer.from(migrationShot.data, 'base64');
  assert(migrationPreview.length < 300 * 1024);
  await writeFile(path.join(out, 'migration-cleanup.jpg'), migrationPreview);
  await writeFile(
    path.join(out, 'results.json'),
    JSON.stringify(
      {
        passed: checks.length,
        checks,
        scope: 'Edge无头实际渲染与合成IPC；未操作原生文件对话框或真实剪贴板',
      },
      null,
      2,
    ),
  );
  console.log(JSON.stringify({ passed: checks.length, checks }));
  await appearanceRecorder.finish();
  await send('Browser.close').catch(() => {});
} finally {
  ws?.close();
  browser.kill();
  await new Promise((resolve) => server.close(resolve));
}

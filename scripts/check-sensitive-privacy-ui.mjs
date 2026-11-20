// 离屏渲染 + 合成 IPC 数据；不操作用户桌面，不代表原生粘贴验收。
import { createServer } from 'node:http';
import { spawn } from 'node:child_process';
import { readFile, writeFile, mkdir } from 'node:fs/promises';
import path from 'node:path';
import assert from 'node:assert/strict';
import { settingsMock } from './settings-ui-mock.mjs';
const root = process.cwd();
const out = path.join(root, '.tools/privacy-ui');
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
    const callbacks=new Map(),events=new Map();let id=0;
    window.ready=false;window.calls=[];window.clearImages=[];window.maskedImages=[];
    window.rows=[
      {id:'image',kind:'image',summary:'••••••',sensitive:true,expires_at:Date.now()+600000,thumbnail:null,preview:null,placeholder:null,files:[],formats:['PNG'],pinned:false,favorite:false,group_id:null},
      {id:'image-file',kind:'files',summary:'synthetic••••png',sensitive:true,expires_at:null,thumbnail:null,preview:null,placeholder:null,files:[],formats:['CF_HDROP'],pinned:false,favorite:false,group_id:null},
      {id:'text',kind:'text',summary:'合成普通文本',sensitive:false,thumbnail:null,files:[],formats:['CF_UNICODETEXT'],pinned:false,favorite:false,group_id:null}
    ];
    const imageBytes=(masked)=>{
      const canvas=document.createElement('canvas');canvas.width=240;canvas.height=160;
      const ctx=canvas.getContext('2d');ctx.fillStyle='#b9d8d2';ctx.fillRect(0,0,240,160);
      for(let y=0;y<8;y++)for(let x=0;x<12;x++){
        ctx.fillStyle=['#317f7d','#f5c3a7','#e8dcb0','#376472'][(x*3+y)%4];
        ctx.fillRect(x*20,y*20,20,20);
      }
      if(!masked){ctx.fillStyle='#ffffff';ctx.font='bold 22px sans-serif';ctx.fillText('TEST IMAGE',45,88);}
      return Array.from(atob(canvas.toDataURL('image/png').split(',')[1]),c=>c.charCodeAt(0));
    };
    window.masked=()=>imageBytes(true);window.clear=()=>imageBytes(false);
    window.emitEvent=(event,payload)=>callbacks.get(events.get(event))?.({event,id:1,payload});
    window.__TAURI_EVENT_PLUGIN_INTERNALS__={unregisterListener(){}};
    window.__TAURI_INTERNALS__={
      transformCallback(fn){callbacks.set(++id,fn);return id;},convertFileSrc:hash=>'/mock/'+hash,
      async invoke(cmd,args={}) {
        window.calls.push({cmd,args});
        if(cmd==='plugin:event|listen'){events.set(args.event,args.handler);return id;}
        if(cmd==='list_items')return args.offset?[]:window.rows.map(row=>({...row}));
        if(cmd==='list_groups')return [];
        if(cmd==='last_error')return null;
        if(cmd==='frontend_ready'){window.ready=true;return;}
        if(cmd==='item_detail')return {text:args.id==='text'?'合成普通文本':'',truncated:false,files:[],...(args.id!=='text'?{image:window.masked()}: {})};
        if(cmd==='reveal_item'){
          if(window.holdReveal)return new Promise(resolve=>window.releaseReveal=()=>resolve({text:'',truncated:false,files:[],image:window.clear()}));
          return {text:'',truncated:false,files:args.id==='image-file'?[{path:'C:/synthetic/image.png',exists:true}]:[],image:window.clear()};
        }
        if(cmd==='keep_sensitive'){window.rows[0].expires_at=null;window.emitEvent('history-changed');return 1;}
        if(cmd==='compact_preview_geometry')return {right:true,arrow:60};
        if(cmd==='compact_preview_state')return {generation:1,item:window.rows[0],detail:{text:'',truncated:false,files:[],image:window.masked()}};
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
  const check = (name) => checks.push(name);
  const navigate = async (query = '') => {
    await send('Page.navigate', { url: origin + '/' + query });
    await until(() => evaluate('window.ready===true'), '页面未 ready');
  };
  const button = async (label) => {
    assert(
      await evaluate(
        `(()=>{const button=[...document.querySelectorAll('button')].find(el=>el.textContent.trim()===${JSON.stringify(label)});if(!button)return false;button.click();return true;})()`,
      ),
      `缺少按钮 ${label}`,
    );
    await pause(60);
  };
  const image = () => evaluate("document.querySelector('.sensitive-image img')?.src");
  const waitImage = async () =>
    until(
      () => evaluate("!!document.querySelector('.sensitive-image img')?.complete"),
      '图片未显示',
    );
  const capture = async (name) => {
    const shot = await send('Page.captureScreenshot', { format: 'jpeg', quality: 65 });
    const bytes = Buffer.from(shot.data, 'base64');
    assert(bytes.length <= 300 * 1024);
    await writeFile(path.join(out, name + '.jpg'), bytes);
  };
  await navigate();
  await waitImage();
  assert(await evaluate("document.querySelector('.sensitive-image img').naturalWidth===240"));
  assert(!(await evaluate("document.body.textContent.includes('记录没有可用的图片预览')")));
  assert.equal(await evaluate("window.calls.filter(call=>call.cmd==='reveal_item').length"), 0);
  check('敏感图片默认显示后端马赛克，无原图请求与错误占位');
  const masked = await image();
  await capture('masked');
  await button('显示原文');
  await waitImage();
  const clear = await image();
  assert.notEqual(clear, masked);
  assert(await evaluate("document.body.textContent.includes('正在显示原图')"));
  assert(await evaluate(`fetch(${JSON.stringify(masked)}).then(()=>false,()=>true)`));
  check('显示原文后切换为清晰图片，并释放马赛克地址');
  const geometry = await evaluate(`(()=>{
    const image=document.querySelector('.sensitive-image img').getBoundingClientRect();
    const actions=document.querySelector('.sensitive-actions').getBoundingClientRect();
    const detail=document.querySelector('.detail').getBoundingClientRect();
    const footer=document.querySelector('.detail-bottom').getBoundingClientRect();
    return image.top>actions.bottom&&image.left>=detail.left&&image.right<=detail.right&&image.bottom<footer.top;
  })()`);
  assert(geometry);
  check('真实布局中按钮、图片和底部操作无重叠或横向溢出');
  await capture('revealed');
  await button('隐藏原文');
  await waitImage();
  assert(await evaluate(`fetch(${JSON.stringify(clear)}).then(()=>false,()=>true)`));
  assert(await evaluate("document.body.textContent.includes('马赛克遮挡')"));
  check('隐藏原文恢复马赛克并撤销原图地址');
  await evaluate('window.holdReveal=true');
  await button('显示原文');
  await evaluate("document.querySelector('[data-id=text]').click()");
  await evaluate('window.releaseReveal()');
  await pause(70);
  assert.equal(await image(), undefined);
  assert(
    await evaluate("document.querySelector('.text-detail').textContent.includes('合成普通文本')"),
  );
  check('切换记录后迟到的原图结果不会覆盖新记录');
  await evaluate(
    "document.querySelector('[data-id=image]').dispatchEvent(new MouseEvent('contextmenu',{bubbles:true,clientX:230,clientY:200}))",
  );
  await button('取消敏感删除倒计时');
  await until(
    () =>
      evaluate("document.querySelector('[data-id=image] .sensitive-badge')?.textContent==='敏感'"),
    '倒计时状态未更新',
  );
  assert(
    await evaluate(
      "window.calls.some(call=>call.cmd==='keep_sensitive'&&call.args.ids[0]==='image')",
    ),
  );
  check('取消敏感删除倒计时后，记录只显示敏感标记');
  await evaluate(
    "document.querySelector('[data-id=image]').dispatchEvent(new MouseEvent('contextmenu',{bubbles:true,clientX:230,clientY:200}))",
  );
  assert(
    await evaluate(
      "[...document.querySelectorAll('[role=menuitem]')].some(el=>el.textContent.trim()==='已取消敏感删除倒计时'&&el.disabled)",
    ),
  );
  await evaluate("document.body.dispatchEvent(new PointerEvent('pointerdown',{bubbles:true}))");
  check('已取消敏感删除倒计时的记录禁用重复操作');
  await evaluate("window.holdReveal=false;document.querySelector('[data-id=image-file]').click()");
  await waitImage();
  assert(
    await evaluate("document.querySelector('.detail-caption').textContent.includes('文件预览')"),
  );
  assert(
    await evaluate(
      "document.querySelector('[data-id=image-file] .sensitive-badge').textContent==='敏感'",
    ),
  );
  assert.equal(await evaluate("document.querySelectorAll('.file-detail li').length"), 0);
  const fileMasked = await image();
  await capture('file-masked');
  check('图片文件类型也显示马赛克，隐藏路径且无倒计时项仅标记敏感');
  await button('显示原文');
  await waitImage();
  const fileClear = await image();
  assert.notEqual(fileClear, fileMasked);
  assert.deepEqual(
    await evaluate(`[
      document.querySelector('.file-detail .file-heading strong')?.textContent,
      document.querySelector('.file-detail .file-parent')?.textContent,
      document.querySelector('.file-detail .file-parent')?.title,
    ]`),
    ['image.png', 'C:/synthetic', 'C:/synthetic/image.png'],
  );
  await capture('file-revealed');
  await button('隐藏原文');
  await waitImage();
  assert(await evaluate(`fetch(${JSON.stringify(fileClear)}).then(()=>false,()=>true)`));
  assert.equal(await evaluate("document.querySelectorAll('.file-detail li').length"), 0);
  check('图片文件显式查看清晰图与文件信息，隐藏后恢复马赛克并释放清晰图');
  await evaluate("document.querySelector('button[aria-label=设置]').click()");
  await until(() => evaluate("!!document.querySelector('.settings-view form')"), '设置未打开');
  await button('隐私');
  const duration =
    "[...document.querySelectorAll('label')].find(el=>el.textContent.includes('敏感内容保留时长'))";
  assert(
    await evaluate(duration + ".querySelector('.duration-input > span').textContent==='分钟'"),
  );
  assert.equal(await evaluate(duration + ".querySelector('input').min"), '0');
  assert(!(await evaluate("document.body.textContent.includes('敏感内容保留分钟数')")));
  assert(
    await evaluate(
      "!document.body.textContent.includes('永久保留') && document.body.textContent.includes('0 为不设敏感删除倒计时')",
    ),
  );
  check('设置显示新文案与分钟单位，最小值允许 0');
  assert(
    await evaluate(`(()=>{
    const label=${duration};
    const input=label.querySelector('input').getBoundingClientRect();
    const unit=label.querySelector('.duration-input > span').getBoundingClientRect();
    return input.width>0 && unit.left>=input.right && unit.top>=input.top && unit.bottom<=input.bottom;
  })()`),
  );
  await capture('privacy-settings');
  await evaluate(
    `(()=>{const input=${duration}.querySelector('input');input.value='0';input.dispatchEvent(new Event('input',{bubbles:true}));})()`,
  );
  await button('保存');
  await until(() => evaluate('window.settingUpdates.length===1'), '设置未提交');
  assert.equal(await evaluate('window.settingUpdates[0].sensitive_ttl_minutes'), 0);
  check('全局保留时长 0 按数值类型保存');
  await send('Page.navigate', { url: origin + '/?panel=preview' });
  await until(
    () => evaluate("!!document.querySelector('.floating-preview img')?.complete"),
    '紧凑浮窗图片未显示',
  );
  assert.equal(await evaluate("document.querySelector('.floating-preview img').naturalWidth"), 240);
  assert(
    await evaluate(
      "document.querySelector('.floating-preview header').textContent.includes('已打码')",
    ),
  );
  assert.equal(await evaluate("window.calls.filter(call=>call.cmd==='reveal_item').length"), 0);
  await capture('floating-masked');
  check('紧凑浮动预览显示马赛克，仍保持只读且不读取清晰原图');
  await evaluate(
    "window.emitEvent('compact-preview-changed',{generation:2,item:window.rows.find(row=>row.id==='image-file'),detail:{text:'',truncated:false,files:[],image:window.masked()}})",
  );
  await until(
    () =>
      evaluate(
        "document.querySelector('.floating-preview header')?.textContent.includes('文件预览') && document.querySelector('.floating-preview img')?.complete",
      ),
    '紧凑浮窗未显示图片文件',
  );
  assert.equal(await evaluate("document.querySelector('.floating-preview img').naturalWidth"), 240);
  assert.equal(await evaluate("document.querySelectorAll('.floating-files li').length"), 0);
  check('紧凑浮窗对文件类型的图片同样显示马赛克，仍隐藏文件路径');
  const result = {
    passed: checks.length,
    failed: 0,
    checks,
    scope: '独立 Edge 无头真实渲染与合成 IPC，不操作桌面剪贴板或真实键鼠',
  };
  await writeFile(path.join(out, 'results.json'), JSON.stringify(result, null, 2));
  console.log(JSON.stringify(result, null, 2));
  await send('Browser.close').catch(() => {});
} finally {
  ws?.close();
  browser.kill();
  await new Promise((resolve) => server.close(resolve));
}

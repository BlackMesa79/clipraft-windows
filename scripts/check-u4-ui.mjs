import { createAppearanceRecorder } from './appearance-baseline.mjs';
// 离屏渲染 + 合成 IPC 数据；不操作用户桌面，不代表原生粘贴验收。
import { createServer } from 'node:http';
import { spawn } from 'node:child_process';
import { readFile, writeFile, mkdir } from 'node:fs/promises';
import path from 'node:path';
import assert from 'node:assert/strict';
import { settingsMock } from './settings-ui-mock.mjs';
const root = process.cwd();
const out = path.join(root, '.tools/u4-ui');
const profile = path.join(out, 'profile-' + Date.now());
await mkdir(profile, { recursive: true });
const server = createServer(async (req, res) => {
  try {
    const url = new URL(req.url, 'http://127.0.0.1');
    if (url.pathname === '/mock/preview') {
      res.writeHead(200, { 'Content-Type': 'image/svg+xml' });
      res.end(
        '<svg xmlns="http://www.w3.org/2000/svg" width="240" height="120"><rect width="240" height="120" fill="#277d77"/><circle cx="120" cy="60" r="40" fill="#eaf4f2"/></svg>',
      );
      return;
    }
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
    window.calls=[];window.ready=false;window.imageRequests=[];
    window.emitEvent=(event,payload)=>callbacks.get(events.get(event))?.({event,id:1,payload});
    window.__TAURI_EVENT_PLUGIN_INTERNALS__={unregisterListener(){}};
    window.__TAURI_INTERNALS__={
      transformCallback(fn){callbacks.set(++id,fn);return id;},
      convertFileSrc(hash){window.imageRequests.push(hash);return '/mock/'+hash;},
      async invoke(cmd,args={}) {
        window.calls.push({cmd,args});
        if(cmd==='plugin:event|listen'){events.set(args.event,args.handler);return id;}
        if(cmd==='panel_state')return new Promise(resolve=>{window.resolveInitial=resolve;});
        if(cmd==='frontend_ready'){window.ready=true;return;}
        return 1;
      }
    };
  })();`;
  await send('Page.enable');
  await send('Runtime.enable');
  await send('Page.addScriptToEvaluateOnNewDocument', { source: mock + settingsMock });
  await send('Emulation.setDeviceMetricsOverride', {
    width: 460,
    height: 530,
    deviceScaleFactor: 1,
    mobile: false,
  });
  await send('Page.navigate', { url: origin + '/?panel=quick' });
  await until(() => evaluate('typeof window.resolveInitial==="function"'), '初始快照未请求');
  const checks = [];
  const check = (name) => checks.push(name);
  let serial = 1;
  const row = (id, extra = {}) => ({
    id,
    kind: 'text',
    summary: '合成记录 ' + id,
    formats: ['CF_UNICODETEXT'],
    files: [],
    ...extra,
  });
  const payload = (overrides = {}) => ({
    serial: serial++,
    session: 1,
    position: 220,
    total: 301,
    loading: false,
    visible: true,
    error: null,
    selected: 2,
    visible_count: 5,
    items: Array.from({ length: 5 }, (_, index) => row('id-' + (218 + index))),
    ...overrides,
  });
  const emit = async (panel) => {
    await evaluate(`window.emitEvent('panel-state',${JSON.stringify(panel)})`);
    await pause(40);
  };
  const selected = () => evaluate("document.querySelector('.item.selected')?.dataset.id");
  const appearanceRecorder = createAppearanceRecorder(evaluate, out);
  const capture = async (name) => {
    await appearanceRecorder.capture(name);
    const shot = await send('Page.captureScreenshot', { format: 'jpeg', quality: 65 });
    const bytes = Buffer.from(shot.data, 'base64');
    assert(bytes.length <= 300 * 1024);
    await writeFile(path.join(out, name + '.jpg'), bytes);
  };
  const center = async () =>
    evaluate(`(() => {
    const r=document.querySelector('.item.selected')?.getBoundingClientRect();
    return r ? {y:(r.top+r.bottom)/2,top:r.top,bottom:r.bottom} : null;
  })()`);
  await evaluate(`window.resolveInitial(${JSON.stringify(payload())})`);
  await until(() => evaluate('window.ready===true'), '前端未 ready');
  for (const count of [3, 4, 5, 6, 7, 14, 15]) {
    const height = 104 + Math.floor(count / 2) * 152;
    await send('Emulation.setDeviceMetricsOverride', {
      width: 420,
      height,
      deviceScaleFactor: 1,
      mobile: false,
    });
    await emit(
      payload({
        visible_count: count,
        items: Array.from({ length: count }, (_, i) => row('count-' + i)),
        selected: Math.floor(count / 2),
      }),
    );
    await pause(180);
    assert.equal(await evaluate("document.querySelectorAll('.item').length"), count);
    assert.equal(
      await evaluate(
        "new Set([...document.querySelectorAll('.item')].map(el=>el.dataset.id)).size",
      ),
      count,
    );
    assert(Math.abs((await center()).y - height / 2) < 1);
    assert(
      await evaluate(
        "(()=>{const r=document.querySelector('.item.selected').getBoundingClientRect();return r.left>=0&&r.right<=innerWidth;})()",
      ),
    );
    assert(await evaluate('document.documentElement.scrollWidth<=innerWidth'));
    assert(
      await evaluate(`(() => {
        const hint = document.querySelector('.cancel-hint');
        const rect = hint.getBoundingClientRect();
        const count = document.querySelector('.count').getBoundingClientRect();
        return hint.textContent.trim() === '空格 / 右键取消' &&
          rect.left >= 0 && rect.top >= 0 && rect.top < innerHeight / 2 &&
          rect.right < count.left && Math.abs(rect.top - count.top) < 1;
      })()`),
    );
    assert.equal(
      await evaluate("window.calls.filter(c=>c.cmd==='resize_quick').at(-1).args.height"),
      height,
    );
    check(`${count} 条：准确数量、唯一 ID、前后端几何和中心锚点`);
  }
  const frozenHeight = await evaluate(
    "window.calls.filter(c=>c.cmd==='resize_quick').at(-1).args.height",
  );
  await evaluate("window.emitEvent('settings-changed',{...window.settings,quick_visible_count:3})");
  await pause(80);
  assert.equal(
    await evaluate("window.calls.filter(c=>c.cmd==='resize_quick').at(-1).args.height"),
    frozenHeight,
  );
  assert.equal(await evaluate("document.querySelectorAll('.item').length"), 15);
  check('活动会话条数保持快照，设置变更不提前缩小舞台');
  for (const total of [1, 2, 3]) {
    await emit(
      payload({
        total,
        position: total - 1,
        visible_count: 5,
        selected: Math.floor(total / 2),
        items: Array.from({ length: total }, (_, i) => row('short-' + i)),
      }),
    );
    assert.equal(await evaluate("document.querySelectorAll('.item').length"), total);
    await pause(180);
    assert(Math.abs((await center()).y - 1168 / 2) < 1);
    check(`${total} 条历史留空，不复制卡片，中心稳定`);
  }
  await send('Emulation.setDeviceMetricsOverride', {
    width: 340,
    height: 280,
    deviceScaleFactor: 1.5,
    mobile: false,
  });
  await emit(
    payload({
      visible_count: 15,
      selected: 7,
      items: Array.from({ length: 15 }, (_, i) => row('scaled-' + i)),
    }),
  );
  await pause(180);
  const limited = await center();
  assert(Math.abs(limited.y - 140) < 1 && limited.top >= 0 && limited.bottom <= 280);
  assert(await evaluate('document.documentElement.scrollWidth<=innerWidth'));
  check('工作区限高/限宽时整体缩放，中心完整可见，无横向溢出');
  await send('Emulation.setDeviceMetricsOverride', {
    width: 420,
    height: 408,
    deviceScaleFactor: 1,
    mobile: false,
  });
  const rows = Array.from({ length: 5 }, (_, i) => row('move-' + i));
  await emit(payload({ session: 2, position: 0, total: 301, items: rows }));
  await pause(180);
  await emit(payload({ session: 2, position: 1, total: 301, items: [], loading: true }));
  assert.equal(await evaluate("document.querySelectorAll('.item').length"), 0);
  await emit(
    payload({ session: 2, position: 1, total: 301, items: [...rows.slice(1), row('move-5')] }),
  );
  assert.equal(await selected(), 'move-3');
  assert(Math.abs((await center()).y - 204) > 1, '过渡中实际渲染尚未到终态');
  assert.equal(
    await evaluate("getComputedStyle(document.querySelector('.item.selected')).transitionDuration"),
    '0.14s',
  );
  check('加载立即移除旧正文，异步新邻域恢复后使用 140ms 过渡');
  for (let i = 2; i < 12; i++) {
    await emit(
      payload({
        session: 2,
        position: i,
        items: Array.from({ length: 5 }, (_, j) => row('burst-' + (i + j))),
      }),
    );
  }
  await pause(200);
  assert.equal(await selected(), 'burst-13');
  assert(Math.abs((await center()).y - 204) < 1);
  check('快速连续输入直接到最新选择，结束后无积压动画');
  await emit(payload({ session: 3, position: 200, items: rows }));
  assert.equal(await evaluate("document.querySelectorAll('.wheel-snap').length"), 5);
  await pause(180);
  assert(Math.abs((await center()).y - 204) < 1);
  await emit(payload({ session: 3, position: 220, items: [...rows].reverse(), selected: 2 }));
  assert.equal(
    await evaluate("getComputedStyle(document.querySelector('.item')).transitionDuration"),
    '0s',
  );
  assert(Math.abs((await center()).y - 204) < 1);
  check('新会话/大跨度跳转立即使用终态');
  await send('Emulation.setEmulatedMedia', {
    features: [{ name: 'prefers-reduced-motion', value: 'reduce' }],
  });
  await emit(payload({ session: 3, position: 201, items: rows }));
  assert.equal(
    await evaluate("getComputedStyle(document.querySelector('.item')).transitionDuration"),
    '0s',
  );
  check('减少动效时禁用过渡，选中语义一致');
  await send('Emulation.setEmulatedMedia', { features: [] });
  for (const theme of ['light', 'dark']) {
    await evaluate(
      `window.emitEvent('settings-changed',{...window.settings,theme:${JSON.stringify(theme)}})`,
    );
    await emit(
      payload({
        session: 4,
        position: 0,
        items: [
          row('image', { kind: 'image', thumbnail: 'preview' }),
          row('file', { kind: 'files', files: ['C:/synthetic/report.txt'] }),
          row('secret', {
            sensitive: true,
            summary: '••••••',
            thumbnail: 'forbidden',
            expires_at: null,
          }),
          row('long', { summary: '长文本'.repeat(80) }),
          row('expiry', { sensitive: true, summary: '••••••', expires_at: Date.now() + 120000 }),
        ],
      }),
    );
    await pause(180);
    assert(
      await evaluate(
        "[document.documentElement,document.body,document.querySelector('#app'),document.querySelector('.quick')].every(el=>getComputedStyle(el).backgroundColor==='rgba(0, 0, 0, 0)')",
      ),
    );
    assert(!(await evaluate("window.imageRequests.includes('forbidden')")));
    assert(
      await evaluate(
        "document.body.textContent.includes('无计时')&&document.body.textContent.includes('分钟后删除')",
      ),
    );
    assert(
      await evaluate(
        "[...document.querySelectorAll('.summary')].every(el=>getComputedStyle(el).whiteSpace==='nowrap')",
      ),
    );
    assert(await evaluate("document.querySelector('.preview img').naturalWidth===240"));
    assert.equal(
      await evaluate("getComputedStyle(document.querySelector('.preview img')).objectFit"),
      'contain',
    );
    assert.equal(await evaluate('document.documentElement.dataset.skin'), 'clipraft');
    assert.equal(
      await evaluate("getComputedStyle(document.querySelector('.wheel-card')).borderRadius"),
      '8px',
    );
    assert.equal(
      await evaluate(
        "getComputedStyle(document.querySelector('.wheel-card')).getPropertyValue('--motion-quick-duration').trim()",
      ),
      '140ms',
    );
    await capture('wheel-' + theme);
    check(`${theme} 主题：透明根、单行文字、文件名、敏感无计时/倒计时与缩略图隔离`);
  }
  await evaluate(
    "window.emitEvent('settings-changed',{...window.settings,skin:'fluent',theme:'dark'})",
  );
  await pause(180);
  assert(
    await evaluate(
      "(()=>{const s=n=>getComputedStyle(document.querySelector(n));return s('.quick').backgroundColor==='rgba(0, 0, 0, 0)'&&s('.wheel-card.selected').height==='64px'&&s('.wheel-card.selected').backgroundColor==='rgb(48, 72, 67)'&&s('.cancel-hint').borderTopWidth==='1px'&&s('.wheel-card').boxShadow!=='none';})()",
    ),
  );
  await capture('fluent-dark-wheel');
  assert.deepEqual(
    await evaluate(
      "(()=>{const style=getComputedStyle(document.querySelector('.wheel-card.selected .summary'));return [style.fontSize,style.fontWeight];})()",
    ),
    ['13px', '400'],
  );
  await evaluate(
    "window.emitEvent('settings-changed',{...window.settings,skin:'clipraft',theme:'dark'})",
  );
  check('U7 Fluent 滚轮保留透明舞台、固定卡高、选中描边与独立阴影');
  await emit(payload({ loading: true, items: [] }));
  assert.equal(await evaluate("!!document.querySelector('.cancel-hint')"), true);
  const loadingHeight = await evaluate(
    "window.calls.filter(c=>c.cmd==='resize_quick').at(-1).args.height",
  );
  await emit(payload({ error: '合成加载失败', items: [] }));
  assert.equal(
    await evaluate("window.calls.filter(c=>c.cmd==='resize_quick').at(-1).args.height"),
    loadingHeight,
  );
  assert(await evaluate("document.body.textContent.includes('所选记录暂时无法显示')"));
  assert(
    await evaluate(
      "getComputedStyle(document.querySelector('.empty h2')).color===getComputedStyle(document.querySelector('.quick')).color",
    ),
  );
  await capture('wheel-error');
  check('加载/失败胶囊保持同配置窗口几何');
  await emit(payload({ visible: false, items: [] }));
  assert.equal(await evaluate("!!document.querySelector('.cancel-hint')"), false);
  check('取消提示位于面板左上，加载时保留，面板关闭后消失');
  await emit(payload({ visible: false, error: '合成粘贴失败', items: [] }));
  assert(
    await evaluate(
      "!!document.querySelector('.notice-only [role=alert]')&&!document.querySelector('.wheel-stage')",
    ),
  );
  await capture('notice');
  check('独立错误使用小胶囊，无旧卡片内容');
  assert.equal(
    await evaluate(
      "window.calls.filter(c=>['list_items','item_detail','reveal_item','paste_item'].includes(c.cmd)).length",
    ),
    0,
  );
  check('展示不读取原文、不自行分页或提交粘贴');
  await emit(payload({ session: 5, position: 0, items: rows }));
  await pause(180);
  assert(
    await evaluate(
      "!document.querySelector('.quick header,.quick footer')&&document.querySelector('.count-current').textContent.trim()==='1'&&document.querySelector('.count-total').textContent.trim()==='301'",
    ),
  );
  assert.equal(
    await evaluate("getComputedStyle(document.querySelector('.count')).writingMode"),
    'horizontal-tb',
  );
  assert(
    await evaluate(
      "[...document.querySelectorAll('.item')].every(el=>getComputedStyle(el).opacity==='1')",
    ),
  );
  assert(
    await evaluate(
      "(()=>{const c=document.querySelector('.count-current').getBoundingClientRect(),t=document.querySelector('.count-total').getBoundingClientRect(),r=document.querySelector('.count').getBoundingClientRect(),s=document.querySelector('.wheel-stage').getBoundingClientRect();return Math.abs(c.top-t.top)<1&&c.right<t.left&&r.top<s.top+30&&r.right>s.right-30&&!document.querySelector('.count-divider')&&document.querySelector('.count').textContent.replace(/\\s/g,'')==='1/301条';})()",
    ),
  );
  const gaps = await evaluate(
    "(()=>{const rows=[...document.querySelectorAll('.item')].map(el=>el.getBoundingClientRect());return rows.slice(1).map((r,i)=>r.top-rows[i].bottom);})()",
  );
  assert(
    gaps.every((gap) => gap >= 8),
    JSON.stringify(gaps),
  );
  check('移除操作说明，右上角横排显示位置与总数，卡片不透明且间隔清楚');
  await capture('wheel-spacing');
  for (const theme of ['light', 'dark']) {
    await evaluate(
      `window.emitEvent('settings-changed',{...window.settings,theme:${JSON.stringify(theme)},quick_background:'native'})`,
    );
    await pause(80);
    assert.equal(await evaluate("document.querySelector('.wheel-backdrop')"), null);
    assert(
      await evaluate(
        "[...document.querySelectorAll('.wheel-card')].every(el=>getComputedStyle(el).boxShadow!=='none'&&getComputedStyle(el).opacity==='1')",
      ),
    );
    await capture('card-shadows-' + theme);
    check(`${theme}：所有卡片有独立阴影、保持不透明、旧背景设置不恢复背景`);
  }

  for (const theme of ['light', 'dark']) {
    await evaluate(
      `window.emitEvent('settings-changed',{...window.settings,theme:${JSON.stringify(theme)}})`,
    );
    await emit(payload({ visible: false, items: [], error: '源文件已失效，无法粘贴' }));
    assert(
      await evaluate(
        "(()=>{const n=document.querySelector('.quick-notice'),r=n.getBoundingClientRect(),m=document.querySelector('.notice-only').getBoundingClientRect();return getComputedStyle(n).boxShadow!=='none'&&n.querySelector('strong').textContent==='操作失败'&&n.querySelector('p').textContent==='源文件已失效，无法粘贴'&&r.left>=20&&r.right<=innerWidth-20&&r.top>=12&&m.bottom-r.bottom>=12;})()",
      ),
    );
    await capture('notice-' + theme);
    await emit(
      payload({
        visible: false,
        items: [],
        error: '源文件路径过长或暂时无法读取，请检查文件状态。'.repeat(50),
      }),
    );
    assert(
      await evaluate(
        "document.querySelector('main').getBoundingClientRect().height<=530&&document.documentElement.scrollWidth<=innerWidth",
      ),
    );
    check(`${theme}：错误提示图标/标题/正文分层、阴影空间与长消息限高`);
  }
  const stable = payload({
    session: 60,
    visible_count: 4,
    selected: 2,
    position: 220,
    items: Array.from({ length: 4 }, (_, i) => row('u5-' + i)),
  });
  await emit(stable);
  await evaluate(
    "window.emitEvent('settings-changed',{...window.settings,theme:'dark',layout:'cards',compact:true,quick_visible_count:15})",
  );
  await pause(60);
  assert.equal(await selected(), 'u5-2');
  assert.equal(await evaluate("document.querySelectorAll('.wheel-card').length"), 4);
  assert.equal(
    await evaluate("window.calls.filter(c=>c.cmd==='resize_quick').at(-1).args.height"),
    408,
  );
  check('U5 主窗口布局/条数设置变化不重排活动quick会话或修改其固定几何');
  await emit(payload({ session: 60, visible_count: 4, loading: true, items: [], selected: 0 }));
  await emit(stable);
  assert.equal(await evaluate("document.querySelectorAll('.wheel-card').length"), 0);
  await emit(
    payload({
      session: 60,
      visible_count: 4,
      selected: 0,
      items: [
        row('u5-sensitive', { summary: '••••••', sensitive: true, thumbnail: 'must-not-load' }),
      ],
    }),
  );
  assert.equal(await selected(), 'u5-sensitive');
  assert(!(await evaluate("window.imageRequests.includes('must-not-load')")));
  assert(!(await evaluate("document.body.textContent.includes('合成记录 u5-2')")));
  check('U5 设置变化后安全失效立即清除正文，旧事件不复活摘要，敏感缩略图不读取');
  const notice = payload({ session: 60, visible: false, items: [], error: '合成删除/到期失败' });
  await emit(notice);
  await emit(
    payload({
      session: 61,
      total: 1,
      position: 0,
      selected: 0,
      visible_count: 15,
      items: [row('u5-reopened')],
    }),
  );
  await emit(notice);
  assert.equal(await selected(), 'u5-reopened');
  assert.equal(await evaluate("document.querySelector('.notice-only')"), null);
  const reopenedSize = await evaluate("window.calls.filter(c=>c.cmd==='resize_quick').at(-1).args");
  assert.deepEqual(reopenedSize, { height: 1168, notice: false, session: 61 });
  assert.equal(
    await evaluate(
      "window.calls.filter(c=>['paste_item','reveal_item','item_detail','list_items','sync_main_layout'].includes(c.cmd)).length",
    ),
    0,
  );
  check('U5 失败提示后重开单条新会话，旧notice不覆盖选择/几何，不调用主窗或粘贴IPC');
  const result = {
    passed: checks.length,
    failed: 0,
    checks,
    scope: 'Edge 无头实际渲染、合成 IPC；原生透明/焦点/DPI/外部粘贴待人工验收',
  };
  await writeFile(path.join(out, 'results.json'), JSON.stringify(result, null, 2));
  console.log(JSON.stringify(result, null, 2));
  await appearanceRecorder.finish();
  await send('Browser.close').catch(() => {});
} finally {
  ws?.close();
  browser.kill();
  await new Promise((resolve) => server.close(resolve));
}

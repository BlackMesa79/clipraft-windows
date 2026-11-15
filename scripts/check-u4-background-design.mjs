// 离屏渲染 + 合成 IPC 数据；不操作用户桌面，不代表原生粘贴验收。
import { createServer } from 'node:http';
import { spawn } from 'node:child_process';
import { readFile, writeFile, mkdir } from 'node:fs/promises';
import path from 'node:path';
import assert from 'node:assert/strict';
const root = process.cwd();
const out = path.join(root, '.tools/u4-background-design');
const profile = path.join(out, 'profile-' + Date.now());
await mkdir(profile, { recursive: true });
const server = createServer(async (req, res) => {
  try {
    const url = new URL(req.url, 'http://127.0.0.1');
    const file = path.join(
      root,
      'docs/design',
      url.pathname === '/' ? 'u4-background.html' : url.pathname,
    );
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
  const checks = [];
  const capture = async (name) => {
    const layout = await send('Page.getLayoutMetrics');
    const shot = await send('Page.captureScreenshot', {
      format: 'jpeg',
      quality: 75,
      captureBeyondViewport: true,
      clip: { x: 0, y: 0, width: 1240, height: Math.ceil(layout.cssContentSize.height), scale: 1 },
    });
    await writeFile(path.join(out, name + '.jpg'), Buffer.from(shot.data, 'base64'));
  };
  await send('Page.enable');
  await send('Runtime.enable');
  await send('Emulation.setDeviceMetricsOverride', {
    width: 1240,
    height: 1350,
    deviceScaleFactor: 1,
    mobile: false,
  });
  await send('Page.navigate', { url: origin + '/u4-background.html' });
  await until(
    () => evaluate("document.querySelectorAll('.wheel-card').length===20"),
    '设计稿未渲染',
  );
  assert.equal(await evaluate("document.querySelectorAll('.proposal').length"), 4);
  checks.push('四种独立背景方案，每种 5 张卡片实际完成渲染');
  await evaluate("document.getElementById('next').click()");
  assert(
    await evaluate(
      "[...document.querySelectorAll('.count')].every(el=>el.textContent==='5 / 301 条')",
    ),
  );
  await evaluate("document.getElementById('previous').click()");
  assert(
    await evaluate(
      "[...document.querySelectorAll('.count')].every(el=>el.textContent==='4 / 301 条')",
    ),
  );
  checks.push('各方案同步切换记录，保持当前计数布局');
  for (const [theme, scene] of [
    ['light', 'editor'],
    ['dark', 'editor'],
    ['light', 'document'],
    ['dark', 'wallpaper'],
  ]) {
    await evaluate(
      `document.getElementById('theme').value=${JSON.stringify(theme)};document.getElementById('theme').dispatchEvent(new Event('change'));document.getElementById('scene').value=${JSON.stringify(scene)};document.getElementById('scene').dispatchEvent(new Event('change'))`,
    );
    await pause(150);
    assert(await evaluate('document.documentElement.scrollWidth<=innerWidth'));
    assert(
      await evaluate(
        "[...document.querySelectorAll('.wheel-card')].every(el=>getComputedStyle(el).opacity==='1')",
      ),
    );
    assert(
      await evaluate(
        "[...document.querySelectorAll('.canvas')].every(canvas=>{const a=canvas.getBoundingClientRect(),b=canvas.querySelector('.selected').getBoundingClientRect();return b.left>=a.left&&b.right<=a.right&&b.top>=a.top&&b.bottom<=a.bottom})",
      ),
    );
    await capture(theme + '-' + scene);
    checks.push(`${theme}/${scene}：不透明卡片、中心项完整、无横向溢出并截图`);
  }
  await evaluate("document.getElementById('baseline').click()");
  assert(
    await evaluate(
      "[...document.querySelectorAll('.wheel-backdrop')].every(el=>getComputedStyle(el).visibility==='hidden')",
    ),
  );
  assert.equal(await evaluate("document.querySelectorAll('.selected').length"), 4);
  await evaluate("document.getElementById('baseline').click()");
  checks.push('原版无背景对照仅关闭衬底，卡片和选中项保持');
  await send('Emulation.setDeviceMetricsOverride', {
    width: 500,
    height: 850,
    deviceScaleFactor: 1,
    mobile: false,
  });
  await pause(150);
  assert(await evaluate('document.documentElement.scrollWidth<=500'));
  assert(
    await evaluate(
      "[...document.querySelectorAll('.canvas')].every(canvas=>{const a=canvas.getBoundingClientRect(),b=canvas.querySelector('.selected').getBoundingClientRect();return b.left>=a.left&&b.right<=a.right})",
    ),
  );
  checks.push('窄屏单列展示，中心卡片不溢出');
  await writeFile(
    path.join(out, 'results.json'),
    JSON.stringify(
      {
        passed: checks.length,
        checks,
        scope: '独立 HTML/CSS 设计稿实际渲染；不代表原生跨窗口模糊或产品验收',
      },
      null,
      2,
    ),
  );
  console.log(JSON.stringify({ passed: checks.length, checks }));
  await send('Browser.close').catch(() => {});
} finally {
  ws?.close();
  browser.kill();
  await new Promise((resolve) => server.close(resolve));
}

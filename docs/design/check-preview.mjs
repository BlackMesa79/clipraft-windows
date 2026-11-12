// 离屏渲染 + 合成 IPC 数据；不操作用户桌面，不代表原生粘贴验收。
import { createServer } from 'node:http';
import { spawn } from 'node:child_process';
import { readFile, writeFile, mkdir } from 'node:fs/promises';
import path from 'node:path';
import assert from 'node:assert/strict';

const root = process.cwd();
const out = path.join(root, '.test-data/design-review-v2');
const profile = path.join(out, 'profile-' + Date.now());
await mkdir(profile, { recursive: true });
const server = createServer(async (req, res) => {
  try {
    const url = new URL(req.url, 'http://127.0.0.1');
    const file = path.join(root, 'docs/design', url.pathname === '/' ? 'index.html' : url.pathname);
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
  await send('Emulation.setDeviceMetricsOverride',{width:1280,height:940,deviceScaleFactor:1,mobile:false});
 const checks=[];
 for(const theme of ['a','b'])for(const mode of ['list','cards','compact']){
  const layout=mode==='compact'?'cards':mode;
  await send('Page.navigate',{url:origin+'/index-v2.html?theme='+theme+'&layout='+layout+'&compact='+(mode==='compact')});
  await until(()=>evaluate('!!document.querySelector(".wheel-card.center")'),'未就绪');await evaluate('document.fonts.ready');await pause(180);
  assert(await evaluate('document.documentElement.scrollWidth<=1280'));
  assert.equal(await evaluate('document.querySelectorAll(".record").length'),8);
  assert.equal(await evaluate('getComputedStyle(document.querySelector(".preview")).display === "none"'),mode==='compact');
  if(mode==='cards')await evaluate('document.querySelectorAll(".record")[1].click()');
  const shot=await send('Page.captureScreenshot',{format:'jpeg',quality:70});const bytes=Buffer.from(shot.data,'base64');assert(bytes.length<300*1024);
  await writeFile(path.join(root,'docs/design',`v2-${theme}-${mode}.jpg`),bytes);
  if(mode==='compact'){
   const clip=await evaluate('(()=>{const r=document.querySelector(".app").getBoundingClientRect();return {x:r.x,y:r.y,width:r.width,height:r.height,scale:1}})()');
   const detail=await send('Page.captureScreenshot',{format:'jpeg',quality:80,clip});
   await writeFile(path.join(root,'docs/design',`v2-${theme}-compact-detail.jpg`),Buffer.from(detail.data,'base64'));
   assert.equal(await evaluate('document.querySelector(".record").getBoundingClientRect().height'),54);
   assert.equal(await evaluate('document.querySelector(".type-icon").getBoundingClientRect().height'),38);
   await evaluate('document.getElementById("scopeChoice").value="work";document.getElementById("scopeChoice").dispatchEvent(new Event("change"));[...document.querySelectorAll("[data-filter]")].find(b=>b.dataset.filter==="files").click()');
   assert.equal(await evaluate('document.querySelectorAll(".record").length'),2);
   assert.equal(await evaluate('document.getElementById("compactCount").textContent'),'2 条记录');
   assert(await evaluate('![...document.getElementById("scopeChoice").options].some(o=>o.value==="favorite")'));
   await evaluate('document.getElementById("favoriteFilter").click()');
   assert.equal(await evaluate('document.querySelectorAll(".record").length'),0);
   await evaluate('document.getElementById("favoriteFilter").click()');

   await evaluate('document.getElementById("scopeChoice").value="all";document.getElementById("scopeChoice").dispatchEvent(new Event("change"));[...document.querySelectorAll("[data-filter]")].find(b=>b.dataset.filter==="all").click()');

   assert(await evaluate('document.querySelector(".app").getBoundingClientRect().width<=390'));
   assert(await evaluate('getComputedStyle(document.querySelector(".records")).display === "block"'));
   await evaluate('document.getElementById("favoriteFilter").click()');
   assert.equal(await evaluate('document.querySelectorAll(".record").length'),2);
   assert.equal(await evaluate('document.getElementById("favoriteFilter").getAttribute("aria-pressed")'),'true');
   await evaluate('[...document.querySelectorAll("[data-filter]")].find(b=>b.dataset.filter==="image").click()');
   assert.equal(await evaluate('document.querySelectorAll(".record").length'),1);
   await evaluate('document.getElementById("favoriteFilter").click();[...document.querySelectorAll("[data-filter]")].find(b=>b.dataset.filter==="all").click()');
   await evaluate('document.getElementById("compactChoice").click()');
   assert.equal(await evaluate('document.body.dataset.layout'),'cards');
   assert.equal(await evaluate('document.body.dataset.compact'),'false');
  }
  await evaluate('document.querySelectorAll(".record")[2].click();document.getElementById("themeChoice").value=document.body.dataset.theme==="a"?"b":"a";document.getElementById("themeChoice").dispatchEvent(new Event("change"))');
  assert.equal(await evaluate('document.querySelector(".record.selected").dataset.row'),'2');
  assert.equal(await evaluate('document.body.dataset.layout'),layout);
  checks.push(theme+' '+mode+'：渲染、布局/主题独立、选择保持、紧凑恢复通过');
 }
 await send('Emulation.setDeviceMetricsOverride',{width:680,height:800,deviceScaleFactor:1,mobile:false});await pause(100);assert(await evaluate('document.documentElement.scrollWidth<=680'));
 await writeFile(path.join(out,'results.json'),JSON.stringify({checks},null,2));console.log(JSON.stringify({checks}));
}finally{ws?.close();browser.kill();await new Promise(resolve=>server.close(resolve));}

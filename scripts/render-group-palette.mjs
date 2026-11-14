import { writeFile } from 'node:fs/promises';
import { groupColorStyle } from '../src/groupColors.ts';
const names=['日常','项目','设计','资料','笔记','灵感','奶娃','色图'];
const rows=(dark)=>names.map((name,i)=>{
 const style=groupColorStyle(i+1),mode=dark?'dark':'light',ink=style['--group-ink-'+mode],fill=style['--group-fill-'+mode];
 return `<g transform="translate(0 ${i*60})"><rect width="524" height="50" rx="7" fill="${dark?'#293442':'#f6f8f9'}"/><rect x="12" y="10" width="122" height="30" rx="5" fill="${fill}"/><circle cx="26" cy="25" r="3" fill="${ink}"/><text x="38" y="30" fill="${ink}" font-size="14">${name}</text><text x="154" y="30" fill="${dark?'#e7edf6':'#293a43'}" font-size="13">记录中的分组标签</text><rect x="320" y="13" width="76" height="24" rx="4" fill="${fill}"/><text x="334" y="30" fill="${ink}" font-size="12">${name}</text><text x="418" y="30" fill="${dark?'#9aabba':'#7b8b93'}" font-size="11">ID ${i+1}</text></g>`;
}).join('');
const svg=`<svg xmlns="http://www.w3.org/2000/svg" width="1200" height="720" viewBox="0 0 1200 720"><style>text{font-family:'Segoe UI','Microsoft YaHei',sans-serif}</style><rect width="1200" height="720" fill="#edf1f3"/><text x="32" y="46" font-size="26" font-weight="600" fill="#293a43">分组自动配色 · 低饱和度建议</text><text x="32" y="76" font-size="14" fill="#60757f">同组共享色相，浅深主题分别调整明度；按 ID 固定，改名和重启保持一致。</text><rect x="24" y="100" width="564" height="558" rx="12" fill="#fff"/><rect x="612" y="100" width="564" height="558" rx="12" fill="#242c38"/><text x="44" y="134" font-size="18" fill="#293a43">浅色 · 前景饱和度 32%</text><text x="632" y="134" font-size="18" fill="#e7edf6">深色 · 前景饱和度 38%</text><g transform="translate(44 154)">${rows(false)}</g><g transform="translate(632 154)">${rows(true)}</g><text x="32" y="689" font-size="13" fill="#60757f">标签底色饱和度：浅色 26% / 深色 24% · 该色版直接使用当前 groupColorStyle 生成。</text></svg>`;
await writeFile('docs/design/分组低饱和色版.svg',svg);
await writeFile('docs/design/分组低饱和色版.html',`<!doctype html><html lang="zh-CN"><meta charset="utf-8"><title>分组低饱和色版</title><style>body{margin:0;background:#edf1f3}svg{display:block;max-width:100%;height:auto}</style>${svg}</html>`);
console.log('已生成 HTML / SVG 色版');

if(process.argv.includes('--render')) {
  const { spawnSync } = await import('node:child_process');
  const { resolve } = await import('node:path');
  const { pathToFileURL } = await import('node:url');
  const { mkdir, stat } = await import('node:fs/promises');
  const profile=resolve('.tools/palette-profile-'+Date.now());await mkdir(profile,{recursive:true});
  const target=resolve('docs/design/分组低饱和色版.png');
  const result=spawnSync('C:/Program Files (x86)/Microsoft/Edge/Application/msedge.exe',['--headless=new','--disable-gpu','--no-first-run','--no-default-browser-check','--hide-scrollbars','--user-data-dir='+profile,'--screenshot='+target,'--window-size=1200,720',pathToFileURL(resolve('docs/design/分组低饱和色版.html')).href],{windowsHide:true,stdio:'ignore',timeout:30000});
  if(result.error||result.status!==0)throw result.error??Error('色版渲染失败');
  const {size}=await stat(target);if(size>300*1024)throw Error('色版超过预览大小限制');
  console.log('已渲染1200×720 PNG色版，'+size+'字节');
}

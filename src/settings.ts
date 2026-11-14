export interface Settings {
  theme: 'light' | 'dark'; layout: 'list' | 'cards'; compact: boolean; hide_on_blur: boolean; select_first_on_show: boolean; main_position: 'remember' | 'mouse';
  autostart: boolean; pause_on_start: boolean;
  max_items: number; max_bytes: number; max_days: number;
  ignored_apps: string[]; sensitive_detection: boolean; sensitive_high_entropy: boolean; sensitive_ttl_minutes: number;
  main_hotkey: string; quick_hotkey: string; quick_visible_count: number;
}
export interface HotkeyOption { id: string; label: string; hold_label: string }
export interface HotkeyFinding { status:'owned'|'available'|'conflict'|'unavailable'; message:string }
export interface HotkeyReport { main:HotkeyFinding; quick:HotkeyFinding }
export interface SettingOptions {
  defaults: Settings;
  ranges: Record<string, { min: number; max: number; step: number }>;
  main_hotkeys: HotkeyOption[]; quick_hotkeys: HotkeyOption[];
  bytes_per_mb: number; ignored_apps_max: number;
}
export function tightened(next: Settings, old: Settings): boolean {
  return next.max_items < old.max_items || next.max_bytes < old.max_bytes ||
    (next.max_days > 0 && (old.max_days === 0 || next.max_days < old.max_days));
}

export function applyTheme(settings?: Settings) { document.documentElement.dataset.theme = settings?.theme ?? "light"; }

const presetShortcuts: Record<string,[number,number]>={alt_v:[4,86],ctrl_alt_v:[5,86],win_alt_v:[12,86],ctrl_backtick:[1,192],ctrl_shift_v:[3,86],ctrl_alt_shift_v:[7,86],alt_shift_v:[6,86]};
export function shortcutParts(id?:string):[number,number]|undefined {
  if (!id) return;
  if (presetShortcuts[id]) return presetShortcuts[id];
  const m=/^custom:(\d+):(\d+)$/.exec(id);if(m)return [Number(m[1]),Number(m[2])];
}
export function shortcutLabel(id?:string,modifiersOnly=false):string {
  const parts=shortcutParts(id);if(!parts)return '';
  const [mods,key]=parts;const names=[...[[8,'Win'],[1,'Ctrl'],[4,'Alt'],[2,'Shift']] as const].filter(([bit])=>!!(mods&bit)).map(([,name])=>String(name));
  if(!modifiersOnly) names.push(key>=48&&key<=90?String.fromCharCode(key):key>=112&&key<=135?`F${key-111}`:({32:'Space',192:'`',189:'-',187:'=',186:';',188:',',190:'.',191:'/',219:'[',221:']',220:'\\',222:"'"} as Record<number,string>)[key]??`VK${key}`);
  return names.join(' + ');
}

export function shortcutError(id:string):string {
  const parts=shortcutParts(id);if(!parts)return '快捷键格式无效';
  const [mods,key]=parts;
  if(!mods || (mods&~15)!==0 || !(mods&13))return '快捷键至少包含 Ctrl、Alt 或 Win';
  if(!((key>=48&&key<=57)||(key>=65&&key<=90)||(key>=112&&key<=135)||key===32||(key>=186&&key<=192)||(key>=219&&key<=222)))return '请使用字母、数字、F1–F24、空格或常用标点作为触发键';
  if((mods===1&&[65,67,86,88,89,90].includes(key))||((mods&8)!==0&&key===76)||key===123)return '此组合保留给系统或基础编辑操作，请换一个快捷键';
  return '';
}

// 录制使用物理键位，避免 Shift、输入法或不同字符大小写改变热键。
export function shortcutFromKeyboard(event:KeyboardEvent):string|undefined {
  if (/^(Control|Shift|Alt|Meta)(Left|Right)$/.test(event.code)) return;
  const code=event.code;
  const oem:Record<string,number>={Space:32,Semicolon:186,Equal:187,Comma:188,Minus:189,Period:190,Slash:191,Backquote:192,BracketLeft:219,Backslash:220,BracketRight:221,Quote:222};
  const key=/^Key[A-Z]$/.test(code)?code.charCodeAt(3):/^Digit[0-9]$/.test(code)?code.charCodeAt(5):/^F([1-9]|1[0-9]|2[0-4])$/.test(code)?111+Number(code.slice(1)):oem[code]??0;
  const mods=Number(event.ctrlKey)|(Number(event.shiftKey)<<1)|(Number(event.altKey)<<2)|(Number(event.metaKey)<<3);
  return `custom:${mods}:${key}`;
}

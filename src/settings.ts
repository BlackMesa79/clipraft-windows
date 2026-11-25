export type ColorScheme = 'light' | 'dark';

export interface Settings {
  skin: 'clipraft' | 'fluent';
  theme: ColorScheme | 'system';
  auto_check_updates: boolean;
  backdrop_strength: number;
  layout: 'list' | 'cards';
  compact: boolean;
  hide_on_blur: boolean;
  select_first_on_show: boolean;
  content_ignore_rules: ContentIgnoreRule[];
  merge_paste_separator: 'newline' | 'blank_line' | 'space' | 'tab' | 'comma';
  main_position: 'remember' | 'mouse';
  autostart: boolean;
  pause_on_start: boolean;
  max_items: number;
  max_bytes: number;
  max_days: number;
  ignored_apps: string[];
  sensitive_detection: boolean;
  sensitive_high_entropy: boolean;
  sensitive_ttl_minutes: number;
  main_hotkey: string;
  quick_hotkey: string;
  quick_visible_count: number;
}
export interface ContentIgnoreRule {
  mode: 'keyword' | 'regex';
  value: string;
}
export interface HotkeyOption {
  id: string;
  label: string;
  hold_label: string;
}
export interface HotkeyFinding {
  status: 'owned' | 'available' | 'conflict' | 'unavailable';
  message: string;
}
export interface HotkeyReport {
  main: HotkeyFinding;
  quick: HotkeyFinding;
}
export interface SettingOptions {
  defaults: Settings;
  ranges: Record<string, { min: number; max: number; step: number }>;
  main_hotkeys: HotkeyOption[];
  quick_hotkeys: HotkeyOption[];
  bytes_per_mb: number;
  ignored_apps_max: number;
  content_ignore_rules_max: number;
  content_ignore_rule_length_max: number;
}
export function tightened(next: Settings, old: Settings): boolean {
  return (
    next.max_items < old.max_items ||
    next.max_bytes < old.max_bytes ||
    (next.max_days > 0 && (old.max_days === 0 || next.max_days < old.max_days))
  );
}

const MAX_BACKDROP_TRANSPARENCY = 36;

export function applyTheme(settings: Settings | undefined, colorScheme: ColorScheme) {
  document.documentElement.dataset.skin = settings?.skin ?? 'clipraft';
  document.documentElement.dataset.theme = colorScheme;
  const backdropOpacity = ((settings?.backdrop_strength ?? 22) * MAX_BACKDROP_TRANSPARENCY) / 100;
  document.documentElement.style.setProperty('--fluent-backdrop-opacity', `${backdropOpacity}%`);
}

const presetShortcuts: Record<string, [number, number]> = {
  alt_v: [4, 86],
  ctrl_alt_v: [5, 86],
  win_alt_v: [12, 86],
  ctrl_backtick: [1, 192],
  ctrl_shift_v: [3, 86],
  ctrl_alt_shift_v: [7, 86],
  alt_shift_v: [6, 86],
};
export function shortcutParts(id?: string): [number, number] | undefined {
  if (!id) return;
  if (presetShortcuts[id]) return presetShortcuts[id];
  const m = /^custom:(\d+):(\d+)$/.exec(id);
  if (m) return [Number(m[1]), Number(m[2])];
}
const modifierNames = [
  [8, 'Win'],
  [1, 'Ctrl'],
  [4, 'Alt'],
  [2, 'Shift'],
] as const;
const keyLabels: Readonly<Record<number, string>> = {
  32: 'Space',
  192: '`',
  189: '-',
  187: '=',
  186: ';',
  188: ',',
  190: '.',
  191: '/',
  219: '[',
  221: ']',
  220: String.fromCharCode(92),
  222: "'",
};
const triggerRanges = [
  [48, 57],
  [65, 90],
  [112, 135],
  [32, 32],
  [186, 192],
  [219, 222],
] as const;

function modifierLabels(modifiers: number): string[] {
  return modifierNames.filter(([bit]) => !!(modifiers & bit)).map(([, name]) => name);
}

function keyLabel(key: number): string {
  if (key >= 48 && key <= 90) return String.fromCharCode(key);
  if (key >= 112 && key <= 135) return `F${key - 111}`;
  return keyLabels[key] ?? `VK${key}`;
}

export function shortcutModifiersLabel(id?: string): string {
  const parts = shortcutParts(id);
  return parts ? modifierLabels(parts[0]).join(' + ') : '';
}

export function shortcutLabel(id?: string): string {
  const parts = shortcutParts(id);
  if (!parts) return '';
  return [...modifierLabels(parts[0]), keyLabel(parts[1])].join(' + ');
}

function reservedShortcut(modifiers: number, key: number): boolean {
  return (
    (modifiers === 1 && [65, 67, 86, 88, 89, 90].includes(key)) ||
    ((modifiers & 8) !== 0 && key === 76) ||
    key === 123
  );
}

export function shortcutError(id: string): string {
  const parts = shortcutParts(id);
  if (!parts) return '快捷键格式无效';
  const [mods, key] = parts;
  if (!mods || (mods & ~15) !== 0 || !(mods & 13)) return '快捷键至少包含 Ctrl、Alt 或 Win';
  if (!triggerRanges.some(([min, max]) => key >= min && key <= max))
    return '请使用字母、数字、F1–F24、空格或常用标点作为触发键';
  if (reservedShortcut(mods, key)) return '此组合保留给系统或基础编辑操作，请换一个快捷键';
  return '';
}

const physicalPunctuation: Readonly<Record<string, number>> = {
  Space: 32,
  Semicolon: 186,
  Equal: 187,
  Comma: 188,
  Minus: 189,
  Period: 190,
  Slash: 191,
  Backquote: 192,
  BracketLeft: 219,
  Backslash: 220,
  BracketRight: 221,
  Quote: 222,
};

function physicalKey(code: string): number {
  if (/^Key[A-Z]$/.test(code)) return code.charCodeAt(3);
  if (/^Digit[0-9]$/.test(code)) return code.charCodeAt(5);
  if (/^F([1-9]|1[0-9]|2[0-4])$/.test(code)) return 111 + Number(code.slice(1));
  return physicalPunctuation[code] ?? 0;
}

// 录制使用物理键位，避免 Shift、输入法或不同字符大小写改变热键。
export function shortcutFromKeyboard(event: KeyboardEvent): string | undefined {
  if (/^(Control|Shift|Alt|Meta)(Left|Right)$/.test(event.code)) return;
  const modifiers =
    Number(event.ctrlKey) |
    (Number(event.shiftKey) << 1) |
    (Number(event.altKey) << 2) |
    (Number(event.metaKey) << 3);
  return `custom:${modifiers}:${physicalKey(event.code)}`;
}

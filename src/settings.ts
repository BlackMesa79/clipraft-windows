export interface Settings {
  autostart: boolean; pause_on_start: boolean;
  max_items: number; max_bytes: number; max_days: number;
  ignored_apps: string[]; sensitive_detection: boolean; sensitive_high_entropy: boolean; sensitive_ttl_minutes: number;
  main_hotkey: string; quick_hotkey: string; quick_visible_count: number;
}
export interface HotkeyOption { id: string; label: string; hold_label: string }
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

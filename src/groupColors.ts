// 分组身份由数据库 ID 决定；共享色相保证改名、重排和重启后颜色一致。
export function groupColorStyle(id: number | null | undefined): Record<string, string> {
  if (id == null || !Number.isSafeInteger(id) || id < 1) return {};
  const hue = ((id * 137.508) % 360).toFixed(2);
  return {
    '--group-ink-light': `hsl(${hue} 32% 29%)`,
    '--group-ink-dark': `hsl(${hue} 38% 78%)`,
    '--group-fill-light': `hsl(${hue} 26% 94%)`,
    '--group-fill-dark': `hsl(${hue} 24% 24%)`,
    '--group-dot-light': `hsl(${hue} 34% 41%)`,
    '--group-dot-dark': `hsl(${hue} 40% 70%)`,
  };
}

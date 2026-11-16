export function directoryLabel(path: string): string {
  // Windows 规范化路径保留在迁移协议中，仅调整常见盘符和 UNC 路径的展示。
  const extendedPrefix = '\\\\?\\';
  const uncPrefix = '\\\\?\\UNC\\';
  if (path.startsWith(uncPrefix)) return '\\\\' + path.slice(uncPrefix.length);
  const drivePath = path.slice(extendedPrefix.length);
  if (path.startsWith(extendedPrefix) && /^[a-z]:\\/i.test(drivePath)) return drivePath;
  return path;
}

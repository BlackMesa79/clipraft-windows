export interface Item { id: string; kind: string; summary: string; formats: string[]; thumbnail: string | null; preview?: string | null; placeholder: string | null; files: string[] }
export const kindLabel = (item: Item) => ({ text: '文本', image: '图片', files: '文件', rich: '富文本' }[item.kind] ?? '内容');
export const filename = (p: string) => p.split(/[\\/]/).pop() ?? p;

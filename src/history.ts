export interface Group { id: number; name: string; sort: number; count: number }
export type HistoryView = 'all' | 'favorites' | 'ungrouped' | number;
export interface Item { source_exe?: string | null; source_title?: string | null; pinned: boolean; favorite: boolean; group_id: number | null; id: string; kind: string; summary: string; formats: string[]; thumbnail: string | null; preview?: string | null; placeholder: string | null; files: string[] }
export const kindLabel = (item: Item) => ({ text: '文本', image: '图片', files: '文件', rich: '富文本' }[item.kind] ?? '内容');
export const filename = (p: string) => p.split(/[\\/]/).pop() ?? p;

export type HistoryKind = 'text' | 'image' | 'files' | 'link';
export interface HistoryFilter { query?: string; kind?: HistoryKind | null; favorite_only?: boolean; group_id?: number }

import type { Item } from './history';

// 与后端 panel_state / panel-state 保持一致，只接收后端安全视图。
export interface PanelState {
  items: Item[];
  selected: number;
  visible: boolean;
  error: string | null;
}

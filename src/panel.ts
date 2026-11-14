import type { Item } from './history';
import { shallowRef, type ShallowRef } from 'vue';

// 与后端 panel_state / panel-state 保持一致，只接收后端安全视图。
export interface PanelState {
  items: Item[];
  selected: number;
  visible: boolean;
  error: string | null;
  serial: number;
  session: number;
  position: number;
  total: number;
  loading: boolean;
}

export function createPanelState(): {
  panel: ShallowRef<PanelState>;
  applyPanel: (next: PanelState) => void;
} {
  const panel = shallowRef<PanelState>({
    items: [],
    selected: 0,
    visible: false,
    error: null,
    serial: -1,
    session: 0,
    position: 0,
    total: 0,
    loading: false,
  });
  function applyPanel(next: PanelState): void {
    if (next.serial <= panel.value.serial) return;
    panel.value = next;
  }
  return { panel, applyPanel };
}

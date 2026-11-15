import source from './quick-geometry.json?raw';
import type { PanelState } from './panel';
import type { Item } from './history';

const SLOT_DEPTH = 40;
const MAX_DEPTH_SLOTS = 3;
const SLOT_ROTATION = 18;
const SLOT_SCALE_STEP = 0.06;
const MIN_SLOT_SCALE = 0.68;
const SELECTED_LAYER = 20;

interface WheelGeometry {
  width: number;
  cardHeight: number;
  pitch: number;
  padding: number;
  duration: number;
  maxRotation: number;
}
export const wheelGeometry: WheelGeometry = JSON.parse(source);
export const wheelHeight = (count: number): number =>
  wheelGeometry.cardHeight +
  wheelGeometry.padding +
  Math.floor(count / 2) * wheelGeometry.pitch * 2;

export interface WheelSlot {
  key: string;
  item: Item;
  distance: number;
  position: number;
}

export function wheelSlots(panel: PanelState): WheelSlot[] {
  if (panel.loading || !panel.visible) return [];
  return panel.items.map((item, index) => ({
    key: `${panel.session}:${item.id}`,
    item,
    distance: index - panel.selected,
    position: ((panel.position + panel.total + index - panel.selected) % panel.total) + 1,
  }));
}

export function slotStyle(distance: number): Record<string, string | number> {
  const depth = Math.abs(distance);
  return {
    transform: `translateY(${distance * wheelGeometry.pitch}px) translateZ(${-Math.min(depth, MAX_DEPTH_SLOTS) * SLOT_DEPTH}px) rotateX(${Math.max(-wheelGeometry.maxRotation, Math.min(wheelGeometry.maxRotation, -distance * SLOT_ROTATION))}deg) scale(${Math.max(MIN_SLOT_SCALE, 1 - depth * SLOT_SCALE_STEP)})`,
    zIndex: SELECTED_LAYER - depth,
  };
}

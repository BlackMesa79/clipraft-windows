import { nextTick, ref, type Ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import type { Item } from './history';

interface PreviewContext {
  active(): Item | undefined;
  compact(): boolean;
  settingsOpen(): boolean;
  listRoot(): HTMLElement | null;
  recordElement(id: string | undefined): HTMLElement | undefined;
  reportError(message: string): void;
}
interface CompactPreviewController {
  floatingActive: Ref<boolean>;
  syncFloatingPreview(): void;
  suspendFloatingPreview(): void;
  schedulePreviewGeometry(): void;
  cancelPreviewGeometry(): void;
  invalidateFloatingPreview(): void;
}
interface PreviewAnchor {
  id: string;
  top: number;
  bottom: number;
}

// 内容去重、请求代号和几何缓存属于同一窗口，监听注册仍由调用方负责。
// 工厂统一持有实例状态；行数包含独立闭包，拆开会分散状态所有权。
// eslint-disable-next-line max-lines-per-function
export function createCompactPreview(context: PreviewContext): CompactPreviewController {
  const floatingActive = ref(true);
  let geometryFrame = 0,
    lastGeometry = '',
    floatingRequest = 0;
  let lastFloatingId: string | null | undefined;
  function previewAnchor(): PreviewAnchor | null {
    const list = context.listRoot();
    const row = context.recordElement(context.active()?.id);
    const box = row?.getBoundingClientRect(),
      viewport = list?.getBoundingClientRect();
    if (!box || !viewport) return null;
    const top = Math.max(box.top, viewport.top);
    const bottom = Math.min(box.bottom, viewport.bottom);
    return context.compact() && floatingActive.value && !context.settingsOpen() && top < bottom
      ? { id: context.active()!.id, top, bottom }
      : null;
  }
  function schedulePreviewGeometry(): void {
    if (geometryFrame) return;
    geometryFrame = requestAnimationFrame(() => {
      geometryFrame = 0;
      const anchor = previewAnchor();
      const signature = JSON.stringify(anchor);
      if (signature === lastGeometry) return;
      lastGeometry = signature;
      void invoke('position_compact_preview', { anchor }).catch(() => {
        lastGeometry = '';
      });
    });
  }
  function syncFloatingPreview(): void {
    void nextTick(schedulePreviewGeometry);
    const id =
      context.compact() && floatingActive.value && !context.settingsOpen()
        ? (context.active()?.id ?? null)
        : null;
    if (id === lastFloatingId) return;
    lastFloatingId = id;
    const request = ++floatingRequest;
    void invoke('sync_compact_preview', { id }).catch((error) => {
      if (request === floatingRequest && id) {
        lastFloatingId = undefined;
        context.reportError('浮动预览失败：' + String(error));
      }
    });
  }
  function suspendFloatingPreview(): void {
    floatingActive.value = false;
    syncFloatingPreview();
  }
  function invalidateFloatingPreview(): void {
    lastFloatingId = undefined;
  }
  function cancelPreviewGeometry(): void {
    cancelAnimationFrame(geometryFrame);
  }
  return {
    floatingActive,
    syncFloatingPreview,
    suspendFloatingPreview,
    schedulePreviewGeometry,
    cancelPreviewGeometry,
    invalidateFloatingPreview,
  };
}

import { ref, type Ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import type { Item } from './history';
import { createDetailImage } from './detailImage';

interface Detail {
  image?: number[];
  text: string;
  truncated: boolean;
  files: { path: string; exists: boolean | null }[];
}
interface DetailContext {
  active(): Item | undefined;
  compact(): boolean;
  resetScroll(): void;
}
interface ItemDetailController {
  detail: Ref<Detail | null>;
  detailError: Ref<string>;
  detailLoading: Ref<boolean>;
  revealed: Ref<boolean>;
  imageSource: Ref<string>;
  invalidateDetail(): void;
  conceal(): void;
  revealDetail(): Promise<void>;
  loadDetail(): Promise<void>;
}

// 调用方保留列表/窗口生命周期；代号只在本记录详情控制器内递增。
// 工厂统一持有实例状态；行数包含独立闭包，拆开会分散状态所有权。
// eslint-disable-next-line max-lines-per-function
export function createItemDetail(context: DetailContext): ItemDetailController {
  const detail = ref<Detail | null>(null),
    detailError = ref(''),
    detailLoading = ref(false);
  const revealed = ref(false);
  const { imageSource, replaceImage } = createDetailImage();
  function setDetail(result: Detail | null): void {
    replaceImage(result?.image);
    detail.value = result;
  }
  let generation = 0;
  function invalidateDetail(): void {
    ++generation;
  }
  function conceal(): void {
    invalidateDetail();
    revealed.value = false;
    setDetail(null);
    detailLoading.value = false;
  }
  async function revealDetail(): Promise<void> {
    if (!context.active()?.sensitive || detailLoading.value) return;
    const id = context.active()!.id,
      request = ++generation;
    detailLoading.value = true;
    detailError.value = '';
    try {
      const result = await invoke<Detail>('reveal_item', { id });
      if (request === generation && context.active()?.id === id) {
        setDetail(result);
        revealed.value = true;
      }
    } catch {
      if (request === generation) detailError.value = '显示原文失败';
    } finally {
      if (request === generation) detailLoading.value = false;
    }
  }
  async function loadDetail(): Promise<void> {
    if (context.compact()) {
      conceal();
      return;
    }
    const id = context.active()?.id,
      request = ++generation;
    context.resetScroll();
    revealed.value = false;
    setDetail(null);
    detailError.value = '';
    detailLoading.value = !!id;
    if (!id || (context.active()?.kind === 'image' && !context.active()?.sensitive)) {
      detailLoading.value = false;
      return;
    }
    await fetchDetail(id, request);
  }
  async function fetchDetail(id: string, request: number): Promise<void> {
    try {
      const result = await invoke<Detail>('item_detail', { id });
      if (request === generation) setDetail(result);
    } catch (error) {
      if (request === generation) detailError.value = String(error);
    } finally {
      if (request === generation) detailLoading.value = false;
    }
  }
  return {
    detail,
    detailError,
    detailLoading,
    revealed,
    imageSource,
    invalidateDetail,
    conceal,
    revealDetail,
    loadDetail,
  };
}

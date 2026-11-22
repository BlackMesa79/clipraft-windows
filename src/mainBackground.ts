import { invoke } from '@tauri-apps/api/core';
import { onMounted, ref, shallowReactive } from 'vue';

export const backgroundLayouts = ['regular', 'compact'] as const;
export type BackgroundLayout = (typeof backgroundLayouts)[number];

export interface BackgroundOptions {
  fit: 'cover' | 'contain';
  position: 'center' | 'top' | 'bottom' | 'left' | 'right';
  scrim: number;
}

export interface MainBackground {
  jpeg: string;
  options: BackgroundOptions;
}

interface BackgroundEntry {
  asset: MainBackground | null;
  error: string;
}

export type Backgrounds = Record<BackgroundLayout, BackgroundEntry>;

export const scrimRange = { min: 65, max: 95 } as const;

const imageSource = (jpeg: string): string => `data:image/jpeg;base64,${jpeg}`;

export function backgroundStyle(jpeg: string, options: BackgroundOptions): Record<string, string> {
  return {
    '--main-background-image': `url("${imageSource(jpeg)}")`,
    '--main-background-fit': options.fit,
    '--main-background-position': options.position,
    '--main-background-scrim': `${options.scrim}%`,
  };
}

export function useMainBackground() {
  const backgrounds = shallowReactive<Backgrounds>({
    regular: { asset: null, error: '' },
    compact: { asset: null, error: '' },
  });
  const backgroundLoading = ref(true);

  function setBackground(layout: BackgroundLayout, asset: MainBackground | null): void {
    backgrounds[layout] = { asset, error: '' };
  }

  async function load(layout: BackgroundLayout): Promise<void> {
    try {
      const saved = await invoke<MainBackground | null>('get_main_background', { layout });
      if (!saved) return;
      const image = new Image();
      image.src = imageSource(saved.jpeg);
      await image.decode();
      setBackground(layout, saved);
    } catch (error) {
      backgrounds[layout] = { asset: null, error: `无法读取已保存的背景图片：${String(error)}` };
    }
  }

  onMounted(async () => {
    await Promise.all(backgroundLayouts.map(load));
    backgroundLoading.value = false;
  });

  return { backgrounds, backgroundLoading, setBackground };
}

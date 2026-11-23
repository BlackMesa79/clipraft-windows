import { computed, onScopeDispose, ref, watch, type Ref } from 'vue';
import { applyTheme, type ColorScheme, type Settings } from './settings';

export function useAppearance(settings: Ref<Settings | undefined>) {
  const systemDark = window.matchMedia('(prefers-color-scheme: dark)');
  const prefersDark = ref(systemDark.matches);
  const update = (event: MediaQueryListEvent) => {
    prefersDark.value = event.matches;
  };
  systemDark.addEventListener('change', update);
  onScopeDispose(() => systemDark.removeEventListener('change', update));

  const colorScheme = computed<ColorScheme>(() => {
    const theme = settings.value?.theme ?? 'light';
    if (theme !== 'system') return theme;
    return prefersDark.value ? 'dark' : 'light';
  });
  watch([settings, colorScheme], ([current, scheme]) => applyTheme(current, scheme), {
    immediate: true,
  });
  return colorScheme;
}

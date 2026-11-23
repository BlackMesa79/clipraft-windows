import { getVersion } from '@tauri-apps/api/app';
import { invoke } from '@tauri-apps/api/core';
import { onScopeDispose, readonly, ref, watch } from 'vue';
import { releaseSummary } from './releaseNotes';

const RELEASE_API = 'https://api.github.com/repos/BlackMesa79/clipraft-windows/releases/latest';
const CHECK_TIMEOUT_MS = 10_000;

interface VersionNotes {
  version: string;
  notes: string;
}

interface ReleaseHistory {
  releases: { tag: string; notes: string }[];
  complete: boolean;
}

type UpdateState =
  | { kind: 'idle' | 'checking' | 'current' | 'empty' }
  | { kind: 'failed'; message: string }
  | {
      kind: 'available';
      tag: string;
      version: string;
      releases: VersionNotes[];
      historyIncomplete: boolean;
    };

function versionParts(version: string): number[] | undefined {
  const parts = /^v?(0|[1-9]\d*)\.(0|[1-9]\d*)\.(0|[1-9]\d*)$/.exec(version)?.slice(1).map(Number);
  return parts?.every(Number.isSafeInteger) ? parts : undefined;
}

function versionOrder(left: number[], right: number[]): number {
  const difference = left.findIndex((part, index) => part !== right[index]);
  return difference < 0 ? 0 : left[difference] - right[difference];
}

function compareRelease(tag: unknown, current: number[], notes = ''): UpdateState {
  const versionTag = typeof tag === 'string' ? tag : '';
  const latest = versionParts(versionTag);
  if (!latest) return { kind: 'failed', message: '无法识别 GitHub 返回的版本号。' };
  if (versionOrder(latest, current) <= 0) return { kind: 'current' };
  const version = latest.join('.');
  return {
    kind: 'available',
    tag: versionTag,
    version,
    releases: [{ version, notes }],
    historyIncomplete: false,
  };
}

function releaseResult(release: unknown, current: number[]): UpdateState {
  if (release === null || typeof release !== 'object') {
    return { kind: 'failed', message: '无法识别 GitHub 返回的版本信息。' };
  }
  const { tag_name, draft, prerelease, body } = release as {
    tag_name?: unknown;
    draft?: unknown;
    prerelease?: unknown;
    body?: unknown;
  };
  if (draft === true || prerelease === true) return { kind: 'empty' };
  const notes = typeof body === 'string' ? releaseSummary(body) : '';
  return compareRelease(tag_name, current, notes);
}

function historyBetween(
  history: ReleaseHistory,
  current: number[],
  latest: number[],
): VersionNotes[] {
  const versions = new Map<string, { parts: number[]; notes: string }>();
  for (const release of history.releases) {
    const parts = versionParts(release.tag);
    if (!parts || versionOrder(parts, current) <= 0 || versionOrder(parts, latest) > 0) continue;
    const key = parts.join('.');
    if (!versions.get(key)?.notes) versions.set(key, { parts, notes: release.notes });
  }
  return Array.from(versions.entries())
    .sort(([, left], [, right]) => versionOrder(right.parts, left.parts))
    .map(([version, release]) => ({ version, notes: release.notes }));
}

async function withReleaseHistory(result: UpdateState, current: number[]): Promise<UpdateState> {
  if (result.kind !== 'available') return result;
  const history = await invoke<ReleaseHistory>('list_release_notes').catch(() => ({
    releases: [],
    complete: false,
  }));
  const latest = versionParts(result.tag);
  if (!latest) return result;
  const historyNotes = historyBetween(history, current, latest);
  const listedNotes = historyNotes.find((release) => release.version === result.version)?.notes;
  const notes =
    result.releases[0].notes ||
    listedNotes ||
    (await invoke<string>('get_release_update_notes', { tag: result.tag }).catch(() => ''));
  const intermediate = historyNotes.filter((release) => release.version !== result.version);
  return {
    ...result,
    releases: [{ version: result.version, notes }, ...intermediate],
    historyIncomplete: !history.complete,
  };
}

async function fetchReleaseApi(
  current: number[],
  signal: AbortSignal,
): Promise<UpdateState | undefined> {
  const response = await fetch(RELEASE_API, {
    headers: { Accept: 'application/vnd.github+json' },
    credentials: 'omit',
    referrerPolicy: 'no-referrer',
    redirect: 'error',
    cache: 'no-store',
    signal,
  });
  if (response.status === 404) return { kind: 'empty' };
  if ([403, 429].includes(response.status) || response.status >= 500) return;
  if (!response.ok) return { kind: 'failed', message: 'GitHub 暂时无法提供版本信息，请稍后重试。' };
  const release: unknown = await response.json();
  return releaseResult(release, current);
}

async function fetchLatest(current: number[], controller: AbortController): Promise<UpdateState> {
  const { signal } = controller;
  const timeout = setTimeout(() => controller.abort(), CHECK_TIMEOUT_MS);
  const apiResult = await fetchReleaseApi(current, signal)
    .catch((error: unknown) => {
      if (signal.aborted) throw error;
      return undefined;
    })
    .finally(() => clearTimeout(timeout));
  if (apiResult) return withReleaseHistory(apiResult, current);
  if (signal.aborted) throw signal.reason;

  const tag = await invoke<string | null>('get_latest_release_tag');
  return tag === null
    ? { kind: 'empty' }
    : withReleaseHistory(compareRelease(tag, current), current);
}

export function useUpdates(autoCheckEnabled: () => boolean | undefined) {
  const version = ref('');
  const updateState = ref<UpdateState>({ kind: 'idle' });
  const installed = getVersion().then(
    (value) => (version.value = typeof value === 'string' ? value : ''),
    () => '',
  );
  let activeCheck: AbortController | undefined;
  let startupHandled = false;
  onScopeDispose(() => activeCheck?.abort());

  async function check() {
    if (updateState.value.kind === 'checking') return;
    updateState.value = { kind: 'checking' };
    const controller = new AbortController();
    activeCheck = controller;
    try {
      const current = versionParts(await installed);
      updateState.value = current
        ? await fetchLatest(current, controller)
        : { kind: 'failed', message: '无法读取当前版本。' };
    } catch {
      updateState.value = {
        kind: 'failed',
        message: controller.signal.aborted
          ? '检查更新超时，请重试。'
          : '无法检查更新，请检查网络后重试。',
      };
    } finally {
      activeCheck = undefined;
    }
  }

  watch(
    autoCheckEnabled,
    (enabled) => {
      if (enabled === undefined || startupHandled) return;
      startupHandled = true;
      if (enabled && updateState.value.kind === 'idle') void check();
    },
    { immediate: true },
  );
  return { version: readonly(version), updateState: readonly(updateState), check };
}

export type Updates = ReturnType<typeof useUpdates>;

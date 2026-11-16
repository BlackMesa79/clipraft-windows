import { computed, onMounted, onUnmounted, ref, watch } from 'vue';
import { invoke } from '@tauri-apps/api/core';

type Phase =
  | 'pending'
  | 'preparing'
  | 'snapshotting'
  | 'writing'
  | 'verifying'
  | 'publishing'
  | 'complete'
  | 'cancelled'
  | 'failed';
type Outcome =
  | {
      state: 'succeeded';
      restored_root: string | null;
      replace_current?: boolean;
      cleanup_warning: string | null;
    }
  | { state: 'failed' | 'cancelled'; message: string };
interface Report {
  id: number;
  kind: 'export' | 'restore';
  running: boolean;
  progress: { phase: Phase; completed_bytes: number; total_bytes: number };
  outcome: Outcome | null;
}
const phases: Record<Phase, string> = {
  pending: '等待开始',
  preparing: '正在准备',
  snapshotting: '正在创建快照，期间复制不会记录',
  writing: '正在写入',
  verifying: '正在验证内容',
  publishing: '正在发布，请稍候',
  complete: '已完成',
  cancelled: '已取消',
  failed: '任务失败',
};
function createState() {
  return {
    mode: ref<'export' | 'restore'>('export'),
    restoreTarget: ref<'current' | 'other'>('current'),
    destination: ref(''),
    archive: ref(''),
    dialog: ref<'export' | 'restore' | 'replace' | null>(null),
    restarting: ref(false),
    offeredRestore: 0,
    report: ref<Report | null>(null),
    error: ref(''),
    pollingError: ref(''),
    working: ref(false),
    ready: ref(false),
    disposed: false,
    sequence: 0,
    timer: undefined as ReturnType<typeof setTimeout> | undefined,
  };
}
type State = ReturnType<typeof createState>;
function occupied(state: State): boolean {
  return (
    state.working.value ||
    state.restarting.value ||
    !!state.report.value?.running ||
    !state.ready.value
  );
}
function cancellable(state: State): boolean {
  return (
    !!state.report.value?.running &&
    !['publishing', 'complete', 'failed', 'cancelled'].includes(state.report.value.progress.phase)
  );
}
function derived(state: State) {
  return {
    occupied: computed(() => occupied(state)),
    cancellable: computed(() => cancellable(state)),
    phase: computed(() => (state.report.value ? phases[state.report.value.progress.phase] : '')),
    percent: computed(() => {
      const progress = state.report.value?.progress;
      if (!progress?.total_bytes) return null;
      return Math.min(100, Math.floor((100 * progress.completed_bytes) / progress.total_bytes));
    }),
  };
}
function install(state: State, emitBusy: (active: boolean) => void): void {
  watch(() => occupied(state) || !!state.dialog.value, emitBusy, { immediate: true });
  watch(state.mode, () => {
    state.dialog.value = null;
    state.destination.value = '';
    state.error.value = '';
  });
  onMounted(() => {
    void poll(state);
  });
  onUnmounted(() => {
    state.disposed = true;
    clearTimeout(state.timer);
    state.dialog.value = null;
    emitBusy(false);
  });
}
async function refresh(state: State): Promise<void> {
  const sequence = ++state.sequence;
  try {
    const next = await invoke<Report | null>('archive_task_status');
    if (state.disposed || sequence !== state.sequence) return;
    const wasRunning = state.report.value?.running;
    state.report.value = next ?? null;
    if (wasRunning) offerReplacement(state);
    state.ready.value = true;
    state.pollingError.value = '';
  } catch (reason) {
    if (state.disposed || sequence !== state.sequence) return;
    state.pollingError.value = String(reason);
    state.ready.value = false;
  }
}
async function poll(state: State): Promise<void> {
  await refresh(state);
  if (!state.disposed)
    state.timer = setTimeout(() => {
      void poll(state);
    }, 750);
}
async function choose(state: State, field: 'archive' | 'destination'): Promise<void> {
  if (occupied(state)) return;
  state.working.value = true;
  state.error.value = '';
  try {
    const selected = await invoke<string | null>('choose_archive_path', {
      selection: field === 'archive' ? 'archive' : 'folder',
    });
    if (!selected || state.disposed) return;
    if (field === 'archive') {
      state.archive.value = selected;
      return;
    }
    const name = state.mode.value === 'export' ? 'history.clipraft' : 'ClipRaft-Restored';
    state.destination.value = selected.replace(/[\\/]$/, '') + '\\' + name;
  } catch (reason) {
    if (!state.disposed) state.error.value = String(reason);
  } finally {
    state.working.value = false;
  }
}
function openPassword(state: State): void {
  if (occupied(state)) return;
  state.error.value = '';
  if (
    ((state.mode.value === 'export' || state.restoreTarget.value === 'other') &&
      !state.destination.value.trim()) ||
    (state.mode.value === 'restore' && !state.archive.value.trim())
  ) {
    state.error.value = '请先选择或填写路径';
    return;
  }
  state.dialog.value = state.mode.value;
}
function offerReplacement(state: State): void {
  const report = state.report.value;
  if (
    !report ||
    report.running ||
    report.outcome?.state !== 'succeeded' ||
    !report.outcome.restored_root ||
    !report.outcome.replace_current
  )
    return;
  if (state.offeredRestore === report.id || state.dialog.value) return;
  state.offeredRestore = report.id;
  state.dialog.value = 'replace';
}
async function start(state: State, password: string): Promise<void> {
  if (occupied(state)) return;
  state.error.value = '';
  state.dialog.value = null;
  state.working.value = true;
  ++state.sequence;
  try {
    const input: { destination?: string; password: string; archive?: string } = { password };
    if (state.mode.value === 'export' || state.restoreTarget.value === 'other')
      input.destination = state.destination.value.trim();
    if (state.mode.value === 'restore') input.archive = state.archive.value.trim();
    const command =
      state.mode.value === 'export' ? 'start_archive_export' : 'start_archive_restore';
    const id = await invoke<number>(command, { input });
    if (state.disposed) return;
    state.report.value = {
      id,
      kind: state.mode.value,
      running: true,
      progress: { phase: 'pending', completed_bytes: 0, total_bytes: 0 },
      outcome: null,
    };
    await refresh(state);
  } catch (reason) {
    if (!state.disposed) state.error.value = String(reason);
  } finally {
    state.working.value = false;
  }
}
async function cancel(state: State): Promise<void> {
  const current = state.report.value;
  if (!current || !cancellable(state)) return;
  try {
    await invoke('cancel_archive_task', { id: current.id });
    await refresh(state);
  } catch (reason) {
    state.error.value = String(reason);
  }
}
async function replaceCurrent(state: State): Promise<void> {
  const report = state.report.value;
  if (!report || occupied(state)) return;
  state.working.value = true;
  state.error.value = '';
  try {
    await invoke('activate_restored_archive', { id: report.id });
    if (!state.disposed) {
      state.restarting.value = true;
      state.dialog.value = null;
    }
  } catch (reason) {
    if (!state.disposed) state.error.value = String(reason);
  } finally {
    state.working.value = false;
  }
}
export function useArchiveTask(emitBusy: (active: boolean) => void) {
  const state = createState();
  install(state, emitBusy);
  return {
    ...state,
    ...derived(state),
    choose: (field: 'archive' | 'destination') => choose(state, field),
    openPassword: () => openPassword(state),
    start: (password: string) => start(state, password),
    replaceCurrent: () => replaceCurrent(state),
    cancel: () => cancel(state),
  };
}

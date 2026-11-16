import { onUnmounted, ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
type Status =
  { state: 'active' | 'empty' | 'retained' } | { state: 'unrecognized'; reason: string };
interface Candidate {
  root: string;
  status: Status;
}
interface Report {
  candidates: Candidate[];
  truncated: boolean;
}
interface Preview {
  ticket: string;
  root: string;
  files: number;
  bytes: number;
}
function state() {
  return {
    parent: ref(''),
    report: ref<Report | null>(null),
    preview: ref<Preview | null>(null),
    busy: ref(false),
    error: ref(''),
    message: ref(''),
    disposed: false,
  };
}
type State = ReturnType<typeof state>;
async function release(current: State): Promise<void> {
  const ticket = current.preview.value?.ticket;
  current.preview.value = null;
  if (ticket) await invoke('cancel_staging_cleanup', { ticket });
}
async function scan(current: State): Promise<void> {
  await release(current);
  current.report.value = await invoke<Report>('scan_archive_staging', {
    parent: current.parent.value,
  });
}
async function choose(current: State): Promise<void> {
  const parent = await invoke<string | null>('choose_archive_path', { selection: 'folder' });
  if (!parent || current.disposed) return;
  current.parent.value = parent;
  await scan(current);
}
async function prepare(current: State, root: string): Promise<void> {
  await release(current);
  const preview = await invoke<Preview>('preview_staging_cleanup', { root });
  if (current.disposed) {
    await invoke('cancel_staging_cleanup', { ticket: preview.ticket });
    return;
  }
  current.preview.value = preview;
}
async function confirm(current: State): Promise<void> {
  const preview = current.preview.value;
  if (!preview) return;
  await invoke('confirm_staging_cleanup', { ticket: preview.ticket });
  current.preview.value = null;
  current.message.value = '已清理确认清单中的暂存文件。';
  await scan(current);
}
async function run(
  current: State,
  emit: (busy: boolean) => void,
  operation: () => Promise<void>,
): Promise<void> {
  if (current.busy.value) return;
  current.busy.value = true;
  emit(true);
  current.error.value = '';
  current.message.value = '';
  try {
    await operation();
  } catch (error) {
    if (!current.disposed) current.error.value = String(error);
  } finally {
    current.busy.value = false;
    emit(false);
  }
}
export function useStagingCleanup(emit: (busy: boolean) => void) {
  const current = state();
  onUnmounted(() => {
    current.disposed = true;
    void release(current).catch((error) => console.error('释放暂存清理预检失败', error));
  });
  return {
    ...current,
    choose: () => run(current, emit, () => choose(current)),
    scan: () => run(current, emit, () => scan(current)),
    prepare: (root: string) => run(current, emit, () => prepare(current, root)),
    confirm: () => run(current, emit, () => confirm(current)),
    cancel: () => run(current, emit, () => release(current)),
  };
}
export function stagingLabel(status: Status): string {
  return {
    active: '任务运行中或已被预检锁定',
    empty: '空暂存目录',
    retained: '含中断数据，需要预检后确认',
    unrecognized: '无法确认所有权，保留',
  }[status.state];
}

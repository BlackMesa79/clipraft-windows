import { ref, type Ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { shortcutError, shortcutFromKeyboard, type Settings } from './settings';

export type HotkeyField = 'main_hotkey' | 'quick_hotkey';
interface RecordIssue {
  conflict: boolean;
  detail: string;
}
interface RecordedHotkey {
  session: number;
  hotkey: string | null;
}
interface RecordingContext {
  draft: Ref<Settings | undefined>;
  scheduleHotkeyCheck(): void;
}
interface RecordingState {
  context: RecordingContext;
  recordingField: Ref<HotkeyField | null>;
  recordOperations: Ref<number>;
  recordIssues: Ref<Partial<Record<HotkeyField, RecordIssue>>>;
  hotkeyRevisions: Record<HotkeyField, number>;
  recordingTimer?: ReturnType<typeof setTimeout>;
  disposed: boolean;
  recordSession: number;
  recordEpoch: number;
  earlyRecordings: Map<number, string | null>;
}
interface HotkeyRecordingController {
  recordingField: RecordingState['recordingField'];
  recordOperations: RecordingState['recordOperations'];
  recordIssues: RecordingState['recordIssues'];
  clearRecordIssue(field: HotkeyField): void;
  record(field: HotkeyField): Promise<void>;
  stopRecording(): Promise<boolean>;
  blurRecording(): void;
  recordingKeydown(event: KeyboardEvent): void;
  receiveRecording(payload: RecordedHotkey): Promise<void>;
  isDisposed(): boolean;
  markDisposed(): void;
}

// 监听、watch 和卸载顺序由设置页编排；所有会话和修订状态仅属于本实例。
export function createHotkeyRecording(context: RecordingContext): HotkeyRecordingController {
  const state: RecordingState = {
    context,
    recordingField: ref(null),
    recordOperations: ref(0),
    recordIssues: ref({}),
    hotkeyRevisions: { main_hotkey: 0, quick_hotkey: 0 },
    disposed: false,
    recordSession: 0,
    recordEpoch: 0,
    earlyRecordings: new Map(),
  };
  return {
    recordingField: state.recordingField,
    recordOperations: state.recordOperations,
    recordIssues: state.recordIssues,
    clearRecordIssue: (field) => clearRecordIssue(state, field),
    record: (field) => record(state, field),
    stopRecording: () => stopRecording(state),
    blurRecording: () => blurRecording(state),
    recordingKeydown: (event) => recordingKeydown(state, event),
    receiveRecording: (payload) => receiveRecording(state, payload),
    isDisposed: () => state.disposed,
    markDisposed: () => {
      state.disposed = true;
    },
  };
}

function clearRecordIssue(state: RecordingState, field: HotkeyField): void {
  state.hotkeyRevisions[field]++;
  delete state.recordIssues.value[field];
}

async function stopRecording(state: RecordingState): Promise<boolean> {
  const epoch = ++state.recordEpoch;
  const field = state.recordingField.value,
    revision = field ? state.hotkeyRevisions[field] : 0;
  clearTimeout(state.recordingTimer);
  state.recordingField.value = null;
  state.recordSession = 0;
  state.earlyRecordings.clear();
  state.recordOperations.value++;
  try {
    await invoke('set_hotkey_recording', { active: false });
    return true;
  } catch (error) {
    if (
      !state.disposed &&
      epoch === state.recordEpoch &&
      field &&
      revision === state.hotkeyRevisions[field]
    ) {
      state.recordIssues.value[field] = { conflict: false, detail: String(error) };
    }
    return false;
  } finally {
    state.recordOperations.value--;
  }
}

interface RecordingAttempt {
  field: HotkeyField;
  epoch: number;
  revision: number;
}
function recordFailed(state: RecordingState, attempt: RecordingAttempt, error: unknown): void {
  if (state.disposed || attempt.epoch !== state.recordEpoch) return;
  if (attempt.revision === state.hotkeyRevisions[attempt.field]) {
    state.recordIssues.value[attempt.field] = { conflict: false, detail: String(error) };
  }
  state.recordingField.value = null;
  state.recordSession = 0;
}

const RECORD_TIMEOUT_MS = 30_000;
async function record(state: RecordingState, field: HotkeyField): Promise<void> {
  if (state.recordOperations.value || state.disposed) return;
  if (state.recordingField.value) {
    await stopRecording(state);
    return;
  }
  const epoch = ++state.recordEpoch;
  clearRecordIssue(state, field);
  const revision = state.hotkeyRevisions[field];
  state.earlyRecordings.clear();
  state.recordingField.value = field;
  state.recordOperations.value++;
  try {
    const session = await invoke<number>('set_hotkey_recording', { active: true });
    if (state.disposed || epoch !== state.recordEpoch) {
      await invoke('set_hotkey_recording', { active: false });
      return;
    }
    state.recordSession = session;
    const early = state.earlyRecordings.get(session);
    state.earlyRecordings.clear();
    if (early !== undefined) {
      void finishRecording(state, early);
      return;
    }
    state.recordingTimer = setTimeout(
      () => expireRecording(state, field, revision),
      RECORD_TIMEOUT_MS,
    );
  } catch (error) {
    recordFailed(state, { field, epoch, revision }, error);
  } finally {
    state.recordOperations.value--;
  }
}

function expireRecording(state: RecordingState, field: HotkeyField, revision: number): void {
  if (revision === state.hotkeyRevisions[field]) {
    state.recordIssues.value[field] = { conflict: false, detail: '录制已超时，请重试' };
  }
  void stopRecording(state);
}

function blurRecording(state: RecordingState): void {
  if (state.recordingField.value) void stopRecording(state);
}

async function finishRecording(state: RecordingState, hotkey: string | null): Promise<void> {
  const field = state.recordingField.value;
  if (!field) return;
  const revision = state.hotkeyRevisions[field];
  const stopped = await stopRecording(state);
  if (
    !stopped ||
    state.disposed ||
    !hotkey ||
    !state.context.draft.value ||
    revision !== state.hotkeyRevisions[field]
  )
    return;
  const reason = shortcutError(hotkey);
  if (reason) {
    state.recordIssues.value[field] = { conflict: true, detail: reason };
    return;
  }
  clearRecordIssue(state, field);
  const unchanged = state.context.draft.value[field] === hotkey;
  state.context.draft.value[field] = hotkey;
  if (unchanged) state.context.scheduleHotkeyCheck();
}

function recordingKeydown(state: RecordingState, event: KeyboardEvent): void {
  if (!state.recordingField.value && !state.recordOperations.value) return;
  // 在窗口捕获阶段处理，不能把 Esc/Enter/Space 交给主面板或当前按钮。
  event.preventDefault();
  event.stopImmediatePropagation();
  if (!state.recordingField.value || event.repeat) return;
  if (event.key === 'Escape') {
    void stopRecording(state);
    return;
  }
  if (event.isComposing || event.keyCode === 229) return;
  const hotkey = shortcutFromKeyboard(event);
  if (hotkey) void finishRecording(state, hotkey);
}

async function receiveRecording(state: RecordingState, payload: RecordedHotkey): Promise<void> {
  const field = state.recordingField.value;
  if (!field) return;
  if (!state.recordSession && state.recordOperations.value) {
    state.earlyRecordings.set(payload.session, payload.hotkey);
    return;
  }
  if (payload.session !== state.recordSession) return;
  await finishRecording(state, payload.hotkey);
}

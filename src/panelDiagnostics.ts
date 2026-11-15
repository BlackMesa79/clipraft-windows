import { invoke } from '@tauri-apps/api/core';
import type { PanelState } from './panel';

// 仅诊断开关启用后发送状态编号，不携带记录ID、正文或错误内容。
export function createPanelDiagnostics() {
  let enabled = false;
  let frame = 0;
  let disposed = false;
  function send(stage: 'received' | 'frame', panel: Pick<PanelState, 'serial' | 'session'>): void {
    void invoke('panel_diagnostic', {
      stage,
      serial: panel.serial,
      session: panel.session,
    }).catch((error: unknown) => console.warn('面板诊断上报失败', error));
  }
  function record(panel: PanelState): void {
    if (!enabled || disposed) return;
    const stamp = { serial: panel.serial, session: panel.session };
    send('received', stamp);
    cancelAnimationFrame(frame);
    // 帧回调仅说明WebView执行过该状态的帧，不作为桌面已显示的证明。
    frame = requestAnimationFrame(() => {
      frame = requestAnimationFrame(() => send('frame', stamp));
    });
  }
  function start(value: boolean, panel: PanelState): void {
    enabled = value === true;
    record(panel);
  }
  function stop(): void {
    disposed = true;
    cancelAnimationFrame(frame);
  }
  return { record, start, stop };
}

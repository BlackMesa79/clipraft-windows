// 仅用于离屏测试的隔离合成数据，不连接真实数据库或剪贴板。
export const settingsMock = `(() => {
  const defaults = {theme:'light',layout:'list',compact:false,hide_on_blur:false,select_first_on_show:false,main_position:'remember',autostart:false,pause_on_start:false,max_items:1000,max_bytes:1073741824,max_days:30,ignored_apps:['KeePass.exe','KeePassXC.exe','1Password.exe','Bitwarden.exe'],sensitive_detection:true,sensitive_high_entropy:false,sensitive_ttl_minutes:10,main_hotkey:'alt_v',quick_hotkey:'ctrl_shift_v',quick_visible_count:5};
  const key = (id,label,hold_label=label) => ({id,label,hold_label});
  const range=(min,max,step=1)=>({min,max,step});
  const options={defaults,ranges:{max_items:range(10,100000),max_bytes:range(10485760,107374182400),max_days:range(0,3650),sensitive_ttl_minutes:range(1,1440),quick_visible_count:range(3,15)},main_hotkeys:[key('alt_v','Alt + V'),key('ctrl_alt_v','Ctrl + Alt + V'),key('win_alt_v','Win + Alt + V'),key('ctrl_backtick','Ctrl + \u0060')],quick_hotkeys:[key('ctrl_shift_v','Ctrl + Shift + V','Ctrl + Shift'),key('ctrl_alt_shift_v','Ctrl + Alt + Shift + V','Ctrl + Alt + Shift'),key('alt_shift_v','Alt + Shift + V','Alt + Shift')],bytes_per_mb:1048576,ignored_apps_max:256};
  window.settings=JSON.parse(JSON.stringify(defaults)); window.settingUpdates=[];window.cleanupPreviews=[];window.hotkeyChecks=[];
  const invoke=window.__TAURI_INTERNALS__.invoke;
  window.__TAURI_INTERNALS__.invoke=async(cmd,args={})=>{
    if(cmd==='sync_main_layout')return;
    if(cmd==='get_settings')return JSON.parse(JSON.stringify(window.settings));
    if(cmd==='get_setting_options')return options;
    if(cmd==='startup_error')return null;
    if(cmd==='check_hotkeys'){
      window.hotkeyChecks.push(args);
      if(window.failHotkeyCheck)throw Error('合成检测失败');
      if(window.holdHotkeyCheck)return new Promise(resolve=>window.releaseHotkeyCheck=resolve);
      const mainOwned=args.mainHotkey===window.settings.main_hotkey;
      return window.hotkeyCheckResult??{main:{status:mainOwned?'owned':'available',message:mainOwned?'当前由本应用注册':'未发现系统注册冲突'},quick:{status:'available',message:'未发现系统注册冲突'}};
    }
    if(cmd==='preview_cleanup'){window.cleanupPreviews.push(args.policy);if(window.failPreview)throw Error('合成预估失败');return 27;}
    if(cmd==='update_settings'){
      window.settingUpdates.push(args.settings);
      if(window.failSettings)throw Error('快捷键已被其他程序占用');
      if(window.holdSettings)await new Promise(resolve=>window.releaseSettings=resolve);
      window.settings=JSON.parse(JSON.stringify(args.settings));
      if(window.normalizedIgnoredApps)window.settings.ignored_apps=[...window.normalizedIgnoredApps];
      return window.settings;
    }
    return invoke(cmd,args);
  };
})();`;

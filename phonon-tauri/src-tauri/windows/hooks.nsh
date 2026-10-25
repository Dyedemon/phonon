; Phonon NSIS hooks — install/uninstall customization
;
; 数据策略（与 tauri 模板分工）：
; - Roaming 用户数据（媒体库/队列/封面缓存）：由 tauri NSIS 模板原生
;   的"删除应用程序数据"checkbox 管理（默认保留），不在此重复询问。
; - Local 缓存（EBWebView/logs，非用户资产）：卸载时静默清理。
;
; 历史：此处曾有一个"用自研 phonon-uninstaller.exe 替换 NSIS 卸载器"
; 的 POSTINSTALL 宏，但该二进制从未加入 bundle resources——宏长期处于
; 半执行状态。自研安装器/卸载器（phonon-installer / phonon-uninstaller）
; 保留在仓库，待 1.0 品牌化安装体验时再接入。

!macro NSIS_HOOK_POSTINSTALL
  ; 清 0.1.x 时代遗留的自研卸载器残留文件
  Delete "$INSTDIR\phonon-uninstaller.exe"
!macroend

!macro NSIS_HOOK_PREINSTALL
!macroend

!macro NSIS_HOOK_PREUNINSTALL
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ; Local 缓存（webview/日志）不是用户资产，卸载直接清；
  ; /r 处理非空目录（EBWebView 内有大量文件）。
  RMDir /r /REBOOTOK "$LOCALAPPDATA\com.phonon.app"
  ; 0.1.x 时代的残留数据目录
  RMDir /r /REBOOTOK "$LOCALAPPDATA\Phonon\.phonon_data"
!macroend

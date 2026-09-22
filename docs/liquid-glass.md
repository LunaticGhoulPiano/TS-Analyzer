    Liquid Glass 現在是 Windows 桌面擷取原型。在 Theme → Liquid Glass (experimental) 啟用；Glass settings 提供折射強度、Frost、RGB 色散、放大倍率、Lens depth、Dark tint 與 Squircle。

  - 背景來自 Windows Graphics Capture 的螢幕影像，不再使用固定漸層或示意背景。
  - 即時模式對主視窗設定 WDA_EXCLUDEFROMCAPTURE，避免擷取自己的畫面造成回授。主視窗會從遵守此設定的截圖、錄影與螢幕分享消失。
  - 要使用 Win＋Shift＋S：進入 Theme → Glass settings → Freeze background / screenshot mode。取得對齊背景後才可凍結；程式保留該影像，關閉擷取並等待工作執行緒結束，再恢復原本的截圖可見性。背景沒有寫入磁碟。
  - 凍結時主畫面顯示狀態；視窗移動、縮放或最小化不會自動恢復擷取。背景固定，縮放時以保留影像重新渲染玻璃。使用 Resume live background 才重新啟動即時擷取及排除設定。
  - 凍結會結束本程式的擷取提示；如果其他應用程式仍在擷取同一螢幕，Windows 可能繼續顯示提示邊框。
  - 切離主題、初始化／擷取失敗或正常結束時，停止擷取並恢復原設定。凍結模式下切換主題也會釋放保留影像。
  - 保留 Windows 的擷取提示邊框，不要求 borderless capture 權限。
  - 即時模式只支援視窗 client area 完整位於單一螢幕內。跨螢幕、部分移出桌面或最小化時停止擷取；畫面暫用透明底，重新完整移入螢幕後重建擷取。
  - 背景與視窗位置使用 Win32 實體桌面座標，支援負座標。同一螢幕內移動或縮放時，立即更新 shader 的來源 UV；桌面影像尚未更新時沿用上一幀，不因座標不完全一致而退回透明。跨螢幕或顯示模式變更仍等待新來源。
  - 擷取失敗會顯示錯誤並回到 Transparent，不用假背景掩蓋錯誤。
  - GPU 來源為 D3D11 BGRA8；背景執行緒最多約 30 次／秒讀回與縮小目前螢幕，再透過 CPU RGBA8 上傳既有 wgpu OpenGL backend。不是零拷貝，實際更新率與延遲取決於裝置。
  - 貼圖為目前螢幕實體尺寸的一半、長邊上限 2048；只保留最新影像，不排隊累積幀。移動時共用 RGBA buffer，只更新取樣矩形；不複製整張影像、不因移動而重新上傳貼圖。相較先前僅裁切視窗，讀回整個螢幕會增加頻寬成本。
  - 渲染順序：桌面擷取 → 螢幕影像縮小 → wgpu texture → 水平／垂直 Gaussian blur → 依目前視窗座標取樣的曲面透鏡 → egui 文字／控制項。
  - Lens depth 使用視窗短邊比例；圓弧曲線產生較寬的邊緣折射。加入色散、放大、低強度顆粒、邊緣光與方向性高光，Dark tint 可調整閱讀對比。
  - 主視窗的原生影片 child HWND 是否受擷取排除涵蓋，需隨實際播放器驗證；獨立影片視窗不屬於主視窗排除範圍。
  - 原型尚未提供 HDR 色彩管理、跨螢幕拼接或擷取影像與 DWM 呈現的同步保證。受保護內容也不保證可擷取。
  - 沿用 windows 0.62.2。Composition 探測僅在 dev-dependencies 啟用對應 API，並將既有 windows-core 0.62.2 列為測試直接依賴，以使用 COM implement 巨集。Cargo.lock 僅新增該依賴關係，沒有新增套件版本；建置皆使用離線快取，未安裝工具。


    Windows 相容性：

  - 本次功能範圍為 Windows 10／11；不新增 macOS 或 Linux 的擷取實作。
  - 即時擷取基線為 Windows 10 2004（build 19041）以上，包含 Windows 10 22H2（19045）及 Windows 11。這是 API 基線，不代表所有版本、驅動與執行環境都已實測。
  - 在修改視窗擷取排除前，使用 RtlGetVersion 檢查實際版本；不依賴 EXE 的 supportedOS manifest。舊版 Win10（例如 1809／1909）會拒絕啟用即時玻璃，顯示原因並退回 Transparent。
  - 版本符合後仍檢查 GraphicsCaptureSession::IsSupported、D3D11 初始化、排除設定與擷取結果；執行期失敗走相同清理及退回路徑。
  - 不使用 IsBorderRequired／Borderless 權限作為解法：該 API 最低 build 20348，不能涵蓋一般 Win10。兩個平台均以停止擷取的凍結模式恢復截圖。
  - 目前原生測試環境是 Windows 11 build 26200。Win10 版本門檻有單元測試，但尚未在 Win10 實機／VM 執行；Win10 實測及多螢幕不同 DPI 的人工驗證仍待完成。


    無螢幕擷取的 Composition 路徑實測（Windows 11 build 26200）：

  - 探測程式位於 crates/tsan-gui/src/platform/windows/composition_probe.rs，僅在測試時編譯，未切換主程式 backend。
  - 普通 HostBackdropBrush 確實能跟隨背景變動；不設定主測試視窗的 capture affinity，第二個量測視窗能擷取到其紅色標記，證明自身繪製內容仍可截圖。
  - Composition 接受普通 2D affine transform 的 effect factory；但 SetSourceParameter 接上 HostBackdropBrush 時回傳 E_INVALIDARG（0x80070057）：Backdrop brushes cannot be transformed。
  - DisplacementMap effect factory 同樣回傳 E_INVALIDARG。因此這條已測公開 API 路徑不能直接替代既有折射 shader；不把一般系統背景模糊宣稱為完成 Liquid Glass。
  - 探測的量測端暫時使用 Windows Graphics Capture，因此測試期間可能有黃框；候選背景筆刷本身沒有擷取工作階段。所有測試視窗結束後自動清理。
  - 原生測試使用每螢幕 DPI awareness，色塊以父子視窗保持遮擋關係，等待已知背景圖樣出現才比較像素，避免把初始化影像誤判成變形效果。
  - Magnification API 的自訂影像 callback 已被官方標示 deprecated，且要求 DWM 關閉；未採用，也未修改 DWM、UIAccess 或系統設定。
  - 尚未達成「即時折射＋無黃框＋一般截圖可见＋Win10／Win11 相容」全部要求。目前正式實驗主題仍是擷取方案與手動凍結模式。

    參考來源：

  - [Windows Graphics Capture](https://learn.microsoft.com/en-us/windows/win32/api/windows.graphics.capture.interop/nf-windows-graphics-capture-interop-igraphicscaptureiteminterop-createformonitor)：取得螢幕 capture item。
  - [HostBackdropBrush](https://learn.microsoft.com/en-us/uwp/api/windows.ui.composition.compositor.createhostbackdropbrush)：系統背景筆刷與禁止像素讀回。
  - [DisplacementMapEffect](https://microsoft.github.io/Win2D/WinUI3/html/T_Microsoft_Graphics_Canvas_Effects_DisplacementMapEffect.htm)：不支援 Composition。
  - [MagSetImageScalingCallback](https://learn.microsoft.com/en-us/windows/win32/api/magnification/nf-magnification-magsetimagescalingcallback)：已棄用且要求 DWM 關閉。
  - [IsBorderRequired](https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.graphicscapturesession.isborderrequired)：取消擷取提示的版本與授權要求。
  - [SetWindowDisplayAffinity](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowdisplayaffinity)：Windows 10 2004 以上的自身視窗排除。
  - [ShojiWM Liquid Glass 設定](https://github.com/bea4dev/liquid-glass-config-shojiwm)：參考實際 backdrop、整片視窗及寬曲面折射的視覺方向；未引入其 compositor 或框架程式。
  - [naughtyduk/liquidGL](https://github.com/naughtyduk/liquidGL)：圓角距離、色散、放大、邊界與高光概念。
  - [OverShifted/LiquidGlass](https://github.com/OverShifted/LiquidGlass/tree/3797fa541c1f026c521a75885ee271f03bdf9f0f)：Gaussian 配對權重、superellipse 距離近似，以及分離式 framebuffer 流程。
  - 不包含 DOM rasterizer、WebGPU fallback、OverEngine 或 ImGui。

    以下保留兩個參考來源的 MIT 授權通知。
    MIT License

    Copyright (c) NaughtyDuk

    Permission is hereby granted, free of charge, to any person obtaining a copy
    of this software and associated documentation files (the "Software"), to deal
    in the Software without restriction, including without limitation the rights
    to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
    copies of the Software, and to permit persons to whom the Software is
    furnished to do so, subject to the following conditions:

    The above copyright notice and this permission notice shall be included in all
    copies or substantial portions of the Software.

    THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
    IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
    FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
    AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
    LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
    OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
    SOFTWARE.
    MIT License

    Copyright (c) 2026 Sepehr Kalanaki

    Permission is hereby granted, free of charge, to any person obtaining a copy
    of this software and associated documentation files (the "Software"), to deal
    in the Software without restriction, including without limitation the rights
    to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
    copies of the Software, and to permit persons to whom the Software is
    furnished to do so, subject to the following conditions:

    The above copyright notice and this permission notice shall be included in all
    copies or substantial portions of the Software.

    THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
    IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
    FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
    AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
    LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
    OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
    SOFTWARE.

    驗證方式：

  - 依 docs/dev/env.md 設定 GStreamer 與 TSDuck DLL 搜尋路徑。
  - 一般 GUI 測試：`rtk cargo test --offline --locked -p tsan-gui`。
  - 含 OpenGL 與 Windows 擷取實測：`rtk cargo test --offline --locked -p tsan-gui -- --include-ignored --test-threads=1`。
  - 原生擷取測試會短暫顯示測試視窗，驗證排除自己的黑色視窗、讀到後方白色視窗、移動／縮放座標、背景即時更新及設定恢復。新增連續 60 次移動時背景不消失的檢查，以及第二個獨立擷取工作階段，確認凍結後能讀到前景視窗、背景影像保持不變，以及恢復即時模式後重新排除自己。測試不儲存桌面影像。
  - 實際 egui／OpenGL 視窗測試涵蓋連續 20 幀移動時保留即時玻璃、凍結、縮放時保留背景、手動恢復、主題切換清理、模擬擷取裝置錯誤退回 Transparent，以及重新啟用。
  - Composition 探測：`rtk cargo test --offline --locked -p tsan-gui composition_probe -- --ignored --nocapture --test-threads=1`；HostBackdrop Win32 路徑需 Windows 11，不能以此代替 Win10 實測。
  - GUI 原生測試只控制自行建立的視窗，使用不啟用視窗的選項；不發送全域鍵盤／滑鼠輸入。
  - GPU 測試使用格線測試圖驗證 texture 上傳、模糊、折射、DPI 與像素讀回；設定 LIQUID_GLASS_PREVIEW 時可輸出 1280 × 800 RGBA8 測試圖供視覺檢查。

  - 實際透明視窗整合測試可透過 LIQUID_GLASS_APP_PREVIEW 指定本機 RGBA 檔案路徑，另寫出同名 .size 尺寸檔。這是程式自身 framebuffer 讀回，與被排除的作業系統截圖不同；一般執行不會保存擷取影像。

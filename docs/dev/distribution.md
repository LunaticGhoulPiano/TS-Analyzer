# 平台版本、發行包與離線 PDF

## 平台版本與驗證範圍

| 平台 | 原生應用版本 | 發行狀態 | 系統目標與限制 |
| --- | --- | --- | --- |
| Windows x64 | 0.2.0 | 本機 development package | Windows 10 22H2／Windows 11；本次實機是 Windows 11，Windows 10 尚需 VM／實機驗證 |
| macOS | 0.0.0 | not-implemented，並非已發行版 | 原生視訊、檔案對話框與安裝後端尚未實作 |
| Linux | 0.0.0 | not-implemented，並非已發行版 | 原生 Wayland／X11 視窗、視訊與安裝後端尚未實作 |

  - 唯一的平台版本來源是 packaging/platforms.toml。每個平台獨立遞增；Cargo workspace 的 crate version 是另一個概念，不用它判斷其他 OS 的新版。
  - build.rs 依 CARGO_CFG_TARGET_OS 選取平台版本；Help、Update 與 package identity 都使用該版本。打包器核對應用實際回報的版本／架構，防止忘記重編而配錯 manifest。
  - Tag 使用 windows-v0.2.0、macos-v0.1.0、linux-v0.1.0 這類名稱；後兩者只是命名示例。Windows ZIP 為 TS-Analyzer-windows-v0.2.0-x86_64-portable.zip。
  - Windows 10／11 共用 Windows 發行軌，不另拆 Windows 10 版本。GPU 能力仍取決於驅動；提供 D3D11、D3D12 後端。Liquid Glass 的 live capture 需要 Windows 10 2004+，擷取失敗會回到 Transparent。
  - 目前 Windows 的 GUI／播放器使用 HWND 與 Direct3D；不能因共用 Rust core 或更新介面，就宣稱已有可用的 macOS／Linux 發行包。

## 不要求使用者安裝開發環境

| 元件 | 建置者需要 | 一般使用者拿到什麼 |
| --- | --- | --- |
| Rust、rust-toolchain.toml、Cargo、winit source | 是 | 編譯進應用，不安裝 compiler／crate |
| MSVC compiler、Windows SDK、pkg-config、headers／import libraries | 是 | 不附開發工具；只附 private MSVC CRT |
| TSDuck | SDK | tscore／tsduck DLL 與必要資料、授權 |
| GStreamer | MSVC x64 runtime＋development | 必需 DLL 的 PE import closure、指定 plugins 與 scanner |
| XeLaTeX | 已有且驗證過的 TeX Live | 只供本程式報告用的 private XeLaTeX subset |

  - Windows 解壓後執行 TS-Analyzer.exe 即可。Setup.exe 可將同一套內容安裝到 %LOCALAPPDATA%\Programs\TS-Analyzer，不需要系統管理員權限。
  - app、runtime/gstreamer、runtime/tsduck、runtime/msvc、runtime/tex、licenses 各自分層。launcher 的 PATH、GStreamer plugin path／scanner／registry 都只套用到子程序；不更改系統 PATH、registry 設定或既有 SDK。
  - GStreamer 不掃描使用者的 plugins。cache 按版本和 package 路徑隔離，避免兩份同版 portable package 共用舊 registry。
  - PE imports 會遞迴涵蓋 normal／delay imports；不能單憑此推論所有動態載入元件齊備，因此打包後另做必要元素、實際 H.264／H.265 解碼與 PDF 驗證。
  - 這是可檢查的本機未簽章 package。沒有替專案上傳 GitHub Release，沒有 Authenticode 簽章，也尚未在全新 Windows 10 VM 認證；不能稱為已完成三平台正式發行。

## PDF 按一次即可，不另下載 TeX

  - 一般使用者選 PDF，程式在背景執行 package 自帶的 XeLaTeX；不用先安裝完整 TeX Live，也不在匯出時下載套件。
  - subset 保留 xelatex／xetex／xdvipdfmx／kpsewhich、其實際 DLL、預建 xelatex.fmt、報告用到的巨集、必要字型及 notices。沒有 TeX 編輯器、tlmgr 或整個 CTAN。
  - TeX 依賴由完整報告的 -recorder 輸出追蹤，再加入引擎動態需求、字型與 PDF driver 的必要資料。報告模板增加新 package 時，必須重新建置並驗證 subset，不能把這份 subset 當成任意 TeX 文書環境。
  - private XeLaTeX 使用自己的 TEXMF、fontconfig 和 cache，清除繼承的 TeX 搜尋設定。可以讀 Windows 字型；缺少 Microsoft JhengHei 時使用包內 FandolSong。shell escape 關閉。
  - 使用者既有的 XeLaTeX／MiKTeX／TeX Live 不會被安裝、升級、覆寫或改設定。使用者若另選 LaTeX source 匯出，仍可把生成的 .tex 交給自己的完整環境編輯。
  - 本機精簡結果：GStreamer 約 40 MiB、TeX runtime 約 70 MiB，整包未壓縮約 165 MiB、ZIP 約 71 MiB。實際數字以輸出 package-size.json 為準；不以編譯器 exe 的大小冒充完整 runtime 大小。
  - 直接改用 PDF library 可免除 TeX，但需重新實作字型嵌入、CJK、換行、分頁、表格與版面；本次保留已使用的 XeLaTeX 工作流程，以精簡離線 runtime 解決使用者安裝時間問題。

## 相同更新入口，不混用平台安裝方式

  - GUI 呼叫共同的 check_for_updates／start_check、start_download、install_staged_update；OS 差異放在 platform module。
  - Windows 查詢 GitHub Releases 的所有可檢查分頁，只選 windows-v 的 stable tag，依數字版本排序，不直接使用可能屬於其他平台的 releases/latest。
  - 下載前驗證 repository URL、平台 asset 名稱、大小與 GitHub 提供的 SHA-256 digest；下載後核對 size/hash。解壓前拒絕 traversal、大小寫重名、symlink、過多 entries 與超過限制的展開大小；解壓後再驗證版本與必要檔案。
  - 沒有符合的 Release／asset／digest 時清楚顯示無可用更新，不假裝下載成功。不自動發布或下載不相干的工具。
  - 更新與安裝使用 Windows 內建 PowerShell，明確指定其系統模組目錄，避免繼承 PowerShell 7 的 PSModulePath 造成版本混用。
  - staging：%LOCALAPPDATA%\TS-Analyzer\updates\<version>-<Unix milliseconds>\。
  - 安裝版本：%LOCALAPPDATA%\Programs\TS-Analyzer\releases\v<Windows version>\。current-version.toml 指向 current／previous version；新包先複製至 staging，完成後再切換 pointer，保留舊版與使用者設定。
  - 安裝版 TS-Analyzer.exe --rollback 可切回 previous version。相同版本重跑安裝保留原 previous pointer。
  - macOS／Linux 目前同 API 會回報尚無已實作的 backend，沒有執行 Windows installer。macOS 未來需簽章／notarization 與 .app 更新，Flatpak 則應交由 Flatpak 的更新交易處理。
  - 完整包更新會重新下載整個 package；目前沒有自製 delta update。不得把本機 fixture 驗證說成已通過真實 GitHub 新舊版本的完整更新。

## Linux 的處理方式

  - Wayland 是顯示協定；Hyprland 與 niri 是 Wayland compositor。它們不是要分別編譯三個應用版本的理由。[niri 專案](https://github.com/niri-wm/niri)
  - Linux GUI 應同時建置 winit 的 Wayland／X11 backend，使用相應 display connection；不要用 WM 名稱硬編碼切換邏輯。winit 提供這兩類 backend。[winit 官方 API](https://docs.rs/winit/latest/winit/)
  - 檔案選擇與桌面整合走 xdg-desktop-portal；Wayland 不應沿用 Windows HWND／強制搬動視窗的方式。先完成共用畫面中的視訊 texture 呈現，再分平台做 GPU interop。
  - 視窗裝飾、blur、位置控制與螢幕擷取是可選能力；不支援時退回正常不透明主題。不能讓 Liquid Glass 成為分析或播放的前置條件，也不能保證各 compositor 的特效完全一致。
  - 建議 Linux 發行使用 Flatpak：固定 runtime，啟用 Wayland 與 fallback-x11，透過 portal 開檔；runtime 可共用，更新只取變動內容。首次安裝仍可能需要下載 runtime，不能稱為零額外容量。[Flatpak runtime／portal／更新設計](https://docs.flatpak.org/en/latest/basic-concepts.html)
  - AppImage 便於單檔分發，但仍需處理 glibc baseline、GPU driver 和圖形協定；deb/rpm 可共用 distro 套件，但需要不同發行版的相容性與維護。此處為 Linux 發行設計，尚不是已生成的可用 Linux installer。
  - 發行驗證需涵蓋 GNOME／KDE Wayland、Hyprland、niri、X11，以及 DPI、檔案 portal、H.264/H.265、音訊與 seek；單一 Windows 主機不能代替這個矩陣。

## 設定與報告位置

  - 設定：%APPDATA%\TS-Analyzer\tsan-config.toml，記錄 theme、transparent_opacity、player_backend、最多 12 個 recent_files。首次讀取時可匯入同目錄舊 recent-files.txt，保留舊檔；不自動重新開啟整個 session。
  - macOS 設定路徑已定義為 ~/Library/Application Support/TS-Analyzer/tsan-config.toml；Linux 為 $XDG_CONFIG_HOME/TS-Analyzer/tsan-config.toml，未設定時使用 ~/.config/TS-Analyzer/tsan-config.toml。
  - 開啟的 TS、比較勾選、頁籤順序、GOP 欄寬、zoom、Glass 細部參數仍是記憶體狀態；Log 可手動 dump，不自動寫入持久檔。
  - 匯出目的地是使用者在 Save dialog 選的資料夾。預設檔名 ts-analyzer_report_YYYYMMDD_HHMMSS.ext 使用本機時間；可以自行改名。
  - 選 .cbor 只寫該 CBOR；選 .xlsx 直接由記憶體 CBOR 生成該 XLSX，不會額外落地 CBOR 或跑 LaTeX。
  - 選 .tex 生成主檔與 <sanitized-stem>_sections；選 .pdf 另保留同名 .tex、sections、images、build 診斷資料。stem 中非 ASCII 字母／数字／-／_ 的字元會轉成 _。
  - 例如選 D:\Reports\news.pdf，會產生 D:\Reports\news.pdf、news.tex、news_sections\Overview.tex、GOP.tex 等分頁、news_sections\images\ 的 SVG／PDF 圖片，以及 news_sections\build\report.log／aux／toc／fls／pdf。不自動寫到原始 TS 旁邊。
  - 更完整的資料定義與限制見 analysis-player-reports.md。

## 打包與測試入口

  - packaging/windows/build-package.ps1：讀 packaging/platforms.toml；參數 OutputDirectory、GStreamerRoot、TSDuckRoot、MsvcRuntime、TexRoot、TexRecorder，AppExe 預設 target/release/tsan-gui.exe；可用 SkipArchive 只產生目錄。輸出目錄必須尚未存在。
  - 建置前以既有環境編譯 release app，生成一份完整 PDF 的 report.fls 作為 TexRecorder。腳本只複製既有檔案、讀 PE imports、編譯 launcher，不安裝或下載工具。
  - packaging/windows/test-runtime.ps1：把既有 gst-launch 診斷工具複製到新的測試目錄，使用 package 私有 DLL/plugins，對 Rec 的 16 個 TS 跑 D3D11／D3D12 decoder→fakesink，驗證影格與 EOS，無影片視窗。
  - packaging/windows/test-update-archive.ps1：驗證完整 fixture，以及 traversal、重名、symlink、hash／size／version 錯誤、缺檔均被拒絕；不執行 fixture 裡的 installer。
  - TS-Analyzer.exe --wait --package-smoke <TS> <output-directory>：核對 TSDuck、GStreamer adapter、CBOR／XLSX／LaTeX／PDF，寫 smoke-result.txt。--build-info <file> 寫出編譯時平台身份，不開 GUI。

## 參考

  - [GStreamer Windows runtime 與 development 的區別](https://gstreamer.freedesktop.org/documentation/installing/on-windows.html)
  - [GStreamer Windows 部署](https://gstreamer.freedesktop.org/documentation/deploying/windows.html)
  - [GitHub Releases API](https://docs.github.com/en/rest/releases/releases)

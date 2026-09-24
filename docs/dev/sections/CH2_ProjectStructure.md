# CH2 — Project Structure

## Windows

### Repository layout

  - The tree shows maintained source and documentation, plus the generated build, work, and delivery roots. Paths are relative to the repository.

~~~text
TS-Analyzer/
    README.md
    LICENSE
    .gitignore
    Cargo.toml
    Cargo.lock
    rust-toolchain.toml
    .vscode/
        tasks.json
        extensions.json
        c_cpp_properties.json
        windows/Invoke-Development.ps1
    crates/
        tsan-core/
        tsan-input/
        tsan-analyzer/
        tsan-tsduck-sys/
            build.rs
            native/                  # C++ bridge and C ABI
            src/
        tsan-player/
        tsan-gui/
            build.rs
            src/
                frontend.rs
                frontend/            # Windows GUI and macOS/Linux stubs
                app.rs
                analysis_views.rs
                player_worker.rs
                report_export.rs
                report_graphics.rs
                xlsx.rs
                config.rs
                update.rs
                update/              # Per-OS updater implementations/stubs
                liquid_glass.rs
                liquid_glass.wgsl
                platform/windows/
        tsan-runtime/                # Shared AnalysisService; live pipeline planned
        tsan-platform/               # OS identity, paths, deployment modes
        tsan-diagnostics/            # Persistent logs, panics, OS-specific dump collector
        tsan-launcher/               # Cargo-built Windows package launcher
        tsan-cli/                    # CLI entry placeholder
    scripts/windows/                 # Product update scripts
        fetch-release.ps1
        stage-update.ps1
        apply-update.ps1
        update-files.ps1
    developmentHelpers/
        config/
            windows.example.psd1
            deny.toml
        scripts/windows/             # Build, run, CI, and tool discovery
        packaging/
            platforms.toml
            windows/                 # Runtime assembly and Inno template/build
        tests/
            diagnostics/
            core/
            windows/
        examples/
            diagnostics/
            core/
            analyzer/
            player/
            runtime/
        test-data/
            inputs/
                manifest.toml
                manifest.template.toml
                synthetic/
                local/               # Optional, ignored recordings
            expected/
    outputs/                         # Generated files; ignored by Git
        windows/                     # Working files, test logs, symbols, dev state
        deploy/                      # User artifacts only; no build scripts/logs
            windows/                 # One formatted folder per complete release
            macos/                   # Reserved; no native package yet
            linux/                   # Reserved; no native package yet
    docs/
        usr/
            outlines.md
            sections/
                CH1_InstallationAndUpdates.md
                CH2_AnalysisAndPlayback.md
                CH3_ReportsAndSettings.md
        dev/
            outlines.md
            sections/
                CH1_DevelopmentFromSource.md
                CH2_ProjectStructure.md
                CH3_GUI.md
                CH4_Roadmap.md
    licenses/
        liquid-glass-notices.txt
        rust/                        # Reviewed, exact-version supplemental notices
            sources.json             # Upstream revisions, URLs, and SHA-256 values
            <crate>-<version>/
    target/                          # Generated Cargo output
~~~

  - Unit tests remain next to their Rust modules under cfg(test). Examples and integration tests under developmentHelpers/ are registered in the owning crate's Cargo.toml.
  - scripts/ contains product code; developmentHelpers/scripts/ contains developer tooling. General analysis/playback logic belongs in library crates.
  - Configuration locations are listed in [Chapter 1](CH1_DevelopmentFromSource.md).
  - Current behavior is described below. Unimplemented live-input/CLI/native-platform work is retained in [Chapter 4](CH4_Roadmap.md).

### Compilation and distribution chain

~~~mermaid
flowchart TD
    editor["VS Code tasks.json"] --> adapter[".vscode/windows adapter"]
    shell["PowerShell -Action"] --> adapter
    adapter --> dev["developmentHelpers/scripts/windows"]
    dev --> config["Local overrides and SDK discovery"]
    dev --> cargo["Cargo: pinned Rust and Cargo.lock"]
    cargo --> rust["Rust crates"]
    cargo --> cpp["tsduck-sys build.rs: C++20 bridge"]
    cpp --> sdk["TSDuck SDK and MSVC linker"]
    rust --> binary["target/debug or target/release"]
    sdk --> binary
    dev --> deploy["Deploy: one delivery task"]
    deploy --> ci["CI checks and delivery preflight"]
    ci --> cargo
    binary --> package["New package: app + private runtime + user docs + licenses"]
    cargo --> launcher["tsan-launcher: static CRT"]
    launcher --> package
    package --> zip["Portable ZIP + SHA-256 in working artifacts"]
    zip --> inno["Installer: same package, generated ISS + ISCC"]
    inno --> setup["Setup EXE + SHA-256 in working artifacts"]
    setup --> complete["Verify all four files and complete delivery"]
    complete --> delivery["outputs/deploy/windows/TS-Analyzer-windows-vVERSION-x86_64/"]
    dev --> work["outputs/windows: assembly, ISS, logs, symbols"]
~~~

  - Build/Run/Test/CI/Deploy commands, prerequisites, and each output directory are in [Chapter 1](CH1_DevelopmentFromSource.md).
  - The launcher is compiled during packaging. GStreamer, TSDuck, MSVC CRT, and the report's XeLaTeX subset remain private to the distributed application.

### 元件與實際依賴

| Crate | 現有職責 |
| --- | --- |
| tsan-core | 188／192／204-byte 格式探測、H.264／H.265 畫面解析、影片 PTS／檔案位置索引。 |
| tsan-input | 檔案來源探測與可重用輸入型別。 |
| tsan-analyzer | 離線封包掃描、PID／節目、PSI／SI、TR 101 290 與標準別規則、時間戳、bitrate、GOP，產生 AnalysisReport。 |
| tsan-tsduck-sys | 進程內 C ABI／C++ TSDuck bridge；原生 session、section／continuity／standard 與 SPS／VUI。 |
| tsan-player | 播放 session 與 Windows GStreamer D3D11／D3D12 後端、appsrc 供應與索引 seek。 |
| tsan-gui | UI、每檔文件狀態、分析排程、單一播放 worker、匯出、設定、Log 與更新。 |
| tsan-runtime | 共用 AnalysisService，GUI 背景工作經此呼叫 Analyzer；未來即時串流的有界分發仍待實作。 |
| tsan-platform | OS 判斷、各平台設定／快取／更新／診斷目錄、部署模式。 |
| tsan-diagnostics | 共用輪替 Log、panic backtrace、安全清理；Windows 另開 collector 程序保存 native minidump，其他 OS 明確占位。 |
| tsan-launcher | Windows 封裝版啟動器，設定私有 runtime 環境後啟動 GUI；其他 OS 明確回報未實作。 |
| tsan-cli | 保留供未來命令列分析；目前執行會明確回報尚未實作。 |

~~~mermaid
flowchart TD
    gui["tsan-gui / OS frontend"] --> runtime["tsan-runtime / AnalysisService"]
    runtime --> analyzer["tsan-analyzer"]
    gui --> platform["tsan-platform"]
    launcher["tsan-launcher"] --> platform
    launcher --> diagnostics["tsan-diagnostics"]
    gui --> diagnostics
    runtime --> diagnostics
    diagnostics --> platform
    gui --> player["tsan-player"]
    gui --> input["tsan-input"]
    analyzer --> core["tsan-core"]
    analyzer --> bridge["native-tsduck feature: tsan-tsduck-sys"]
    bridge --> duck["libtsduck / libtscore"]
    player --> core
    player --> input
    input --> core
    player --> gst["GStreamer"]
    gui --> export["GUI 內的報告匯出模組"]
    export --> tex["XeLaTeX：只供 PDF"]
    subgraph planned["保留占位，尚未接入"]
        cli["tsan-cli"]
    end
~~~

  - 箭頭代表呼叫或依賴，不表示每個 crate 是獨立程序。一般使用者啟動 GUI；TSDuck 由 C++ bridge 在同一程序呼叫，不啟動 tsp。
  - GUI／播放器原生實作目前限定 Windows。其他平台 main／backend 明確回報未實作；共用 Rust 程式碼不等於三平台應用已完成。
  - FFI 隔離於 tsan-tsduck-sys；GUI／Windows API 邊界也有明確的 unsafe 區塊。

### OS frontend 與共用分析邊界

~~~mermaid
flowchart TD
    main["main: build target OS"] --> dispatch["frontend::run / OperatingSystem"]
    dispatch --> win["WindowsFrontend: eframe GUI"]
    dispatch --> mac["MacosFrontend: explicit not-implemented error"]
    dispatch --> linux["LinuxFrontend: explicit not-implemented error"]
    win --> jobs["TsanApp: background job + progress/cancel"]
    jobs --> service["AnalysisService: same interface for every frontend"]
    service --> analyzer["tsan-analyzer: shared TS logic"]
    analyzer --> report["AnalysisReport"]
    report --> result["Worker result channel"]
    result --> win
~~~

  - OS 由編譯目標自動判斷；同一份原始碼分平台編譯，不是在一個 Windows EXE 中載入其他 OS 的原生 GUI。
  - macOS／Linux frontend 已採相同介面並取得 AnalysisService，但目前在啟動時明確失敗，不假裝已顯示 GUI。實作時接入共用服務並呈現 AnalysisReport，不複製分析演算法。
  - native-tsduck 是可選增強；Windows GUI 啟用它，純 Rust Analyzer 可在其他平台獨立使用。共通掃描與資料定義相同，原生 bridge 提供的額外結果仍須逐平台啟用與驗證。
  - Cargo 的相對 dependency path 集中在 workspace.dependencies，子 crate 使用 workspace = true。相對於 Cargo.toml 的 example/test path 及 include_str! 的編譯期腳本位置仍是合理的原始碼依賴；執行時不依賴原始碼所在目錄。

### 分析流程

~~~mermaid
flowchart TD
    file["TS 檔案"] --> doc["每檔 AnalyzerDocument"]
    doc --> queue["1～4 個背景分析工作"]
    queue --> service["tsan-runtime: AnalysisService"]
    service --> probe["tsan-analyzer: 格式探測與批次讀取"]
    probe --> packets["每包取出 188-byte TS 核心"]
    packets --> rust["Rust 封包／影片／合規分析"]
    packets --> native["TSDuck session"]
    rust --> report["AnalysisReport：記憶體"]
    native --> report
    report --> views["Overview、PSI/SI、TR、Bitrate、時鐘、GOP"]
    report --> export["匯出"]
    doc --> packetview["Packets 頁面按需回讀原始封包"]
~~~

  - 分析容量依可用 CPU 數扣除兩個後限制為 1～4。尚未開始的工作依目前選取、已勾選比較、檔案大小排序；關閉文件會取消分析。
  - 4096 個輸入封包為一批。192-byte 前綴與 204-byte 尾碼不送入 188-byte 解析器；報告保留原始格式、起始偏移與封包位置。
  - 各文件保存獨立結果、PID／GOP 選取與縮放狀態。沒有磁碟分析快取；重啟不自動還原整個 session。
  - GOP 由影片 slice header 分析得到畫面類型，提供 Length、Structure、Program TS／VCL bytes；random-access indicator 不是 GOP Length 的替代品。
  - Bitrate 依 PCR 區間估計；GOP 的 packet position 與播放索引另有各自定義，見 本章的 GOP、Seek 與報告定義。
  - PSI／SI 名稱與標準來自觀察到的表格內容。VCT 或 delivery descriptor 只提供 signalling hint，不能證明實際 RF modulation、FEC、MER 或 BER。
  - 合規規則以 MPEG-2 TS 共通檢查為基礎，再套用 DVB、ATSC、ISDB、DTMB 等適用範圍；報告區分未量測、不適用與未實作。

### 播放流程

~~~mermaid
flowchart LR
    ui["選取 TS／播放控制"] --> worker["單一 Player worker"]
    worker --> session["tsan-player session"]
    file["TS 檔案"] --> index["video PES PTS 索引"]
    file --> reader["檔案讀取器"]
    index --> reader
    session --> reader
    reader --> appsrc["appsrc → queue → tsparse → tsdemux"]
    appsrc --> video["影片 parser → D3D decoder → video sink"]
    appsrc --> audio["decodebin3 → 音訊轉換 → wasapi2sink"]
    video --> snapshot["sample／segment 與播放狀態"]
    snapshot --> worker
    worker --> ui
~~~

  - 比較勾選控制 Analyzer；Player 只跟隨單一選取文件。切換 Player／檔案會載入對應 TS。
  - 索引用影片 PES PTS 建立，不以 PCR 換算 seek 位置。優先使用 random-access 標記，必要時退回 PES 入口；目前選第一個影片 PES PID。
  - appsrc 供應 TS 並支援 TIME seek；FLUSH／segment 配合解碼。連續 seek 保留最新要求，GUI 不把請求位置直接當作已呈現的影格位置。
  - 破損 PTS、缺失 random-access／參數集、多節目與特殊 interlaced／multilayer 檔案仍需各別驗證。

### 匯出與產品支援程式

~~~mermaid
flowchart LR
    report["AnalysisReport"] --> cbor["CBOR bytes"]
    cbor --> raw[".cbor"]
    cbor --> excel["xlsx.rs → .xlsx"]
    report --> bundle["主 .tex、sections、images"]
    bundle --> source["LaTeX source"]
    bundle --> engine["XeLaTeX"]
    engine --> pdf[".pdf 與 build 診斷"]
~~~

  - XLSX 直接讀記憶體中的 CBOR，不經過 LaTeX，也不需要 Excel；TeX／PDF 目前直接使用 AnalysisReport。
  - 報告匯出仍位於 tsan-gui，尚未抽成共用 report crate。
  - tsan-platform 提供 GUI 與 tsan-launcher 共用的部署模式與 OS 路徑規則。根目錄 scripts/windows/ 保存隨程式執行的更新 PowerShell 腳本；launcher 由 Cargo 建置，只啟動 GUI。CLI 分析與匯出入口尚未接入。
  - Rust 分析、輸入與播放功能由現有共用 crate 提供；未來報告 library 也應獨立於前端。scripts/ 不承擔一般分析或匯出業務邏輯。
  - 開發工具、ISS 範本與驗證在 developmentHelpers/；設定與快取位置、portable／安裝／更新差異見 [Chapter 1](CH1_DevelopmentFromSource.md)。

### GOP 定義與畫面

  - 從 PMT 判斷 H.264（0x1B）或 H.265（0x24），跨 TS／PES 邊界累積 NAL header，解析 slice type。每個 picture 的其他 slices 只累計 bytes，不重複計數。
  - GOP 以 I／IDR／CRA／BLA 開始，到下一個 intra picture 之前結束；這是 intra-to-intra 分組，不表示已證明為 closed GOP。Structure 使用 bitstream／decode order，並非 display order。
  - Length 單位為 coded pictures；I 為 intra、P 為 predicted、B 為 bidirectional。HEVC 只計 base layer。錄製起點缺少 PPS 等參數時顯示「?」，不猜測類型。
  - VCL bytes 包含壓縮 slice 與 NAL headers，不含 start codes、TS／PES overhead、非 VCL NAL（包括 filler）。GUI 可切換 Compressed VCL，圖表以 bytes 顯示；CBOR／XLSX 保留整數 bytes。
  - GOP Bytes 預設 Program TS：以該視訊唯一對應的 PMT program，計算 PAT、PMT、PCR 與全部 elementary-stream PIDs 的完整封包 bytes，含 headers／stuffing／音訊，不含 null PID 或其他節目。採輸入的 188／192／204-byte packet size。在遇到下一個 intra picture 時記錄結果，計入前後兩個邊界封包，因此相鄰 GOP 的數值不能直接加總；這個規則已明示在畫面、hover 與匯出欄位。找不到唯一 program 時保留 VCL，不猜測節目範圍。
  - 以 M25 原始封包核對：第一個完整 GOP 的 packet 9903–20273，共 3136 個 program packets，得到 589568 bytes；原本 VCL 為 404031 bytes。兩者是不同計量，不是 KiB 換算誤差。
  - 頭尾不完整、已偵測傳輸中斷或含未知 picture type 的 GOP 不列入完整 GOP 統計及圖表；列表保留全部資料。「完整」表示觀察到的邊界與 header 完整，並非解碼正確性認證。
  - 預設顯示 codec、pictures、完整 GOP 數、Length 最小／最大／平均及第一個完整 GOP。列表、Length、Bytes 分開切換；Structure 依欄寬換行。
  - 各比較欄位保持自己的 PID、GOP 選取與圖表範圍。欄內恢復水平間距，外層保留可拖曳分隔線；不足寬的表格可水平捲動。GOP 表格使用與 Packets 一致的交錯底色，欄位分隔線在 hover／拖曳時顯示；拖曳欄位標題右側調整欄寬，拖曳底部把手調整高度；長列表只繪製可見列。
  - GOP Length／Compressed VCL 的 X 軸為該 GOP 第一個 TS packet；Program TS 的 X 軸為下一個 GOP 開始的 packet。兩者皆是 zero-based packet number；每個點是一個完整 GOP。Y 軸分別為 pictures 與 bytes，bytes 刻度顯示整數。GOP 圖表依數據差值保留上下各 10% 空間，避免低變動量被絕對值比例的留白壓平；固定值才使用最小軸寬。GUI 與報告圖表採相同座標；單曲線不顯示 Reset Filter，仍可 Reset view。
  - 操作說明使用至少 16-point 的文字、正常前景對比及自動換行；不再以 Small／weak 顯示 GOP、PSI／SI、合規事件與圖表說明。
  - Bitrate 自動 Y 軸在堆疊曲線的總高度之外保留 10% 餘量；固定值仍有最小 padding。手動 zoom 保留使用者選取範圍。

### Seek 與時間軸

  - 載入時以 video PES PTS 建立時間／byte offset 索引，處理 PTS wrap 與有限的 B-picture 重排，不用 PCR 換算 seek byte position。
  - 索引優先用 video PES 的 random_access_indicator，沒有該標記時退回 PES positions。缺乏真正 random-access point／參數集的特殊 TS 不保證可立即解碼。
  - appsrc 使用 TIME segment。seek callback 以索引定位到前方入口，第一個 buffer 以入口的 DTS-relative position 標時；FLUSH 清除舊資料，decoder 從入口解碼，segment 控制目標前影格的呈現。
  - tsparse／tsdemux 忽略 PCR、關閉 skew corrections。PTS／DTS 必須仍可用；若 PTS 大幅跳躍而不能建立索引，不會虛構 duration／seek 時間。
  - 動態解碼器尚未提供第一張有效影格時保留 pending seek，避免事件在播放分支完成前遺失。連續拖曳保留最新 target，超時允許取代卡住的要求。
  - GUI position 優先讀 video sink 的 last-sample，經 sample 的 segment 轉為 stream time。目標時間不會直接冒充實際播放位置。
  - 目前索引選第一個 video PES PID。多節目切換、缺失 random-access flags、破損 PTS、特殊 interlaced／multilayer bitstream 仍需另備樣本，不能由本次單節目測試推論全部 TS 均可精準 seek。

### 報告資料流與分層

  - CBOR schema 5 是原始分析快照（新增 next GOP packet、program TS bytes、program number／PIDs；XLSX reader 仍接受 schema 4）；XLSX 直接 decode 相同 CBOR bytes，沒有 LaTeX／PDF 中介，也不需要 Excel 或 Python。
  - XLSX 分 Overview、PIDs、Programs、PSI SI、Clocks、GOP、Bitrate samples、Compliance、Events。第一列凍結、可篩選，數字保持 numeric，文字用 inline string 防止來源字串被當成公式。
  - Overview 保留完整來源路徑；明細以來源序號與檔名識別，避免同名檔案混淆。沒有有效 PCR 的 bitrate 留空。某個 sampling window 沒有足夠 PCR interval 時，使用該 PID interval rate 的 median，與 GUI 相同。
  - 超過 Excel 每 sheet 列數上限時分 sheet；超過 32,767 UTF-16 code units 的文字 cell 或 4 GiB ZIP entry 會明確報錯，應改存 CBOR，避免靜默截斷。
  - LaTeX／PDF 使用相同 bundle。圖表各自產生 SVG（獨立查看）與 PDF（供 LaTeX 引用），不需外部繪圖程式。
  - PDF 執行兩次 xelatex，關閉 shell escape，保留 source、分頁、images 及 build log；失敗也保留診斷資料。

~~~text
report.tex
report.pdf                       # 選 PDF 時
report_sections/
  Overview.tex
  PSI_SI_Tree.tex
  TR_101_290.tex
  Graphs.tex                     # 引入三個圖表分頁
  Bitrate.tex
  Timestamps.tex
  GOP.tex
  images/
    file_0_bitrate.svg
    file_0_bitrate.pdf
    file_0_gop_102_length.svg
    file_0_gop_102_length.pdf
    ...
  build/                         # 選 PDF 時
    report.pdf
    report.log
    report.aux
    report.toc
    ...
~~~


### Validation history and targeted tests

  - 以下是先前開發期間針對特定樣本的紀錄，不是每次 CI 都執行，也不代表新版本或所有廣播 TS 已驗證；本輪結果以當次 task 終端與輸出為準。

  - Rec 的 16 個 TS 全部分析及匯出 CBOR／XLSX／LaTeX。M25、V1 的完整 GOP Length 為 16；14 個 H.265 ATSC／QAM64／QAM256 為 60。
  - D3D12、D3D11 均逐檔檢查 30%、70%、15%、85% 四個跳轉位置，以 sink 的 sample／segment 比對目標，容許誤差 100 ms。另檢查 seek 後恢復播放時間持續前進。
  - M25 測試副本的 366 個 PCR 欄位固定為零，原檔保持不變；四個 seek 與恢復播放檢查通過。
  - 16 個 XLSX 的所有 XML 以獨立 ZIP／XML reader 驗證，並核對完整 GOP 數與 Length。另以試算表引擎匯入 H.264／H.265 workbook、渲染九張 sheet。
  - M25 PDF 實際以 XeLaTeX 編譯，檢查 GOP 圖表／表格。其他 15 個 LaTeX bundle 已生成，不代表全部 PDF 都經人工逐頁檢查。
  - 以下 ignored tests 需明確給本機路徑；原生 playback test 會開 video window。命令先使用 [Chapter 1](CH1_DevelopmentFromSource.md) 的手動 Cargo 環境。

~~~powershell
cargo test --offline --locked --workspace
cargo run --release --offline --locked -p tsan-analyzer --example gop_probe -- 'developmentHelpers/test-data/inputs/local'
$env:TSAN_TEST_TS_DIR = 'developmentHelpers/test-data/inputs/local'
cargo test --offline --locked -p tsan-player real_files_seek_to_presented_frames -- --ignored --nocapture
$env:TSAN_TEST_D3D11 = '1'
cargo test --offline --locked -p tsan-player real_files_seek_to_presented_frames -- --ignored --nocapture
Remove-Item Env:TSAN_TEST_D3D11
$env:TSAN_REPORT_TS_DIR = 'developmentHelpers/test-data/inputs/local'
$env:TSAN_REPORT_OUTPUT = 'outputs/windows/manual-reports'
cargo test --offline --locked -p tsan-gui export_recordings -- --ignored --nocapture
~~~

  - The recordings export test produces CBOR/XLSX/TeX for each selected recording and PDF for filenames starting with M25. It is not a generic PDF packaging recorder generator; use the explicit procedure in Chapter 1 for other representative files.

### Playback API references

  - [GStreamer Rust bindings](https://gstreamer.freedesktop.org/documentation/rust/stable/latest/docs/gstreamer/)
  - [Segments](https://gstreamer.freedesktop.org/documentation/additional/design/segments.html)
  - [Seeking](https://gstreamer.freedesktop.org/documentation/additional/design/seeking.html)
  - [appsrc](https://gstreamer.freedesktop.org/documentation/app/appsrc.html)
  - [tsdemux](https://gstreamer.freedesktop.org/documentation/mpegtsdemux/tsdemux.html)

## macOS

  - Shared parsers, input types, and analysis models are separated from GUI/native code, but the native application path is not implemented.
  - Window integration, playback sinks, file dialogs, runtime packaging, and update application need macOS implementations. A Windows package cannot provide them.
  - MacosFrontend and the updater return explicit macOS not-implemented errors. AnalysisService and the parser are shared; the CLI remains a placeholder.
  - Paths use ~/Library/Application Support/TS-Analyzer and ~/Library/Caches/TS-Analyzer. Missing home information is an error, not a Windows path fallback.
  - outputs/deploy/macos is reserved for future user packages.

## Linux

  - LinuxFrontend and the updater return explicit Linux not-implemented errors. Native Wayland/X11 windowing, playback sinks, file dialogs, packaging, and update application are not implemented.
  - Paths use XDG_CONFIG_HOME, XDG_CACHE_HOME, and XDG_STATE_HOME; missing, empty, or relative values fall back to ~/.config, ~/.cache, and ~/.local/state. Missing home information with no valid override is an error.
  - outputs/deploy/linux is reserved for future user packages.
  - Compositor differences belong at the platform boundary; transport parsing and report metrics should keep the same definitions.
  - Future validation must cover actual display protocols, GPU drivers, and runtime/ABI baselines. It must not infer compatibility from a compositor's name alone.

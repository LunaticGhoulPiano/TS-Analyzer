# GOP、播放時間與報告輸出

## 元件與環境

  - TSDuck：透過 C++ FFI 在同一個 process 分析 MPEG-TS／PSI／SI 與視訊 metadata，不會執行 tsp，也不負責播放。
  - Rust Analyzer：封包、連續計數、PCR／PTS／DTS、bitrate sampling、合規事件，以及 H.264／H.265 Annex-B slice header 的 GOP 分析。
  - GStreamer：appsrc → queue → tsparse → tsdemux，再分流至 H.264／H.265 parser、D3D12 或 D3D11 decoder／video sink。音訊經 decodebin3、audioconvert、audioresample、wasapi2sink。
  - XeLaTeX：只用於 PDF；CBOR、XLSX、LaTeX source 匯出不需執行 XeLaTeX。
  - 本次使用既有 Rust 1.97.1、GStreamer 1.28.7 MSVC x64、TSDuck SDK 與 TeX Live。未新增 Cargo dependencies、安裝工具或改全域 PATH。建置方式見 [env.md](env.md)。

## GOP 定義與畫面

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

## 多檔分析與選取

  - 檔案串流以 4096 個 packet 為一批讀取並呼叫 TSDuck FFI，保留逐封包的 Rust 檢查。依 CPU 保留兩個 logical cores、最多四個分析工作；等待中的檔案先選目前選取、已勾選比較、較小檔案。
  - 進度依已處理 bytes 顯示。關閉文件會取消該分析；不產生磁碟分析 cache。dev profile 的 parser 使用 opt-level 2，避免大量 TS 在完全未最佳化的 debug parser 上處理。
  - Player 頁面只跟隨藍色高光的單一 TS；比較勾選仍只控制 Analyzer 的比較欄。進入 Player 或改選 TS 會自動載入，不需另一個 Play selected TS 按鈕。切檔時若原本正在播放，載入後繼續播放；否則保持暫停。

## Seek 與時間軸

  - 載入時以 video PES PTS 建立時間／byte offset 索引，處理 PTS wrap 與有限的 B-picture 重排，不用 PCR 換算 seek byte position。
  - 索引優先用 video PES 的 random_access_indicator，沒有該標記時退回 PES positions。缺乏真正 random-access point／參數集的特殊 TS 不保證可立即解碼。
  - appsrc 使用 TIME segment。seek callback 以索引定位到前方入口，第一個 buffer 以入口的 DTS-relative position 標時；FLUSH 清除舊資料，decoder 從入口解碼，segment 控制目標前影格的呈現。
  - tsparse／tsdemux 忽略 PCR、關閉 skew corrections。PTS／DTS 必須仍可用；若 PTS 大幅跳躍而不能建立索引，不會虛構 duration／seek 時間。
  - 動態解碼器尚未提供第一張有效影格時保留 pending seek，避免事件在播放分支完成前遺失。連續拖曳保留最新 target，超時允許取代卡住的要求。
  - GUI position 優先讀 video sink 的 last-sample，經 sample 的 segment 轉為 stream time。目標時間不會直接冒充實際播放位置。
  - 目前索引選第一個 video PES PID。多節目切換、缺失 random-access flags、破損 PTS、特殊 interlaced／multilayer bitstream 仍需另備樣本，不能由本次單節目測試推論全部 TS 均可精準 seek。

## 報告資料流與分層

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

## 系統狀態存在哪裡

| 資料 | 位置／方式 | 重新啟動後 |
| --- | --- | --- |
| 最近開啟檔案 | %APPDATA%\TS-Analyzer\tsan-config.toml 的 recent_files array，最新在前，去重後最多 12 個 | 讀回 recent menu，不自動重開 |
| Theme、透明度、播放 backend | 同一 tsan-config.toml 的 theme、transparent_opacity、player_backend | 啟動時讀回 |
| 已開 TS、頁籤順序、比較勾選、欄寬 | documents／visible_documents／pane_widths 記憶體欄位 | 不保存 |
| 頁面、PID、GOP、zoom | 每個 document 的 ViewState | 不保存 |
| Log | LogEntry vector，含序號、Unix milliseconds、level、category、message | 不自動寫檔；Log 的 dump 功能可選路徑輸出 |
| 原始 TS／分析結果 | 原始 TS 只讀；分析結果在記憶體 | 選擇匯出才產生報告 |

  - 本機設定檔：C:\Users\USER\AppData\Roaming\TS-Analyzer\tsan-config.toml。尚無 TOML 時匯入同目錄 recent-files.txt，保留原 TXT。設定有變更才以暫存檔＋rename 更新；無效 TOML 會記錄錯誤並停止覆寫。
  - Import Recent Files ... → Clear Recent Files 立即清空 recent_files 並儲存 TOML，不會關閉已載入文件或刪除 TS；下次啟動仍為空清單。
  - schema 1 使用固定的 flat TOML 欄位；未知或格式錯誤的欄位不會靜默丟棄。支援 UTF-8 path、escaped basic string、literal string、multiline array 及 comments。
  - 不自動恢复整個 session 或寫入持久 Log。打包版 GStreamer／TeX cache 與更新暫存位於 %LOCALAPPDATA%\TS-Analyzer，詳見 distribution.md。

## 驗證

  - Rec 的 16 個 TS 全部分析及匯出 CBOR／XLSX／LaTeX。M25、V1 的完整 GOP Length 為 16；14 個 H.265 ATSC／QAM64／QAM256 為 60。
  - D3D12、D3D11 均逐檔檢查 30%、70%、15%、85% 四個跳轉位置，以 sink 的 sample／segment 比對目標，容許誤差 100 ms。另檢查 seek 後恢復播放時間持續前進。
  - M25 測試副本的 366 個 PCR 欄位固定為零，原檔保持不變；四個 seek 與恢復播放檢查通過。
  - 16 個 XLSX 的所有 XML 以獨立 ZIP／XML reader 驗證，並核對完整 GOP 數與 Length。另以試算表引擎匯入 H.264／H.265 workbook、渲染九張 sheet。
  - M25 PDF 實際以 XeLaTeX 編譯，檢查 GOP 圖表／表格。其他 15 個 LaTeX bundle 已生成，不代表全部 PDF 都經人工逐頁檢查。
  - 以下 ignored tests 需明確給本機路徑；原生 playback test 會開 video window。命令沿用 env.md 的 SDK 環境。

~~~bash
rtk cargo test --offline --locked --workspace
rtk cargo run --release --offline --locked -p tsan-analyzer --example gop_probe -- 'C:/Recordings'
TSAN_TEST_TS_DIR='C:/Recordings' rtk cargo test --offline --locked -p tsan-player real_files_seek_to_presented_frames -- --ignored --nocapture
TSAN_TEST_D3D11=1 TSAN_TEST_TS_DIR='C:/Recordings' rtk cargo test --offline --locked -p tsan-player real_files_seek_to_presented_frames -- --ignored --nocapture
TSAN_REPORT_TS_DIR='C:/Recordings' TSAN_REPORT_OUTPUT='C:/path/to/output' rtk cargo test --offline --locked -p tsan-gui export_recordings -- --ignored --nocapture
~~~

## 參考

  - [GStreamer Rust bindings](https://gstreamer.freedesktop.org/documentation/rust/stable/latest/docs/gstreamer/)
  - [Segments](https://gstreamer.freedesktop.org/documentation/additional/design/segments.html)
  - [Seeking](https://gstreamer.freedesktop.org/documentation/additional/design/seeking.html)
  - [appsrc](https://gstreamer.freedesktop.org/documentation/app/appsrc.html)
  - [tsdemux](https://gstreamer.freedesktop.org/documentation/mpegtsdemux/tsdemux.html)

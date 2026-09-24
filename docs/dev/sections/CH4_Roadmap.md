# CH4 — Roadmap

  - These are implementation or validation goals, not a list of completed features. Current architecture is described in [Chapter 2](CH2_ProjectStructure.md).

## Windows

### 共用功能與入口

  - GUI 與 CLI 共用的分析、輸入、播放及未來報告功能應由 Rust library crate 提供；前端負責參數、操作與呈現，不互相依賴另一個前端。
  - 部署與路徑由 tsan-platform 提供，Windows 啟動器位於 tsan-launcher；產品更新腳本放在根目錄 scripts/windows/。建置、驗證與 ISS 範本位於 developmentHelpers/。
  - tsan-cli 目前只有未實作提示。命令列分析需接入 tsan-runtime 的 AnalysisService；報告邏輯目前在 GUI，接入 CLI 前需抽成共用 library。
  - 完整 session、欄寬、圖表範圍與視窗配置保存仍待實作；設定遷移需有版本、錯誤處理及使用者資料保留測試。

### 即時輸入、分發與錄製

  - UDP unicast／multicast、指定網卡及 RTP 接收仍待接入正式流程；驗證開始、停止、取消、重新 join 與錯誤後恢復，避免遺留 worker。
  - Analyzer、Recorder、Player 使用各自有界的 queue，明確記錄容量、丟棄策略、overflow 與時間；UI 接收 snapshot，不阻塞封包處理。
  - RTP 測試需涵蓋 sequence wrap、missing、duplicate、reorder、CSRC、extension、padding 與損壞 header。UDP 無足夠證據時不推定 datagram loss。
  - 原始錄製保留收到的 TS packet width、payload 與到達順序；RTP 只拆除封裝。正規化輸出須另外命名，不能混同原始錄製。
  - Ring buffer 同時限制 bytes 與保留時間；錄製需有容量估算、磁碟錯誤、未完成檔案處理及版本化 sidecar，metadata 不混入 TS payload。
  - 分開記錄網路、RTP、TS、應用 queue、磁碟與 decoder 錯誤，避免將本機阻塞誤報為傳輸端問題。

### 解析與播放

  - AAC metadata 需補齊 ADTS／LATM／ASC 及 AAC-LC、HE-AAC 的 SBR／PS 辨識；以合法、截斷、跨 PES 邊界及未知 profile 樣本驗證，不以固定名稱推測。
  - 持續補齊各標準的 table／descriptor context、版本、完整性及交叉引用；未知欄位保留原始資料與來源位置，不猜測其標準。
  - H.264／H.265 的多節目、interlaced、multilayer、缺失參數集與損壞時間戳仍需擴充驗證；profile、level、色深及 HDR metadata 必須有解析依據。
  - 音訊裝置切換、WASAPI 初始化失敗、視訊裝置遺失、sleep／resume 與視窗生命週期需有恢復測試；fallback 策略要配合實際打包元件驗證。
  - appsrc 已設定 32 MiB 緩衝；後續依量測調整，而非再次把增大緩衝列為未完成的功能。
  - 播放驗收矩陣涵蓋 1080p60、4K30／60、H.264、H.265 Main／Main10、D3D11／D3D12 與各 GPU 廠牌；記錄實際 decoder、caps、色深與已測限制。

### 測試與發行

  - 增加 parser 的任意資料、截斷、極端長度、版本切換及 timestamp wrap 測試，確認沒有 panic、hang、overflow 或無界配置。
  - 故障注入涵蓋 sync／CC／CRC、RTP、queue overflow、慢磁碟、Player／UI stall；驗證錯誤歸屬與恢復行為。
  - 100 Mbps 持續處理、兩小時逐 byte 相同的錄製及 8～24 小時資源測試保留為目標；記錄 CPU、RAM、GPU、queue、thread、handle 與磁碟容量，未量測前不宣稱通過。
  - 測試資料需有來源、授權、SHA-256 與 expected；大型或不可再散布素材放 local 或外部目錄，正式產品不依賴測試素材。
  - 指標比對須先統一 packet width、時鐘、取樣窗口、單位與容許誤差；無法解釋的差異保留為未解決，不直接選一份輸出作標準。
  - Windows 10／11 乾淨環境、GPU／驅動、Unicode／長路徑與 DPI 需有相容性紀錄；公開版本間更新另作端到端驗證。
  - 補齊 SBOM、簽章流程及相依元件清單的自動驗證；發行包只帶執行所需元件及授權，不加入編譯器或開發測試資料。

### 後續範圍

  - RF／ASI、BDA／Linux DVB 與硬體 adapter 保留為後續階段，透過輸入介面及 capability metadata 擴充。
  - RF 的 MER／BER／SNR／level 等量測需來自可用硬體，不由一般 TS 檔案推算。
  - 外部 plugin 的 ABI／程序通訊、ATSC 3.0、轉碼、多路即時監控及長期監控服務另行設計，不因既有占位而宣稱完成。

## macOS

  - 原生視窗、影片與音訊、檔案對話框、主題、runtime 封裝、簽章與更新仍待實作。
  - 保留獨立平台版本；共用分析及未來報告 library 應維持一致資料定義。

## Linux

  - Wayland／X11 原生視窗、播放、對話框、封裝及更新仍待實作；需要選定 ABI／發行基線及驗證 GPU 後端。
  - 驗證 Hyprland、Niri 等 compositor 的差異，避免把 WM 名稱當成另一個解析器或檔案格式。
  - 保留獨立平台版本與同一更新入口，平台模組負責套用更新。

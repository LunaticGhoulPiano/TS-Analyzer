# TS Analyzer 專案架構

## 1. 文件目的

    本文件定義 TS Analyzer 的 repository 目錄、Rust crate 邊界、依賴方向、執行期資料流、設定責任、僅供開發使用的測試資料，以及 release 封裝邊界。

    第一階段的目標平台是 Windows 11 x64，實作語言為 Rust 2024 Edition，編譯目標為 `x86_64-pc-windows-msvc`。分析器由本 repository 自行實作；EasyICE、DVB Inspector、TSDuck 及兩個已 clone 的原始碼 repository 都不是執行期依賴。

## 2. 架構邊界

  - 擷取、分析、錄製、播放與 UI 是互相獨立的 consumer。
  - 播放器或 UI 變慢時，不得無記錄地阻塞分析或錄製。
  - 錄製路徑在 packet normalization 前接收保留來源內容的資料。
  - 分析與播放接收已正規化的 188-byte TS packet，並保留來源 metadata。
  - 每個 queue 都必須有容量上限，並公開容量、使用量、overflow 次數與 overflow policy。
  - Parser 不得依賴 GUI widget、GStreamer、socket 或檔案輸出。
  - 正式程式不得載入 `dev_tests_data`，也不得呼叫 `validation_tools` 來完成分析。
  - 未知或損壞的輸入必須表示為資料或事件，不得造成 panic。

## 3. 執行期資料流

```text
File / UDP / RTP Input
          │
          ▼
tsan-input
接收來源資料、驗證 RTP 並移除 RTP 封裝
          │
          ▼
IngressBatch：來源 TS payload + arrival／RTP metadata
          │
          ├──────────────► tsan-recorder：保留來源內容的 ring buffer
          ├──────────────► tsan-recorder：raw recorder + sidecar metadata
          │
          ▼
tsan-core
Packet size 偵測／sync recovery／normalization
          │
          ▼
已正規化的 188-byte PacketBatch
          │
          ▼
tsan-runtime
Bounded dispatcher + session lifecycle
     ┌────┼───────────────────┐
     ▼    ▼                   ▼
tsan-analyzer             tsan-player          Runtime metrics
     │                         │                      │
     └─────────────────────────┴─────────┬────────────┘
                                         ▼
                               唯讀 snapshot + events
                                         │
                                ┌────────┴────────┐
                                ▼                 ▼
                            tsan-cli           tsan-gui
```

    `tsan-input` 產生的 `IngressBatch` 同時保留 TS payload 與來源 metadata。錄製路徑直接使用此資料；分析及播放路徑則先交由 `tsan-core` 偵測 188／192／204-byte 格式、恢復同步並正規化成 188-byte packet。

## 4. 儲存庫目錄

```text
TS-Analyzer/
├── Cargo.toml
├── Cargo.lock
├── rust-toolchain.toml
├── rustfmt.toml
├── deny.toml
├── LICENSE
├── crates/
│   ├── tsan-core/
│   ├── tsan-input/
│   ├── tsan-analyzer/
│   ├── tsan-runtime/
│   ├── tsan-recorder/
│   ├── tsan-player/
│   ├── tsan-cli/
│   └── tsan-gui/
├── config/
│   └── tsan.example.toml
├── dev_tests_data/
│   ├── inputs/
│   │   ├── synthetic/
│   │   ├── redistributable/
│   │   ├── local/
│   │   ├── manifest.template.toml
│   │   └── manifest.toml
│   └── expected/
│       ├── transport/
│       ├── analyzer/
│       ├── tables/
│       ├── clocks/
│       └── tr101290/
└── docs/
    ├── dev/
    │   ├── env.md
    │   ├── test_data.md
    │   └── structure.md
    ├── metrics.md
    ├── json-schema.md
    ├── validation_tools.md
    ├── test-plan.md
    └── third-party-licenses.md
```

    `config`、多數文件及 `dev_tests_data` 是預定路徑，由第一個需要它們的 batch 建立。Git 不必保留空目錄。

## 5. Rust Crate 依賴方向

```text
tsan-cli ─────┐
              ├──► tsan-runtime
tsan-gui ─────┘        │
                       ├──► tsan-input ──────┐
                       ├──► tsan-analyzer ───┤
                       ├──► tsan-recorder ───┼──► tsan-core
                       └──► tsan-player ─────┘
```

    箭頭由 consumer 指向允許依賴的 crate，規則如下：

  - `tsan-core` 不依賴其他 workspace crate。
  - `tsan-input`、`tsan-analyzer`、`tsan-recorder` 與 `tsan-player` 可以依賴 `tsan-core`。
  - `tsan-runtime` 組合四個 service crate，並負責其 lifecycle。
  - `tsan-cli` 與 `tsan-gui` 只依賴 runtime-facing API，不重複實作 parser 邏輯。
  - 較低層 crate 不得依賴 `tsan-runtime`、`tsan-cli` 或 `tsan-gui`。

## 6. Rust Crate 目錄與責任

### 6.1 `tsan-core`

    負責 transport 層、重視 allocation 成本的 domain type 與 parsing primitive。

```text
crates/tsan-core/src/
├── lib.rs
├── batch.rs
├── error.rs
├── event.rs
├── transport/
│   ├── mod.rs
│   ├── packet.rs
│   ├── adaptation_field.rs
│   ├── packet_size.rs
│   └── sync.rs
└── time/
    ├── mod.rs
    ├── pcr.rs
    └── pts.rs
```

    主要型別包括 `IngressBatch`、`PacketBatch`、`PacketView`、`PacketFormat`、`SourceMetadata`、`AnalysisEvent`、`Pid`、`ProgramNumber`、`PcrTicks` 與 `PtsTicks`。

    此 crate 不得擁有 network socket、blocking file writer、GStreamer object、GUI model 或 session thread。

### 6.2 `tsan-input`

    負責來源擷取與來源專屬 metadata。

```text
crates/tsan-input/src/
├── lib.rs
├── source.rs
├── file.rs
├── udp.rs
├── multicast.rs
└── rtp.rs
```

    `InputSource` 公開來源能力並產生有容量上限的 batch。UDP datagram 沒有更高層 sequence 機制時，其遺失狀態保持 unknown；RTP sequence loss、reordering、duplication 與 malformed packet 必須和 TS continuity error 分開回報。

### 6.3 `tsan-analyzer`

    負責由已正規化 TS packet 推導出的所有分析。

```text
crates/tsan-analyzer/src/
├── lib.rs
├── pid.rs
├── bitrate.rs
├── continuity.rs
├── clocks/
├── section/
├── tables/
├── descriptors/
├── pes/
└── tr101290/
```

    分析狀態在內部可變，但發布的 snapshot 必須唯讀。未知 Table 與 Descriptor 必須保留 raw bytes 及 parsing context。此 crate 不解碼影音 frame，也不把 GStreamer 結果當成分析真值。

### 6.4 `tsan-runtime`

    負責應用程式協調與有界限的 concurrency。

```text
crates/tsan-runtime/src/
├── lib.rs
├── config.rs
├── session.rs
├── dispatcher.rs
├── queue.rs
├── command.rs
└── snapshot.rs
```

    Session 狀態定義為 `Idle`、`Starting`、`Running`、`Stopping` 與 `Failed`。只有此 crate 能組合 input、analyzer、recorder 與 player lifecycle。UI 只接收 snapshot 並送出 command，不直接接收 raw packet。

### 6.5 `tsan-recorder`

    負責保留來源內容的錄製與有界限的歷史資料。

```text
crates/tsan-recorder/src/
├── lib.rs
├── raw.rs
├── ring.rs
└── sidecar.rs
```

    Raw recording 不得改寫 PID、continuity counter、packet 順序或 payload。對 RTP input，保留來源內容的 TS recording 只移除 RTP envelope；已解析的 RTP header 欄位與 datagram boundary 保留於 sidecar metadata。Sidecar 也記錄來源時間、sync event、queue drop 與 recording gap。

### 6.6 `tsan-player`

    負責 `PlayerBackend` abstraction 與 GStreamer 實作。

```text
crates/tsan-player/src/
├── lib.rs
├── backend.rs
└── gstreamer.rs
```

    GStreamer 必須隔離在 backend boundary 後方，優先使用 D3D12，D3D11 作為 fallback。播放器缺少元件或執行失敗時，不得停止 analyzer 或 recorder。

### 6.7 `tsan-cli`

    負責 command-line parsing、人類可讀輸出、JSON 輸出選擇、環境診斷及僅供開發使用的測試 command。所有 transport 與 analysis 邏輯都委派給 library crate。

### 6.8 `tsan-gui`

    負責 `eframe`／`egui` view 與 command submission。

```text
crates/tsan-gui/src/
├── main.rs
├── app.rs
└── views/
```

    GUI 不直接讀取 socket、解析 TS packet、寫入 recording，也不得在 UI thread 執行 blocking player call。

## 7. 設定模型

    執行期設定使用有型別的 Rust structure，不使用未定型的 key/value 或 pointer-based option。

```text
AppConfig
├── SourceConfig
│   ├── FileConfig
│   ├── UdpConfig
│   └── RtpConfig
├── AnalysisConfig
├── QueueConfig
├── RecorderConfig
├── PlayerConfig
└── OutputConfig
```

    設定優先順序如下：

```text
CLI option
    ↓
Session config
    ↓
User config
    ↓
Compiled defaults
```

    Windows 上的儲存位置如下：

| 資料 | 位置 |
|---|---|
| 納入版控的設定範例 | `config/tsan.example.toml` |
| 使用者設定 | `%APPDATA%\TS-Analyzer\config.toml` |
| Log 與 cache | `%LOCALAPPDATA%\TS-Analyzer\` |
| Recording 與 JSON export | 使用者明確選擇的輸出路徑 |
| 開發期 `validation_tools` 路徑 | 環境變數或被 Git 忽略的本機開發設定 |

    除非使用者明確選擇，應用程式不得將 analysis JSON 寫在輸入檔旁。設定檔必須包含 schema version 與 migration policy；使用者設定損壞時，回復安全預設值並顯示明確警告。

## 8. 執行期資料儲存

| 資料 | 負責 crate | 保存期間 |
|---|---|---|
| 來源 bytes | `tsan-input` | 僅限 batch lifetime |
| 已正規化 packet | `tsan-core`／`tsan-runtime` | 僅限 bounded queue lifetime |
| Analyzer mutable state | `tsan-analyzer` | Session lifetime |
| 唯讀 snapshot | `tsan-runtime` | 最新且數量有上限的 snapshot |
| Event history | `tsan-runtime`／export layer | 有界限的 memory，可選擇匯出 JSON |
| 保留來源內容的 ring | `tsan-recorder` | Bytes 與 duration 都有上限 |
| Raw recording | `tsan-recorder` | 使用者選擇的檔案 |
| Recording sidecar | `tsan-recorder` | Recording 旁的 versioned JSON |
| Player state | `tsan-player` | Session lifetime |

    Live input 不允許大型且無上限的 collection。任何保留的歷史資料都必須宣告最大 byte 數、duration、item 數，或同時宣告三者。

## 9. 開發測試資料

    `dev_tests_data` 只存在於原始碼開發環境與 CI，不會由正式 binary 安裝、載入或依賴。

```text
dev_tests_data/
├── inputs/
│   ├── synthetic/          # 由 Rust 確定性產生的輸入
│   ├── redistributable/    # 授權允許提交的真實輸入
│   ├── local/              # 大型或不可再散布的輸入；由 Git 忽略
│   └── manifest.toml
└── expected/               # 正規化後的預期結果
    ├── transport/
    ├── analyzer/
    ├── tables/
    ├── clocks/
    └── tr101290/
```

    `manifest.template.toml` 只定義新增案例時使用的欄位格式，不由測試程式讀取。`manifest.toml` 只保存 input 與 expected 都已存在的實際案例；其用途、生命週期、欄位及命名規則定義於 [test_data.md](./test_data.md)。

    Rust integration test 應放在各 package 下，例如：

```text
crates/tsan-core/tests/
crates/tsan-input/tests/
crates/tsan-analyzer/tests/
crates/tsan-runtime/tests/
crates/tsan-cli/tests/
```

    Virtual workspace root 本身不是 Rust package，因此不使用 root-level Rust integration test。Test helper 可以從 workspace root 定位 `dev_tests_data`，但正式 module 不得公開它的路徑或形成對它的依賴。

## 10. 預期結果與驗證工具

    `validation_tools` 是開發期以人工方式操作、用來確認 expected data 的外部工具，不假設它們提供 API，也不納入自動化測試流程。指定版本如下：

| 驗證工具 | 版本 | SHA-256 |
|---|---:|---|
| EasyICE `EasyICE.exe` | 2.7.0.2 | `73821b0263073040b877d3737ae603fc2b8a2b5b2357b8ea36ad1e8121d486b9` |
| DVB Inspector `DVBinspector-1.21.0.jar` | 1.21.0 | `d54813816eb5b8aeaf41da3cfda89f9b2ae950a1b582712fae821c90a34f3c7a` |

    不提交指向個別開發者 Downloads 目錄的絕對路徑。本機路徑由此任務專用的環境變數或被忽略的開發設定解析；使用驗證工具人工確認 expected 前必須驗證其 SHA-256。

    Synthetic input 的 expected 由可重現的產生規格決定，不強制使用驗證工具。真實 input 才由開發者人工查看或匯出驗證工具結果，將已確認的欄位正規化為 TS Analyzer 的 expected schema。原始中間輸出不形成第三個永久 dataset。若 metric 只在 unit、time base、sample window 或 tolerance 上不同，expected schema 必須記錄正規化後的定義；若相同定義仍有衝突，則保留為未解決項目，在解決前不得納入 release gate。

    下列原始碼 clone 只供唯讀架構研究：

```text
C:\Users\USER\Documents\GitHub\dvbinspector
C:\Users\USER\Documents\GitHub\libeasyice
```

    不從這些 repository 複製、翻譯、連結或納入原始碼；其目前 Git revision 也不能取代固定版本的驗證工具 binary。

## 11. 發行封裝邊界

    Release 以 allowlist 封裝，可包含：

  - `tsan-cli.exe` 與 `tsan-gui.exe`。
  - 已核准的 runtime DLL。
  - 通過授權 allowlist 的 GStreamer runtime 與 plugin。
  - `LICENSE`、第三方授權聲明與使用者文件。

    Release 必須排除：

  - `dev_tests_data`。
  - EasyICE 與 DVB Inspector 驗證工具 binary。
  - TSDuck 開發工具。
  - DVB Inspector 與 libeasyice 原始碼 clone。
  - Cargo target 目錄、本機設定、log、capture 與驗證工具中間輸出。

    Release process 不得直接將整個 repository 封裝成產品套件。

## 12. 架構驗證

    每個實作 batch 必須執行其中相關的檢查：

```bash
rtk cargo fmt --all -- --check
rtk cargo build --workspace --locked
rtk cargo test --workspace --locked
rtk cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
rtk cargo deny check
```

    架構 review 必須拒絕 dependency cycle、正式程式存取 `dev_tests_data`、無界限的 live-state collection、parser 對 GUI 的依賴、analyzer 對 player output 的依賴，以及未記錄的 queue data loss。

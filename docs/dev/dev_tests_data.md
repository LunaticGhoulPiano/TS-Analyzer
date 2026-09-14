# `dev_tests_data`: 開發用的 local tests data

`dev_tests_data` 是 TS Analyzer 的開發期測試資料集。自動化測試以固定 input 執行分析，再把實際結果和對應的 expected 比較，用來發現 parser、分析邏輯或比對輸出格式。

這個資料集只供開發環境與 CI 使用，不應該被打包到 release。

## 1. 分類

- `template`: 格式模板
- `manifest`: 實際設定

| 路徑 | 責任 | 是否由測試讀取 |
|---|---|---|
| `inputs/manifest.template.toml` | 定義新增案例時要遵循的通用欄位格式 | 否 |
| `inputs/manifest.toml` | 列出目前真正存在且可執行的測試案例 | 是 |
| `inputs/synthetic/` | 保存由本專案確定性產生的 input | 是 |
| `inputs/redistributable/` | 保存具有明確散布授權的真實 input | 是 |
| `inputs/local/` | 保存不可提交的大型、私有或授權不明 input | 僅本機測試 |
| `expected/` | 保存每個 input 對應的結構化預期結果 | 是 |
| `crates/*/tests/` | 保存讀取 manifest、執行分析及比對 expected 的 Rust 測試程式 | 是 |

## 3. 步驟

1. 決定驗證哪一個主要行為
2. 依照 `manifest.template.toml` 選擇必要欄位
3. 建立 input 與對應的 expected。
4. 計算 input 大小及 SHA-256。
5. 確認 input、expected 都已存在後，才把案例加入 `manifest.toml`
6. Dataset integrity test 驗證路徑、大小、SHA-256 與 expected 對應
7. 各 crate 的 integration test 以 `case_id` 載入案例並比對分析結果

## 4. `case_id` 命名格式

```text
<domain>_<expected_behavior>_<condition>
```

   - 使用小寫 ASCII、數字與底線
   - 第一段表示被測領域，例如 `transport`、`continuity`、`psi`、`clock`、`rtp` 或 `tr101290`
   - 第二段表示分析器應有的行為，例如 `detect_packet_size`、`recover_sync`、`parse_pat` 或 `report_missing_packet`
   - 最後一段表示使案例彼此不同的條件，例如 `188`、`after_offset`、`single_program` 或 `bad_crc`
   - 不加入專案名稱、開發階段或資料所在目錄名稱
   - 不把與主要驗證目標無關的 TS 內容塞進名稱
   - Input 與 expected 檔名必須和 `case_id` 相同，只更換副檔名

正確範例：

```text
transport_detect_packet_size_188
transport_recover_sync_after_offset
continuity_report_missing_packet
psi_parse_pat_single_program
psi_reject_pat_bad_crc
```

## 5. Manifest 欄位

| 欄位 | 必要性 | 意義 |
|---|---|---|
| `manifest_version` | 必要 | `manifest.toml` 自身的格式版本 |
| `case_id` | 必要 | 符合命名規則的穩定案例識別名稱 |
| `description` | 必要 | 簡短說明此案例唯一的主要驗證行為 |
| `input_file` | 必要 | 相對於 `inputs/` 的 input 路徑 |
| `input_sha256` | 必要 | 已提交 input 的內容指紋 |
| `input_size_bytes` | 必要 | Input 的確切 byte 數 |
| `packet_size_bytes` | 必要 | TS packet 大小，例如 188、192 或 204 |
| `transport` | 必要 | 測試所模擬的輸入方式，例如 `file`、`udp` 或 `rtp` |
| `license` | 必要 | Input 的 SPDX license expression |
| `run_in_ci` | 必要 | 此案例是否為 CI 的必要案例 |
| `standards` | 選用 | 此案例涉及的 MPEG-TS、DVB、ATSC 或 ISDB 標準 |
| `multiplex` | 選用 | `spts` 或 `mpts` |
| `programs` | 選用 | 已知且與測試有關的 program number |
| `codecs` | 選用 | 已知且與測試有關的 codec |

## 6. Expected 欄位

Synthetic input 使用 `generated_specification`，expected 直接由產生該 input 的規格決定。真實 input 經 EasyICE 或 DVB Inspector 人工確認時使用 `manual_validation`；validation tool 的版本、SHA-256、操作方式及實際確認範圍寫在開發文件或 expected metadata，不放進 input manifest。

| 欄位 | 意義 |
|---|---|
| `category` | Expected 所屬的 `transport`、`analyzer`、`tables`、`clocks` 或 `tr101290` 類別 |
| `file` | 相對於 `inputs/` manifest 的 expected 路徑 |
| `schema_version` | Expected JSON 格式版本 |
| `basis` | `generated_specification` 或 `manual_validation` |

## 7. SHA-256 與私人資料

- 已提交的 synthetic 或 redistributable input 必須在 manifest 保存 SHA-256。
- SHA-256 用來確認 input 沒有改變，不代表分析結果正確。
- 私有或不可散布的 input 及其 SHA-256 不得寫入公開 manifest。
- EasyICE 與 DVB Inspector binary 的 SHA-256 不屬於 input manifest。

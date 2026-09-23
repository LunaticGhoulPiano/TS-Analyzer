# 從原始碼建置

## 需求

  - Windows 11 x64、Visual Studio C++ Build Tools（MSVC 與 Windows SDK）。
  - Rust `1.97.1`／`x86_64-pc-windows-msvc`；repository 的 `rust-toolchain.toml` 已固定版本。
  - GStreamer **MSVC x64** runtime 與 development 套件，版本至少 `1.28`。編譯時需能找到 GStreamer 的 `pkg-config` 資訊；執行時需能找到 DLL 與 plugins。
  - TSDuck x64 SDK（headers、`tsduck.lib`／`tscore.lib` 與 DLL）。編譯預設在 `C:\\Program Files\\TSDuck` 尋找；也可設定 `TSDUCK_HOME`。VS Code C/C++ 擴充套件使用 `.vscode/c_cpp_properties.json`；預設以 `ProgramFiles` 定位 TSDuck，自訂安裝位置則由啟動 VS Code 的環境提供 `TSDUCK_HOME`。
  - 首次編譯需能下載 Cargo 套件及 workspace 指定的 Git 版 `winit`。

## 建置與啟動

  - 以下命令在 repository 根目錄執行；不需重建 crate 或 lockfile。

```bash
rustup toolchain install 1.97.1 --profile minimal --target x86_64-pc-windows-msvc
```

  - 以下命令使用 Git Bash；GStreamer 實際安裝位置不同時請修改 `gst_root`。

```bash
gst_root=/c/Program\ Files/gstreamer/1.0/msvc_x86_64
tsduck_root=/c/Program\ Files/TSDuck
export TSDUCK_HOME="C:\\Program Files\\TSDuck"
export PATH="$gst_root/bin:$tsduck_root/bin:$PATH"
export PKG_CONFIG_PATH="$gst_root/lib/pkgconfig${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
```

```sh
rustc --version
cargo --version
pkg-config --modversion gstreamer-1.0
gst-inspect-1.0 --version
cargo metadata --locked --no-deps --format-version 1
cargo build --locked --workspace
cargo run --locked -p tsan-gui
```

  - 上面的 `export` 僅對該 Git Bash 工作階段有效。若 TSDuck 不在預設位置、且 VS Code 不是從該工作階段啟動，請將 `TSDUCK_HOME` 設為 Windows 使用者環境變數後重新啟動 VS Code；C++ IntelliSense 才能找到 TSDuck 標頭。`bridge.cpp` 只使用標頭名稱（例如 `#include "tsTSPacket.h"`），不在 `#include` 中放絕對或跨目錄相對路徑。
  - 若 GStreamer 探測失敗，確認 development 套件的 `lib/pkgconfig` 可由 `PKG_CONFIG_PATH` 找到；若啟動時缺 DLL 或 plugin，確認相同 MSVC x64 安裝的 `bin` 在 `PATH`。不要混用 MinGW 與 MSVC 版本。
  - Analyzer 預設連結進程內 TSDuck，不會啟動 `tsp`。只建置不含原生 TSDuck 的 Analyzer 可用 `cargo build --locked -p tsan-analyzer --no-default-features`；此模式沒有 TSDuck 的標準／section 統計及 SPS／VUI 幀率。
  - 上述是開發者的建置需求；一般使用者從完整 Windows package 啟動不需要 Rust、MSVC compiler、Windows SDK、pkg-config 或 development headers。打包腳本與私有 runtime、安裝及更新方式見 [distribution.md](distribution.md)。Linux／macOS 的完整建置／播放／安裝 backend 尚未驗證。

## 驗證

```sh
cargo fmt --all -- --check
cargo test --locked --workspace
cargo run --locked -p tsan-player --example gstreamer_smoke
```

  - smoke example 會檢查 GStreamer 版本、必要元素及 D3D12／D3D11 adapter；通過不代表所有 TS 檔案或 seek 行為都已驗證。

## 分析、播放器、匯出與系統狀態

  - 詳見 [analysis-player-reports.md](analysis-player-reports.md)，包含 GOP、seek、報告分層及 recent／theme／log 保存方式。

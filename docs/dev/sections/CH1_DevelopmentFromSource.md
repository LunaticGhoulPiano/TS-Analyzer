# CH1 — Development from Source

## Windows

### Required tools

  - Target: Windows 10 22H2 / Windows 11 x64. Compatibility must still be checked on the intended OS, GPU, and driver.
  - [Git](https://git-scm.com/downloads) to obtain the repository and the pinned Git dependency.
  - [Rustup](https://rustup.rs/), with the version and components in [rust-toolchain.toml](../../../rust-toolchain.toml). The current pin is Rust 1.97.1, rustfmt, and clippy. Use the x86_64-pc-windows-msvc toolchain on Windows; the repository does not force a Windows target on other hosts.
  - [Visual Studio Build Tools](https://visualstudio.microsoft.com/downloads/): Desktop development with C++, the MSVC x64 tools, and a Windows SDK. The TSDuck bridge uses C++20.
  - [GStreamer](https://gstreamer.freedesktop.org/download/): MSVC x64 runtime and development files, at least 1.28. Keep their versions aligned; do not substitute MinGW packages. The development environment must provide pkg-config.exe and gstreamer-1.0.pc.
  - [TSDuck](https://tsduck.io/): x64 SDK with headers under include/, import libraries under lib/Release-Win64/, and tscore.dll/tsduck.dll under bin/.
  - [PowerShell 7](https://learn.microsoft.com/en-us/powershell/scripting/install/installing-powershell-on-windows) for development tasks. Product update scripts use the Windows-provided PowerShell 5.1.
  - [VS Code](https://code.visualstudio.com/) is optional. The repository tasks work through PowerShell without the editor; rust-analyzer and the C/C++ extension provide editor assistance.
  - For PDF export from source and runtime packaging: [TeX Live](https://www.tug.org/texlive/) with XeLaTeX and the generated report's macro/font dependencies.
  - For an installer EXE: [Inno Setup](https://jrsoftware.org/isdl.php), providing ISCC.exe. It is a packaging compiler, not a Rust dependency or an end-user requirement.
  - Optional dependency-policy check: cargo-deny using developmentHelpers/config/deny.toml. It is not installed by the task entry point.

  - Install prerequisites deliberately. Tasks do not install tools, download missing Cargo dependencies, or change system environment variables.
  - The package contains only the required GStreamer/TSDuck/MSVC/XeLaTeX runtime files and licenses, not the SDKs, Rust compiler, or full TeX installation.

### 1. Obtain the source and prepare dependencies

  - Run the following in PowerShell. After cloning, all commands in this chapter assume the repository root unless a block explicitly changes directory.

~~~powershell
git clone https://github.com/LunaticGhoulPiano/TS-Analyzer.git
Set-Location TS-Analyzer
rustup toolchain install 1.97.1 --profile minimal --component rustfmt --component clippy --target x86_64-pc-windows-msvc
cargo fetch --locked
~~~

  - Use the pin from rust-toolchain.toml if it has changed. The explicit setup commands may download the toolchain and Cargo dependencies.
  - Subsequent development tasks use --locked --offline. If Cargo.lock changes, run cargo fetch --locked again before the offline tasks.
  - Cargo.toml centralizes internal crate paths and shared external dependencies. It also contains native feature choices, development optimization, and the pinned winit Git override. Cargo.lock records the resolved dependency versions.

### 2. Check tool paths and optional local configuration

~~~powershell
pwsh -NoProfile -File .vscode/windows/Invoke-Development.ps1 -Action CheckEnvironment
~~~

  - CheckEnvironment prints discovered tool locations and whether they exist. It does not install or repair tools, or prove that a complete build will succeed.
  - Discovery uses environment variables, PATH, standard installation locations, and vswhere where applicable.
  - If a tool is not found or a different installation is needed, create the ignored local configuration and edit its values:

~~~powershell
if (-not (Test-Path -LiteralPath developmentHelpers/config/windows.local.psd1)) {
    Copy-Item -LiteralPath developmentHelpers/config/windows.example.psd1 -Destination developmentHelpers/config/windows.local.psd1
}
~~~

| Setting in windows.local.psd1 | Environment override | Purpose |
| --- | --- | --- |
| GStreamerRoot | GSTREAMER_1_0_ROOT_MSVC_X86_64 | MSVC x64 SDK root. |
| TSDuckRoot | TSDUCK_HOME | TSDuck SDK root. |
| TexRoot | TSAN_DEV_TEX_ROOT | TeX Live root containing bin/windows/xelatex.exe. |
| MsvcRuntime | TSAN_DEV_MSVC_RUNTIME | x64 MSVC CRT directory; Package can locate it through vswhere. |
| MsvcLicenses | TSAN_DEV_MSVC_LICENSES | License directory for that runtime; discovered from its installation root or supplied for a relocated SDK. |
| InnoCompiler | ISCC_EXE | ISCC.exe file. |
| TexRecorder | TSAN_DEV_TEX_RECORDER | Complete representative report's .fls from the selected TeX installation. |
| Recordings | TSAN_TEST_RECORDINGS | Local TS directory; defaults to developmentHelpers/test-data/inputs/local. |
| OutputDirectory | TSAN_DEV_OUTPUT | Intermediate files, logs and developer state; defaults to outputs/windows. |
| DeployDirectory | TSAN_DEV_DEPLOY | Delivery root; defaults to outputs/deploy/windows. Deploy creates a TS-Analyzer-windows-v<version>-x86_64/ child containing both formats and checksums. |
| Package | — | Existing unpacked package used by package verification and the optional low-level Installer action; Deploy passes its newly built package automatically. |

  - Generated work and delivery files live under the repository root outputs/ and are ignored by Git. developmentHelpers/ contains maintained tooling, configuration, and test resources. Cargo output remains in target/.
  - If an existing local configuration or TSAN_DEV_OUTPUT, TSAN_DEV_DEPLOY, or TSAN_DEV_TEX_RECORDER override still points to the former output location, update that override to the new root. Explicit overrides continue to take precedence.
  - Empty tool settings enable automatic discovery. Relative paths resolve from the repository root; absolute paths can be supplied locally.
  - Precedence is a command-line argument where offered, then its environment override, local configuration, and finally discovery/defaults.
  - Use -Configuration to select another .psd1 file. An explicitly selected missing file is an error.
  - Do not commit machine-specific SDK paths. The scripts do not create a local configuration or persist prompted inputs automatically.

### Configuration locations

| File or location | Purpose |
| --- | --- |
| [Cargo.toml](../../../Cargo.toml) and crates/*/Cargo.toml | Workspace, dependencies, features, build profiles, and registered examples/tests. |
| [Cargo.lock](../../../Cargo.lock) | Locked Rust dependency versions. |
| [rust-toolchain.toml](../../../rust-toolchain.toml) | Rust version and components; the native host target is selected by rustup. |
| [.vscode/tasks.json](../../../.vscode/tasks.json) | Task labels, arguments, and interactive path inputs. |
| [.vscode/extensions.json](../../../.vscode/extensions.json) | Workspace extension recommendations; does not install extensions automatically. |
| [.vscode/windows/Invoke-Development.ps1](../../../.vscode/windows/Invoke-Development.ps1) | Editor adapter for the shared development entry point. |
| [.vscode/c_cpp_properties.json](../../../.vscode/c_cpp_properties.json) | C++20/TSDuck IntelliSense; uses workspace/environment variables. It does not configure Cargo builds. |
| [developmentHelpers/config/windows.example.psd1](../../../developmentHelpers/config/windows.example.psd1) | Portable configuration template. |
| developmentHelpers/config/windows.local.psd1 | Optional ignored machine overrides. |
| [developmentHelpers/config/deny.toml](../../../developmentHelpers/config/deny.toml) | cargo-deny license/dependency policy. |
| [developmentHelpers/packaging/platforms.toml](../../../developmentHelpers/packaging/platforms.toml) | Independent platform versions, status, and minimum OS. |
| [developmentHelpers/packaging/windows/ts-analyzer.iss](../../../developmentHelpers/packaging/windows/ts-analyzer.iss) | Inno identity, install scope, file rules, and shortcuts. |
| [developmentHelpers/test-data/inputs/manifest.toml](../../../developmentHelpers/test-data/inputs/manifest.toml) and expected/ | Test input identity and expected-data records. |
| outputs/windows/state/tsan-config.toml | GUI settings when launched through the Run task. |
| outputs/windows/diagnostics/ | Automatic diagnostic sessions when launched through Run. TSAN_DIAGNOSTICS_DIR overrides the application directory; Run sets it to OutputDirectory/diagnostics for its child process. |
| Application tsan-config.toml | User preferences; locations are listed below. |
| Package deployment.toml / package.toml / build-info.toml / SHA256SUMS | Generated deployment mode, runtime/build identity, and file integrity inventory. |

  - There is no repository .cargo/config.toml redirecting the build. Cargo uses target/ by default; CARGO_TARGET_DIR and external Cargo configuration still apply.
  - The Windows tasks temporarily set PATH, PKG_CONFIG_PATH, and TSDUCK_HOME, then restore the caller's environment.
  - A task does not configure IntelliSense in an already-running editor. A nonstandard TSDuck installation may need TSDUCK_HOME in the environment used to start VS Code.

### 3. Build, run, and check

~~~powershell
pwsh -NoProfile -File .vscode/windows/Invoke-Development.ps1 -Action Build -Profile Debug
pwsh -NoProfile -File .vscode/windows/Invoke-Development.ps1 -Action Run -Profile Debug
pwsh -NoProfile -File .vscode/windows/Invoke-Development.ps1 -Action CI
~~~

  - Close the application before continuing from Run.
  - CI checks formatting, development scripts/tasks and complete delivery-folder handling, locked Rust license collection, workspace tests (including isolated panic/native crash recovery), update-archive rejection cases, release selection, and portable replacement/rollback. It does not build an installer, publish a release, or run ignored GPU/window/recording tests.
  - The same entry works in a configured Windows CI runner. No repository GitHub auto-publish workflow is currently provided.
  - Direct Cargo use remains available. Prepare the current PowerShell process's native SDK paths first:

~~~powershell
. ./developmentHelpers/scripts/windows/environment.ps1
$devSettings = Get-DevelopmentConfiguration (Get-Location).Path
Enable-DevelopmentEnvironment $devSettings
cargo run --locked -p tsan-gui
~~~

  - Direct cargo run uses the application's normal configuration and diagnostics locations. Run instead sets TSAN_CONFIG_PATH to the development state directory and TSAN_DIAGNOSTICS_DIR to OutputDirectory/diagnostics.
  - For optional cargo-deny checks, after installing that tool explicitly:

~~~powershell
cargo deny --config developmentHelpers/config/deny.toml check
~~~

### VS Code extensions

  - Open the root workspace to see the recommendations in .vscode/extensions.json. These are editor aids, not application runtime dependencies; the file does not install tools or extensions.

| Extension | Project use |
| --- | --- |
| rust-lang.rust-analyzer | Rust navigation, diagnostics, completion, and test discovery. |
| tamasfe.even-better-toml | Cargo manifests, platform versions, and configuration TOML. |
| ms-vscode.cpptools | Native C++ TSDuck bridge and Windows debugger integration. |
| ms-vscode.powershell | Windows build, packaging, validation, and update scripts. |
| ms-vscode.hexeditor | Inspect transport-stream packet bytes. |
| james-yu.latex-workshop | Edit and inspect generated report LaTeX; compiling requires a configured TeX engine. |
| yzhang.markdown-all-in-one | Maintain user/developer Markdown documentation. |

### VS Code tasks and outputs

  - Open the repository root as the workspace. Terminal → Run Task lists the Windows tasks; Ctrl+Shift+B runs Windows: Build debug.
  - .vscode/tasks.json calls the adapter under .vscode/windows/, which forwards to developmentHelpers/scripts/windows/Invoke-Development.ps1. A tasks.json placed only inside a subdirectory is not automatically loaded.
  - Deploy prompts once for a .fls file and automatically passes its new package to the installer build. Package verification tasks prompt for an existing package and recordings. Prompted inputs are not saved automatically.
  - outputs/ is ignored as a whole. Generated directories are created when a task needs them; .gitkeep files are not used there. The macOS/Linux delivery paths reserve the future layout, not tracked empty folders.
  - Each packaging/test run uses a new UTC yyyyMMdd-HHmmss-fff-PID directory under OutputDirectory. Deliverables go to DeployDirectory/TS-Analyzer-windows-v<version>-x86_64/; existing completed releases are never overwritten.

| VS Code task | Action | Output |
| --- | --- | --- |
| Windows: Check environment | CheckEnvironment | Tool locations in the terminal. |
| Windows: Build debug / Build release | Build -Profile Debug / Release | Cargo target/debug or target/release. |
| Windows: Run debug / Run release | Run -Profile Debug / Release | Cargo binary, OutputDirectory/state/tsan-config.toml, and OutputDirectory/diagnostics/session-*/. |
| Windows: Test workspace | Test | Cargo test results; individual tests may use OS temporary directories. |
| Windows: CI checks | CI | Terminal results, license-tests/<run-id>, update-tests/<run-id>, portable-update-tests/<run-id>, and outputs/windows/diagnostics-tests/<run-id>. |
| Windows: Generate synthetic fixtures | GenerateFixtures | developmentHelpers/test-data/inputs/synthetic/transport_detect_packet_size_188.ts. |
| Windows: Build deploy (Portable + Installer) | Deploy | Runs CI, compiles Release, builds ZIP and Installer, verifies checksums, and creates DeployDirectory/TS-Analyzer-windows-v<version>-x86_64/. Working files and symbols stay in OutputDirectory. |
| Windows: Validate development scripts | VerifyDevelopment | Terminal validation results. |
| Windows: Verify update archives | VerifyUpdate | OutputDirectory/update-tests and portable-update-tests. |
| Windows: Verify installation lifecycle | VerifyInstallation | OutputDirectory/installation-tests/<run-id>. |
| Windows: Verify packaged playback | VerifyRuntime | OutputDirectory/runtime-tests/<run-id>. |
| Windows: Verify packaged reports | VerifyReports | OutputDirectory/report-tests/<run-id>. |

  - GenerateFixtures verifies existing matching content and refuses to overwrite different fixture bytes.
  - VerifyInstallation uses an isolated AppId, install directory, shortcuts, and fixture releases. It tests install, upgrade, background update, uninstall, and user-data retention, then cleans up its test registration.
  - VerifyRuntime decodes recordings to fakesink; it does not validate visible playback or seeking.
  - VerifyReports exercises the packaged native runtime and CBOR/XLSX/TeX/PDF export. It needs compatible runtime/GPU components.
  - Terminal output is not universally saved to disk. Dedicated runtime/report/installer/update tests retain their own results and logs.

### 4. Prepare the private PDF runtime

  - A .tex file is the report source. XeLaTeX compiles it into a .pdf. With -recorder, it also writes a .fls text file listing the inputs it used, including TeX packages and fonts. You do not write this file yourself.
  - Packaging reads that .fls and copies the required files from the selected TexRoot, plus engine/font support files and licenses. The recorder must come from that same TeX Live installation, not a previously bundled runtime.
  - The following example uses outputs/windows/pdf-runtime/report.tex. These are repository-relative example paths; no particular drive or user account is required.

  1. From the repository root, create the report directory and start the source-built GUI:

~~~powershell
New-Item -ItemType Directory -Force -Path outputs/windows/pdf-runtime | Out-Null
pwsh -NoProfile -File .vscode/windows/Invoke-Development.ps1 -Action Run -Profile Debug
~~~

  2. Import representative TS files and wait for analysis to finish. In Export Analyzed Report, select LaTeX source (.tex + sections) and save as report.tex in the repository's outputs/windows/pdf-runtime/ directory. Keep the generated report_sections/ directory beside it; it contains the report sections and chart images. Close the GUI.
  3. From the repository root, compile that report using the configured local TeX installation. This creates report_sections/build/report.pdf and report_sections/build/report.fls:

~~~powershell
. ./developmentHelpers/scripts/windows/environment.ps1
$devSettings = Get-DevelopmentConfiguration (Get-Location).Path
if (-not $devSettings.TexRoot) { throw 'Configure TexRoot or make the local XeLaTeX installation discoverable first.' }
$texSource = (Resolve-Path -LiteralPath 'outputs/windows/pdf-runtime/report.tex').Path
$texDirectory = Split-Path -Parent $texSource
$pdfBuild = Join-Path $texDirectory 'report_sections/build'
New-Item -ItemType Directory -Force -Path $pdfBuild | Out-Null
$xelatex = Join-Path $devSettings.TexRoot 'bin/windows/xelatex.exe'
Push-Location -LiteralPath $texDirectory
try {
    1..2 | ForEach-Object {
        & $xelatex -interaction=nonstopmode -halt-on-error -no-shell-escape -recorder "-output-directory=$pdfBuild" $texSource
        if ($LASTEXITCODE -ne 0) { throw 'Report compilation failed; inspect the build log.' }
    }
} finally {
    Pop-Location
}
$recorderPath = Join-Path $pdfBuild 'report.fls'
if (-not (Test-Path -LiteralPath $recorderPath -PathType Leaf)) { throw 'Expected recorder was not generated.' }
Write-Output "TexRecorder: $recorderPath"
~~~

  4. When Windows: Build deploy (Portable + Installer) asks for the recorder, paste the printed TexRecorder: path without the label. For this example, the following repository-relative input is equivalent:

~~~text
outputs/windows/pdf-runtime/report_sections/build/report.fls
~~~

  - A PDF export from the source-built GUI also runs XeLaTeX with -recorder: saving outputs/windows/pdf-runtime/report.pdf creates the same report_sections/build/report.fls location. It is reusable only if that export used the selected local TeX installation.
  - Reuse the recorder for code-only or version-only changes. Regenerate it when the report template, chart types, TeX packages, fonts, or TeX installation change. Use a complete representative report and re-run package report verification after changing PDF dependencies.
  - The recorder and build files stay in outputs/; users receive the required private runtime in the package. This subset is for application reports, not a general-purpose TeX distribution.

### 5. Development workflow: source changes to deploy

  1. Edit the source. For an application release, also change version under [windows] in developmentHelpers/packaging/platforms.toml, for example 0.2.1 to 0.2.2. The Cargo workspace version is separate. Keep the installer AppId unchanged.
  2. During development, use Windows: Run debug to rebuild changed code and run the GUI. Close the GUI before building delivery files.
  3. Run **Windows: Build deploy (Portable + Installer)** once. Enter the complete .fls path prepared in section 4, or leave the input empty to use TexRecorder in the local configuration or TSAN_DEV_TEX_RECORDER. An unchanged report template and TeX installation can reuse the recorder.
  4. The task checks required tools, runs CI, compiles the Release GUI and launcher, assembles the private runtimes/licenses, builds the Portable ZIP, builds the Installer from that exact new package, and verifies both checksums. It does not ask for a package directory or require another task.
  5. On success, copy the four files from the directory printed after Deploy:. With Windows version 0.2.2, this is outputs/deploy/windows/TS-Analyzer-windows-v0.2.2-x86_64/. Preserve the corresponding package run's symbols/ for debugging that release.
  6. Record source changes with a Conventional Commit and push. To publish, create a GitHub release with tag windows-v<version> against that commit and upload the four delivery files. The task does not install the application or publish to GitHub.

| Stage | Task to select in Terminal -> Run Task | Behavior |
| --- | --- | --- |
| Local development | Windows: Run debug | Recompiles changed code and opens the GUI. |
| Checks only | Windows: CI checks | Runs the checks without building user deliverables. |
| Complete delivery | Windows: Build deploy (Portable + Installer) | Runs CI, builds both formats, checks hashes, and groups the four files by platform/version/architecture. |

  - Equivalent PowerShell command from the repository root:

~~~powershell
pwsh -NoProfile -File .vscode/windows/Invoke-Development.ps1 -Action Deploy -TexRecorder 'outputs/windows/pdf-runtime/report_sections/build/report.fls'
~~~

  - The .fls path above is the concrete result of section 4. Use the actual path of your recorder if it is elsewhere. To avoid entering it each time, set a repository-relative TexRecorder in developmentHelpers/config/windows.local.psd1; then leave the task prompt empty or omit -TexRecorder on the command line.
  - A separate CI, Build release, Package, or Installer invocation is not required before or after Deploy. Even a one-line code/version change uses the same Deploy task; Cargo rebuilds affected targets incrementally.
  - Inno Setup must be available before Deploy starts. Missing tools or an existing version's delivery folder stop the task before expensive compilation. Failed CI, compilation, or packaging stops the chain.
  - ZIP and Installer are first built under outputs/windows/packages/<run-id>/artifacts/. Only after both exist and their checksums match are the four files copied into the versioned delivery folder. Failed builds retain intermediate files/logs but do not create a completed delivery folder. A retry uses a new working run directory.
  - An existing versioned delivery folder is never replaced automatically. Use a new version for a new release, or deliberately remove an unpublished local delivery before rebuilding that same version. There is no archive directory. Older flat delivery files are not used as inputs to Deploy.
  - The legacy command-line action Package is an alias for the complete Deploy workflow. The low-level Installer action and build-package.ps1/build-installer.ps1 remain available for development diagnostics; the normal VS Code workflow exposes one delivery task.
  - Deploy resolves Cargo's target_directory and builds tsan-launcher with a static CRT. The installer keeps the same application identity and receives this invocation's package directly.
  - Release builds retain line-level debug information. Packaging keeps tsan_gui.pdb and tsan_launcher.pdb in symbols/, with executable/PDB SHA-256 values and the platform version in symbols/manifest.json. These developer files stay outside the user ZIP and Installer.
  - User packages include docs/usr/ and licenses/. They exclude docs/dev/, developmentHelpers/, .vscode/, SDKs, and compilers.

~~~text
outputs/
    windows/
        state/tsan-config.toml
        packages/<run-id>/
            TS-Analyzer-windows-v<version>-x86_64/
                TS-Analyzer.exe
                deployment.toml
                app/
                runtime/
                scripts/windows/fetch-release.ps1
                docs/usr/
                licenses/
                package.toml
                build-info.toml
                README.txt
                SHA256SUMS
            symbols/
            artifacts/                 # ZIP, setup EXE, checksums before completion
        installers/<run-id>/
            installer.iss
            installed.toml
            compiler.log
        development-tests/<run-id>/
        license-tests/<run-id>/
        update-tests/<run-id>/
        portable-update-tests/<run-id>/
        installation-tests/<run-id>/
        runtime-tests/<run-id>/
        report-tests/<run-id>/
    deploy/
        windows/
            TS-Analyzer-windows-v0.2.2-x86_64/
                TS-Analyzer-windows-v0.2.2-x86_64-portable.zip
                TS-Analyzer-windows-v0.2.2-x86_64-portable.zip.sha256
                TS-Analyzer-windows-v0.2.2-x86_64-setup.exe
                TS-Analyzer-windows-v0.2.2-x86_64-setup.exe.sha256
        macos/                         # Reserved; packaging not implemented
        linux/                         # Reserved; packaging not implemented
~~~

### Third-party license collection

  - The project license remains at the repository root as LICENSE. Maintained third-party notices live under licenses/: liquid-glass-notices.txt and rust/<crate>-<version>/.
  - licenses/rust/sources.json records each supplemental notice's crate version, Cargo source, SPDX metadata, exact upstream revision/URL, and SHA-256. These supplements fill omissions in the published crate contents, including the four embedded font notices. Preserve the original upstream text; .gitattributes disables line-ending conversion for these files.
  - Packaging collects ordinary and nested notices from the locked Cargo packages, adds the exact-version supplements, and writes licenses/rust-dependencies.json plus the selected sources.json into the package. No network download is performed during collection.
  - Missing/empty license text, a mismatched supplement checksum, or changed source/SPDX metadata fails packaging. On dependency upgrades, review the new upstream revision and update any required supplement files and sources.json; do not reuse an older version's entry automatically.
  - Windows: CI checks runs test-rust-licenses.ps1 against the current Windows dependency inventory and exercises rejection cases. Its license files and fixtures stay under outputs/windows/license-tests/<run-id>/.
  - Native GStreamer, TSDuck, MSVC, and TeX notices are collected from the selected runtime installations into the assembled package's licenses/ directory. They are not replaced with the Rust supplements or the project's own LICENSE.
  - Both the portable ZIP and the installer include the assembled licenses/ directory. Run Deploy again for a new release to include changed notices in both delivery formats.

### Versions, installation identity, and updates

  - Cargo crate versions and platform release versions are separate. Change the intended OS entry in platforms.toml; do not advance another platform's version as a side effect.
  - GUI identity, package metadata, asset names, and the windows-vVERSION release tag must agree. Published artifacts must not be silently replaced with different contents.
  - The Windows installer keeps AppId=TSAnalyzer, a current-user install scope, UsePreviousAppDir=yes, and the same shortcut/uninstall identity. The install path is not versioned.
  - deployment.toml alongside the package identifies installed or portable mode. Source builds without a marker cannot use automatic package replacement.
  - tsan-platform provides shared deployment/path rules; tsan-launcher starts the Windows package. Root scripts/windows/ contains product update scripts. fetch-release.ps1 runs through PowerShell -File with separate OS, architecture, and mode arguments.
  - The shared update entry points are check_for_updates/start_check, start_download, and install_staged_update; platform-specific implementations remain behind that interface.
  - Release selection scans stable releases for the current OS and numeric version, then selects the matching mode asset. Missing assets are an error; the updater does not switch modes.
  - Windows PowerShell returns a JSON array from Invoke-RestMethod as one pipeline object. Assign the response before converting it to the release array; wrapping the command directly in @() nests the array and breaks filtering and pagination. The fetch-release regression test preserves this HTTP response shape.
  - Installed updates run the installer against the original directory. Portable updates verify the ZIP, back up managed files, replace them, and remove obsolete managed entries while preserving user data.
  - Portable validation rejects path traversal, case collisions, symlinks/junctions, device names, data/ entries, excessive expansion, and collisions with unmanaged files.
  - A lock serializes replacement. Workers wait for the application to exit without force-killing it; replacement failures roll back the files changed by that attempt. This is not a power-loss recovery system.
  - SHA-256 checks integrity. Authenticode signing is not configured. Full GitHub end-to-end updating must be verified with two actual published releases in addition to local fixture tests.
  - Installer output separation can be checked without installing anything:

~~~powershell
. ./developmentHelpers/scripts/windows/environment.ps1
$devSettings = Get-DevelopmentConfiguration (Get-Location).Path
$testOutput = Join-Path $devSettings.OutputDirectory ("installer-build-tests/" + [DateTime]::UtcNow.ToString("yyyyMMdd-HHmmss-fff"))
& ./developmentHelpers/tests/windows/test-installer-build.ps1 -Compiler $devSettings.InnoCompiler -OutputDirectory $testOutput
~~~

  - This test compiles non-executable fixtures under outputs/, checks the installer/checksum and working files, and never launches the installer. Its output is not for distribution.
  - The tasks neither commit nor publish. Upload only the four files from outputs/deploy/windows/TS-Analyzer-windows-v<version>-x86_64/. The assembled package remains under outputs/windows/ for verification and debugging.
  - Each completed release has its own formatted folder under outputs/deploy/windows/. Timestamped build, test, and symbol directories remain under outputs/windows/.
  - developmentHelpers/ contains maintained build inputs as well as tooling. In particular, GUI build.rs reads packaging/platforms.toml; do not delete the whole helper tree as a cache.

| Data | Installed or direct source build | Portable |
| --- | --- | --- |
| Settings | %APPDATA%/TS-Analyzer/tsan-config.toml | Package/data/tsan-config.toml |
| Runtime cache | %LOCALAPPDATA%/TS-Analyzer/cache | Package/data/cache |
| Update staging/logs | %LOCALAPPDATA%/TS-Analyzer/updates | Package/data/updates |
| Automatic diagnostics | %LOCALAPPDATA%/TS-Analyzer/diagnostics | Package/data/diagnostics |
| Reports and exported logs | Save-dialog destination | Save-dialog destination |

  - An absolute TSAN_CONFIG_PATH explicitly overrides settings; empty or relative overrides fall back to the normal platform location. The Run task uses it for development state. The launcher scopes TSAN_CACHE_ROOT and TSAN_TEX_ROOT to its private runtime.
  - Uninstall preserves AppData and user-owned recordings/reports. Portable removal also removes data/ if the entire directory is deleted.
  - target/ can be deleted with builds and the application stopped; Cargo reconstructs it.
  - outputs/ can be deleted after retaining any needed delivery ZIPs/installers under deploy/, assembled packages, logs, recorder files, or development settings. Tasks recreate outputs, but deleted state and historical validation results are not recovered.
  - Do not delete developmentHelpers/test-data/expected/, maintained synthetic fixtures, scripts, or configuration templates as if they were build output.

## macOS

  - Shared diagnostics use ~/Library/Logs/TS-Analyzer/; TSAN_DIAGNOSTICS_DIR can supply an absolute override. Logging and Rust panic capture are shared; native crash dump collection is explicitly unimplemented.

  - The OS dispatcher selects MacosFrontend and returns a macOS-specific not-implemented error with a nonzero exit code. It receives the same AnalysisService contract as Windows.
  - Shared Rust development uses the pinned Rust toolchain and Cargo. Run cargo test --locked -p tsan-core -p tsan-input -p tsan-runtime -p tsan-platform. No Windows SDK is needed for these targets.
  - Native TSDuck integration is opt-in outside the Windows GUI: --features tsan-analyzer/native-tsduck requires a TSDuck SDK for the Cargo target via TSDUCK_HOME. Cross-builds do not discover a host SDK as a substitute.
  - Future delivery artifacts belong in outputs/deploy/macos/. Build/test intermediates belong in outputs/macos/.

  - The native GUI, playback integration, package build, and updater are not implemented. There is no supported full-application build/release sequence yet.
  - Platform status/version remains independently recorded under [macos] in platforms.toml.
  - Shared Rust modules can be worked on separately; that does not establish native application compatibility. Windows PowerShell tasks, MSVC SDKs, and Inno installers are not macOS tooling.
  - Native prerequisites, deployment target, signing/notarization, runtime packaging, and update verification must be documented when that backend is implemented.

## Linux

  - Shared diagnostics use $XDG_STATE_HOME/TS-Analyzer/diagnostics/ or ~/.local/state/TS-Analyzer/diagnostics/; TSAN_DIAGNOSTICS_DIR can supply an absolute override. Native crash dump collection is explicitly unimplemented.

  - The OS dispatcher selects LinuxFrontend and returns a Linux-specific not-implemented error with a nonzero exit code. It receives the same AnalysisService contract as Windows.
  - Shared Rust development uses the pinned Rust toolchain and Cargo. Run cargo test --locked -p tsan-core -p tsan-input -p tsan-runtime -p tsan-platform.
  - Native TSDuck integration is opt-in outside the Windows GUI: --features tsan-analyzer/native-tsduck requires the target SDK via TSDUCK_HOME.
  - Future delivery artifacts belong in outputs/deploy/linux/. Build/test intermediates belong in outputs/linux/.

  - The native GUI, playback integration, package build, and updater are not implemented. There is no supported full-application build/release sequence yet.
  - Platform status/version remains independently recorded under [linux] in platforms.toml.
  - Future packaging must define the distribution/glibc baseline and native runtime dependencies. Wayland/X11, display protocols, GPU backends, and compositor behavior require validation.
  - Hyprland and Niri are compositor environments to test; separate application versions per window manager are not currently defined. Windows tasks cannot establish Linux compatibility.

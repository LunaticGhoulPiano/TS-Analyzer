# CH1 — Installation and Updates

## Windows

### Requirements and downloads

  - Target systems: Windows 10 22H2 and Windows 11, x64, with a compatible graphics driver. Individual GPU/driver combinations still need validation.
  - Download from the project's [Releases](https://github.com/LunaticGhoulPiano/TS-Analyzer/releases). Choose one distribution:

| Download | Use |
| --- | --- |
| TS-Analyzer-windows-vVERSION-x86_64-setup.exe | Installation wizard, Start menu shortcut, optional desktop shortcut, and Windows uninstall entry. |
| TS-Analyzer-windows-vVERSION-x86_64-portable.zip | Extract and run from a folder you manage. |

  - Use the application assets, rather than GitHub's Source code archives.
  - Both distributions include the private GStreamer, TSDuck, MSVC runtime, and XeLaTeX files needed by the application. Users do not need Rust, Windows SDK, Inno Setup, Excel, or a full TeX installation.
  - The current packaging process does not configure Authenticode signing. A SHA-256 checksum checks file integrity; it is not a publisher signature.

### Install or extract

  1. For the installer, run the setup EXE, select a destination, and finish the wizard. Installation is for the current Windows user.
  2. For portable use, extract the complete ZIP into a writable folder, then run TS-Analyzer.exe.
  3. Keep app/, runtime/, scripts/, docs/, and licenses/ with the launcher. Moving just the EXE will break the package.
  4. Store original recordings and exported reports in folders you manage separately.

### Program files and user data

| Item | Installed copy | Portable copy |
| --- | --- | --- |
| Application | %LOCALAPPDATA%/Programs/TS-Analyzer by default; the wizard allows another directory. | The extracted directory. |
| Settings | %APPDATA%/TS-Analyzer/tsan-config.toml | data/tsan-config.toml in the extracted directory. |
| Runtime cache | %LOCALAPPDATA%/TS-Analyzer/cache | data/cache |
| Update downloads and logs | %LOCALAPPDATA%/TS-Analyzer/updates | data/updates |
| Exported reports and logs | The location chosen in the save dialog. | The location chosen in the save dialog. |

  - Runtime paths apply to the application process; the launcher does not change the system PATH.
  - To move a portable copy, close it and move the complete folder, including data/.
  - Recordings remain at their original locations; importing does not copy them into the application folder.
  - Settings and report contents are explained in [Chapter 3](CH3_ReportsAndSettings.md).

### Update

  1. Open Update and check for a newer compatible Windows release.
  2. Download the update from the application, then use its update action.
  3. The application closes while the update worker waits for it to exit. Keep the destination drive connected.
  4. A successful update restarts the application from its original location.

  - Installed copies use the new installer and retain the same installation directory, shortcuts, and uninstall identity.
  - Portable copies replace the managed program files in the existing portable directory and preserve data/.
  - The updater selects the asset for the current platform and distribution mode. It does not switch a portable copy into an installation.
  - Failed portable file replacement restores the previous managed files. Backups and update.log remain under the relevant updates directory; installed updates also have installer.log.
  - A source build is updated through Git and rebuilt. Older packages without deployment.toml require a manual download.

### Remove

  - Installed: close TS Analyzer, then use Windows Settings → Apps → TS Analyzer → Uninstall. This removes installed program files, shortcuts, and the uninstall entry.
  - Settings, caches, update logs, original recordings, and exported reports are retained. After uninstalling all installed copies, you can separately remove the TS-Analyzer folders under AppData if you no longer need their contents.
  - Portable: close the application, back up any wanted data or reports inside the folder, then delete the extracted directory.
  - No system-wide GStreamer, TeX, or development tools need to be removed as part of uninstalling the application.

## macOS

  - A native application, installer, and updater are not implemented yet. There is no supported macOS installation procedure.

## Linux

  - A native application, package, and updater are not implemented yet. There is no supported Wayland/X11 installation procedure.

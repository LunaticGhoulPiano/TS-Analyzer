; Compiled by build-installer.ps1. Paths and version come from the package.
#ifndef PackageDir
  #error PackageDir is required
#endif
#ifndef PackageVersion
  #error PackageVersion is required
#endif
#ifndef InstallerOutput
  #error InstallerOutput is required
#endif

[Setup]
; Keep this identity fixed across every Windows release.
AppId=TSAnalyzer
AppName=TS Analyzer
AppVersion={#PackageVersion}
AppPublisher=LunaticGhoulPiano
AppPublisherURL=https://github.com/LunaticGhoulPiano/TS-Analyzer
DefaultDirName={localappdata}\Programs\TS-Analyzer
DefaultGroupName=TS Analyzer
UsePreviousAppDir=yes
UsePreviousGroup=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.19045
DisableDirPage=no
OutputDir={#InstallerOutput}
OutputBaseFilename=TS-Analyzer-windows-v{#PackageVersion}-x86_64-setup
Compression=lzma2
SolidCompression=yes
CloseApplications=yes
RestartApplications=no
UninstallDisplayIcon={app}\TS-Analyzer.exe
WizardStyle=modern

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; Flags: unchecked

[Files]
Source: "{#PackageDir}\*"; DestDir: "{app}"; Excludes: "\Setup.exe,*.pdb,\SHA256SUMS,\deployment.toml,\data\*"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "{#SourcePath}\installed.toml"; DestDir: "{app}"; DestName: "deployment.toml"; Flags: ignoreversion

[Icons]
Name: "{group}\TS Analyzer"; Filename: "{app}\TS-Analyzer.exe"
Name: "{autodesktop}\TS Analyzer"; Filename: "{app}\TS-Analyzer.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\TS-Analyzer.exe"; Description: "Launch TS Analyzer"; Flags: nowait postinstall skipifsilent

[Code]
function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  Marker: AnsiString;
begin
  Result := '';
  if LoadStringFromFile(ExpandConstant('{app}\deployment.toml'), Marker) then
  begin
    if Pos('mode = "portable"', String(Marker)) > 0 then
      Result := 'This folder contains a portable copy. Choose another installation folder.';
  end;
end;

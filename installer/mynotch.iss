; Per-user installer for MyNotch: no admin rights, installs to %LOCALAPPDATA%\Programs\MyNotch
; and adds a Start Menu shortcut. Build: `cargo build --release -p mynotch`, then
; `ISCC.exe installer\mynotch.iss` -> installer\Output\MyNotch-Setup.exe.
; The Whisper model (%LOCALAPPDATA%\MyNotch\models) and settings (%APPDATA%\MyNotch) are
; left alone by install and uninstall. Voice also needs CUDA 13's cuBLAS DLLs on PATH.

#define AppName "MyNotch"
#define AppVersion "0.1.0"

[Setup]
AppId={{6B0E5C1A-3D2F-4F6B-9A57-4C8E1B7D2A90}
AppName={#AppName}
AppVersion={#AppVersion}
AppPublisher=Contact Established
DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
PrivilegesRequired=lowest
DisableProgramGroupPage=yes
UninstallDisplayIcon={app}\mynotch.exe
OutputDir=Output
OutputBaseFilename=MyNotch-Setup
Compression=lzma2
SolidCompression=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
CloseApplications=yes
WizardStyle=modern

[Files]
Source: "..\target\release\mynotch.exe"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\mynotch.exe"

[Run]
Filename: "{app}\mynotch.exe"; Description: "Start {#AppName} now"; Flags: nowait postinstall skipifsilent

[UninstallRun]
Filename: "{sys}\taskkill.exe"; Parameters: "/F /IM mynotch.exe"; Flags: runhidden; RunOnceId: "StopMyNotch"

[UninstallDelete]
Type: filesandordirs; Name: "{app}"

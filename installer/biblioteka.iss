#define MyAppName "Library Studio"
#define MyAppVersion "0.3.3"
#define MyAppExeName "biblioteka.exe"
#ifndef ExePath
  #define ExePath "..\src-tauri\target\release\biblioteka.exe"
#endif

[Setup]
AppId={{B1B7107E-4A2C-4E5D-9F11-8C0A6D3E91B4}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher=Library Studio
DefaultDirName={autopf}\Library Studio
DefaultGroupName=Library Studio
DisableProgramGroupPage=yes
OutputDir=output
OutputBaseFilename=Library-Studio-Setup-{#MyAppVersion}
SetupIconFile=..\src-tauri\icons\icon.ico
UninstallDisplayIcon={app}\{#MyAppExeName}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
CloseApplications=yes
RestartApplications=no
VersionInfoVersion=0.3.3.0
VersionInfoProductName=Library Studio
VersionInfoDescription=Private book catalog

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "polish"; MessagesFile: "compiler:Languages\Polish.isl"

[CustomMessages]
english.DesktopIcon=Create a desktop shortcut
polish.DesktopIcon=Utwórz skrót na pulpicie
english.ExtraIcons=Additional icons:
polish.ExtraIcons=Dodatkowe ikony:
english.Launch=Launch Library Studio
polish.Launch=Uruchom Library Studio

[Tasks]
Name: "desktopicon"; Description: "{cm:DesktopIcon}"; GroupDescription: "{cm:ExtraIcons}"; Flags: unchecked

[Files]
Source: "{#ExePath}"; DestDir: "{app}"; DestName: "{#MyAppExeName}"; Flags: ignoreversion

[Icons]
Name: "{group}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "{cm:Launch}"; Flags: nowait postinstall skipifsilent

[Code]
function WebView2Installed: Boolean;
var
  Version: String;
  Client: String;
begin
  Client := 'Microsoft\EdgeUpdate\Clients\' + Chr(123) + 'F3017226-FE2A-4295-8BDF-00C3A9A7E4C5' + Chr(125);
  Result :=
    RegQueryStringValue(HKLM, 'SOFTWARE\WOW6432Node\' + Client, 'pv', Version) or
    RegQueryStringValue(HKLM, 'SOFTWARE\' + Client, 'pv', Version) or
    RegQueryStringValue(HKCU, 'SOFTWARE\' + Client, 'pv', Version);
end;

function InitializeSetup: Boolean;
begin
  Result := WebView2Installed;
  if not Result then
    MsgBox('Brak środowiska Microsoft Edge WebView2. Zainstaluj WebView2 Runtime i uruchom instalator ponownie.', mbError, MB_OK);
end;

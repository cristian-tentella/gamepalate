#define BinDir "..\..\target\release"
#define AppExe "gamepalate.exe"
#define Version GetStringFileInfo(BinDir + "\" + AppExe, "ProductVersion")

[Setup]
AppId={{520E869D-2EE1-494D-B36E-95A64B797F50}
AppName=gamepalate
AppVersion={#Version}
AppVerName=gamepalate {#Version}
AppPublisher=Cristian Tentella
AppPublisherURL=https://github.com/cristian-tentella/gamepalate
AppSupportURL=https://github.com/cristian-tentella/gamepalate/issues
AppUpdatesURL=https://github.com/cristian-tentella/gamepalate/releases
VersionInfoVersion={#Version}
DefaultDirName={autopf}\gamepalate
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=..\..\target\installer
OutputBaseFilename=gamepalate-setup
SetupIconFile=gamepalate.ico
UninstallDisplayIcon={app}\{#AppExe}
WizardStyle=modern
Compression=lzma2/max
SolidCompression=yes

[Tasks]
Name: desktopicon; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#BinDir}\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#BinDir}\gamepalate-cli.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\LICENSE-MIT"; DestDir: "{app}"
Source: "..\..\LICENSE-APACHE"; DestDir: "{app}"

[Icons]
Name: "{autoprograms}\gamepalate"; Filename: "{app}\{#AppExe}"
Name: "{autodesktop}\gamepalate"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#AppExe}"; Description: "{cm:LaunchProgram,gamepalate}"; Flags: nowait postinstall skipifsilent
Filename: "{app}\{#AppExe}"; Flags: nowait; Check: ShouldLaunch

[Code]
function ShouldLaunch: Boolean;
var
  I: Integer;
begin
  Result := WizardSilent;
  for I := 1 to ParamCount do
    if CompareText(ParamStr(I), '/NOLAUNCH') = 0 then
      Result := False;
end;

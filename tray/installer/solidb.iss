; The Windows installer of SoliDB (Inno Setup 6).
;
;   ISCC /DAppVersion=2.13.0 /DBin=<folder holding the executables> solidb.iss
;
; It copies the server, its tools, the notification-area icon, the service
; host and the admin application into Program Files, and offers to register the Windows
; service, to start the icon at sign-in and to add the folder to PATH.
; The data (%ProgramData%\SoliDB, %LOCALAPPDATA%\SoliDB) is never touched,
; neither by an upgrade nor by an uninstall.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef Bin
  #define Bin "."
#endif
; The admin application, which the icon runs for "Open admin console".
#ifndef Admin
  #define Admin "..\..\admin"
#endif
#ifndef OutDir
  #define OutDir "."
#endif

[Setup]
; Identifies SoliDB to Windows across versions: never change it, or an
; upgrade installs a second copy beside the first.
AppId={{6F0C7B1E-3A52-4E0B-9C4D-5D2B8A7E1F30}
AppName=SoliDB
AppVersion={#AppVersion}
AppPublisher=Solisoft
AppPublisherURL=https://github.com/solisoft/solidb
AppSupportURL=https://github.com/solisoft/solidb/issues
DefaultDirName={autopf}\SoliDB
DefaultGroupName=SoliDB
DisableProgramGroupPage=yes
; The service, Program Files and the machine's PATH all need it.
PrivilegesRequired=admin
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir={#OutDir}
OutputBaseFilename=solidb-windows-amd64-setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
ChangesEnvironment=yes
UninstallDisplayName=SoliDB {#AppVersion}
UninstallDisplayIcon={app}\solidb.exe
; A server the icon started on its own holds solidb.exe open: let Windows
; ask it to close (it gets Ctrl-C, and closes its database).
CloseApplications=yes
RestartApplications=no

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "french"; MessagesFile: "compiler:Languages\French.isl"

[CustomMessages]
english.TaskService=Run SoliDB as a Windows service, started with the machine
french.TaskService=Faire tourner SoliDB comme service Windows, démarré avec la machine
english.TaskIcon=Show the SoliDB icon in the notification area when I sign in
french.TaskIcon=Afficher l'icône SoliDB dans la zone de notification à l'ouverture de session
english.TaskPath=Add SoliDB's tools to the PATH
french.TaskPath=Ajouter les outils SoliDB au PATH
english.RunIcon=Start the SoliDB icon now
french.RunIcon=Lancer l'icône SoliDB maintenant
english.StatusService=Registering the SoliDB service...
french.StatusService=Enregistrement du service SoliDB...
english.StatusStart=Starting SoliDB...
french.StatusStart=Démarrage de SoliDB...
english.StatusStop=Stopping SoliDB...
french.StatusStop=Arrêt de SoliDB...
english.NotStarted=SoliDB is installed, but its service did not start.%n%nAnother program may be using its port, or a SoliDB server may already be running. The reason is in the log:%n%1
french.NotStarted=SoliDB est installé, mais son service n'a pas démarré.%n%nUn autre programme utilise peut-être son port, ou un serveur SoliDB tourne déjà. La raison est dans le log :%n%1

[Tasks]
Name: "service"; Description: "{cm:TaskService}"
Name: "icon"; Description: "{cm:TaskIcon}"
Name: "path"; Description: "{cm:TaskPath}"

[Files]
Source: "{#Bin}\solidb.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Bin}\solidb-dump.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Bin}\solidb-restore.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Bin}\solidb-migrate-engine.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Bin}\solidb-tray.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Bin}\solidb-service.exe"; DestDir: "{app}"; Flags: ignoreversion
; The parts of the admin application that are its code: the list is the
; icon's (PARTS in tray/src/console.rs). The Soli runtime it runs on is not
; here: the icon downloads it at the first "Open admin console".
Source: "{#Admin}\app\*"; DestDir: "{app}\admin\app"; Excludes: "CLAUDE.md,AGENTS.md"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "{#Admin}\config\*"; DestDir: "{app}\admin\config"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "{#Admin}\public\*"; DestDir: "{app}\admin\public"; Flags: ignoreversion recursesubdirs createallsubdirs
Source: "{#Admin}\soli.toml"; DestDir: "{app}\admin"; Flags: ignoreversion

[InstallDelete]
; An upgrade leaves no file of the admin application of the version before.
Type: filesandordirs; Name: "{app}\admin"

[Icons]
Name: "{group}\SoliDB"; Filename: "{app}\solidb-tray.exe"
Name: "{commonstartup}\SoliDB"; Filename: "{app}\solidb-tray.exe"; Tasks: icon

[Run]
Filename: "{app}\solidb-tray.exe"; Description: "{cm:RunIcon}"; Flags: nowait postinstall skipifsilent runasoriginaluser

[UninstallRun]
; Stops the server (it closes its database) and removes the service.
Filename: "{app}\solidb-service.exe"; Parameters: "uninstall"; Flags: runhidden; RunOnceId: "RemoveService"

[Code]
const
  EnvKey = 'SYSTEM\CurrentControlSet\Control\Session Manager\Environment';
  WM_CLOSE = $0010;

// The icon closes on WM_CLOSE and leaves the server to whoever runs it.
procedure CloseIcon;
var
  Window: HWND;
begin
  Window := FindWindowByClassName('SoliDBTrayWindow');
  if Window <> 0 then
  begin
    PostMessage(Window, WM_CLOSE, 0, 0);
    Sleep(500);
  end;
end;

function ServiceIsInstalled: Boolean;
var
  Code: Integer;
begin
  // `sc query` answers 1060 for a service that does not exist.
  Result := Exec(ExpandConstant('{sys}\sc.exe'), 'query SoliDB', '', SW_HIDE,
    ewWaitUntilTerminated, Code) and (Code = 0);
end;

// Before any file is replaced: an upgrade must not find solidb.exe running.
function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  Code: Integer;
  Host: String;
begin
  Result := '';
  CloseIcon;
  Host := ExpandConstant('{app}\solidb-service.exe');
  if FileExists(Host) and ServiceIsInstalled then
  begin
    WizardForm.StatusLabel.Caption := CustomMessage('StatusStop');
    Exec(Host, 'stop', '', SW_HIDE, ewWaitUntilTerminated, Code);
  end;
end;

// ---- PATH: the folder is added once, and taken out again by an uninstall.

function PathWithout(Path, Dir: String): String;
var
  Rest, Part: String;
  At: Integer;
begin
  Result := '';
  Rest := Path;
  while Rest <> '' do
  begin
    At := Pos(';', Rest);
    if At = 0 then
    begin
      Part := Rest;
      Rest := '';
    end
    else
    begin
      Part := Copy(Rest, 1, At - 1);
      Rest := Copy(Rest, At + 1, Length(Rest));
    end;
    if (Part <> '') and (CompareText(RemoveBackslashUnlessRoot(Part), Dir) <> 0) then
    begin
      if Result <> '' then
        Result := Result + ';';
      Result := Result + Part;
    end;
  end;
end;

procedure SetPath(Add: Boolean);
var
  Path, Dir, Next: String;
begin
  Dir := RemoveBackslashUnlessRoot(ExpandConstant('{app}'));
  if not RegQueryStringValue(HKLM, EnvKey, 'Path', Path) then
    Path := '';
  Next := PathWithout(Path, Dir);
  if Add then
  begin
    if Next <> '' then
      Next := Next + ';';
    Next := Next + Dir;
  end;
  if Next <> Path then
    RegWriteExpandStringValue(HKLM, EnvKey, 'Path', Next);
end;

// Registers the service when asked to, and starts it whenever it exists:
// an upgrade stopped it before replacing its files. A service that does
// not start is said, not passed over: the usual reason is a port in use.
procedure RunService;
var
  Code: Integer;
  Host: String;
begin
  Host := ExpandConstant('{app}\solidb-service.exe');
  if WizardIsTaskSelected('service') and not ServiceIsInstalled then
  begin
    WizardForm.StatusLabel.Caption := CustomMessage('StatusService');
    Exec(Host, 'install', '', SW_HIDE, ewWaitUntilTerminated, Code);
  end;
  if ServiceIsInstalled then
  begin
    WizardForm.StatusLabel.Caption := CustomMessage('StatusStart');
    if not Exec(Host, 'start', '', SW_HIDE, ewWaitUntilTerminated, Code) or (Code <> 0) then
    begin
      Log('The SoliDB service did not start (solidb-service start: ' + IntToStr(Code) + ').');
      SuppressibleMsgBox(
        FmtMessage(CustomMessage('NotStarted'), [ExpandConstant('{commonappdata}\SoliDB\solidb.log')]),
        mbError, MB_OK, IDOK);
    end;
  end;
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssPostInstall then
  begin
    if WizardIsTaskSelected('path') then
      SetPath(True);
    RunService;
  end;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usUninstall then
  begin
    CloseIcon;
    SetPath(False);
  end;
end;

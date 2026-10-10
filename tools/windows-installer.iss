#ifndef AppVersion
  #define AppVersion "0.1.0"
#endif
[Setup]
AppId={{9FA46BCA-804A-4B40-80D7-DFF74C6F3C93}
AppName=Crant Peek
AppVersion={#AppVersion}
AppPublisher=Crant Peek
AppPublisherURL=https://github.com/RhedarLiu/CrantPeek
DefaultDirName={localappdata}\Programs\Crant Peek
DefaultGroupName=Crant Peek
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=..\dist
OutputBaseFilename=Crant-Peek-Windows-Setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
UninstallDisplayIcon={app}\CrantPeek.exe
CloseApplications=yes
CloseApplicationsFilter=CrantPeek.exe
RestartApplications=no
LicenseFile=..\LICENSE
[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
[Files]
Source: "..\dist\Crant-Peek-Windows\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs
[Icons]
Name: "{group}\Crant Peek"; Filename: "{app}\CrantPeek.exe"
[Run]
Filename: "{app}\CrantPeek.exe"; Description: "Open Crant Peek"; Flags: nowait postinstall skipifsilent

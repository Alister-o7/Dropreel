; Inno Setup 6 script for DropReel. Built by packaging\package.ps1, which passes
; AppVersion, SourceDir (the assembled dist\DropReel folder) and OutputDir.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef SourceDir
  #define SourceDir "..\dist\DropReel"
#endif
#ifndef OutputDir
  #define OutputDir "..\dist"
#endif

[Setup]
AppId={{6C1F3E2A-9B4D-4E8A-A7C5-2D9E0B1F4A63}
AppName=DropReel
AppVersion={#AppVersion}
AppPublisher=DropReel
AppPublisherURL=https://github.com/alister-o7/dropreel
DefaultDirName={autopf}\DropReel
DefaultGroupName=DropReel
DisableProgramGroupPage=yes
; Installs per-user by default (no admin prompt); the user can choose all users instead.
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir={#OutputDir}
OutputBaseFilename=DropReel-{#AppVersion}-setup
SetupIconFile=..\assets\icon.ico
UninstallDisplayIcon={app}\DropReel.exe
UninstallDisplayName=DropReel
LicenseFile=..\LICENSE
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
ChangesAssociations=yes

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#SourceDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{autoprograms}\DropReel"; Filename: "{app}\DropReel.exe"
Name: "{autodesktop}\DropReel"; Filename: "{app}\DropReel.exe"; Tasks: desktopicon

[Registry]
; Adds DropReel to "Open with" for video files without taking over existing defaults.
Root: HKA; Subkey: "Software\Classes\Applications\DropReel.exe"; ValueType: string; ValueName: "FriendlyAppName"; ValueData: "DropReel"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Applications\DropReel.exe\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: """{app}\DropReel.exe"",0"
Root: HKA; Subkey: "Software\Classes\Applications\DropReel.exe\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\DropReel.exe"" ""%1"""
Root: HKA; Subkey: "Software\Classes\Applications\DropReel.exe\SupportedTypes"; ValueType: string; ValueName: ".mkv"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.mkv\OpenWithList\DropReel.exe"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Applications\DropReel.exe\SupportedTypes"; ValueType: string; ValueName: ".mp4"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.mp4\OpenWithList\DropReel.exe"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Applications\DropReel.exe\SupportedTypes"; ValueType: string; ValueName: ".m4v"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.m4v\OpenWithList\DropReel.exe"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Applications\DropReel.exe\SupportedTypes"; ValueType: string; ValueName: ".avi"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.avi\OpenWithList\DropReel.exe"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Applications\DropReel.exe\SupportedTypes"; ValueType: string; ValueName: ".mov"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.mov\OpenWithList\DropReel.exe"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Applications\DropReel.exe\SupportedTypes"; ValueType: string; ValueName: ".wmv"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.wmv\OpenWithList\DropReel.exe"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Applications\DropReel.exe\SupportedTypes"; ValueType: string; ValueName: ".webm"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.webm\OpenWithList\DropReel.exe"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Applications\DropReel.exe\SupportedTypes"; ValueType: string; ValueName: ".ts"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.ts\OpenWithList\DropReel.exe"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Applications\DropReel.exe\SupportedTypes"; ValueType: string; ValueName: ".m2ts"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.m2ts\OpenWithList\DropReel.exe"; Flags: uninsdeletekey

[Run]
Filename: "{app}\DropReel.exe"; Description: "{cm:LaunchProgram,DropReel}"; Flags: nowait postinstall skipifsilent

; Inno Setup 6 script for Dropreel. Built by packaging\package.ps1, which passes
; AppVersion, SourceDir (the assembled dist\Dropreel folder) and OutputDir.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef SourceDir
  #define SourceDir "..\dist\Dropreel"
#endif
#ifndef OutputDir
  #define OutputDir "..\dist"
#endif

[Setup]
AppId={{6C1F3E2A-9B4D-4E8A-A7C5-2D9E0B1F4A63}
AppName=Dropreel
AppVersion={#AppVersion}
AppPublisher=Dropreel
AppPublisherURL=https://github.com/alister-o7/dropreel
DefaultDirName={autopf}\Dropreel
DefaultGroupName=Dropreel
DisableProgramGroupPage=yes
; Installs per-user by default (no admin prompt); the user can choose all users instead.
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir={#OutputDir}
OutputBaseFilename=Dropreel-{#AppVersion}-setup
SetupIconFile=..\assets\icon.ico
UninstallDisplayIcon={app}\Dropreel.exe
UninstallDisplayName=Dropreel
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
Name: "{autoprograms}\Dropreel"; Filename: "{app}\Dropreel.exe"
Name: "{autodesktop}\Dropreel"; Filename: "{app}\Dropreel.exe"; Tasks: desktopicon

[Registry]
; Adds Dropreel to "Open with" for video files without taking over existing defaults.
Root: HKA; Subkey: "Software\Classes\Applications\Dropreel.exe"; ValueType: string; ValueName: "FriendlyAppName"; ValueData: "Dropreel"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Applications\Dropreel.exe\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: """{app}\Dropreel.exe"",0"
Root: HKA; Subkey: "Software\Classes\Applications\Dropreel.exe\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\Dropreel.exe"" ""%1"""
Root: HKA; Subkey: "Software\Classes\Applications\Dropreel.exe\SupportedTypes"; ValueType: string; ValueName: ".mkv"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.mkv\OpenWithList\Dropreel.exe"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Applications\Dropreel.exe\SupportedTypes"; ValueType: string; ValueName: ".mp4"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.mp4\OpenWithList\Dropreel.exe"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Applications\Dropreel.exe\SupportedTypes"; ValueType: string; ValueName: ".m4v"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.m4v\OpenWithList\Dropreel.exe"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Applications\Dropreel.exe\SupportedTypes"; ValueType: string; ValueName: ".avi"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.avi\OpenWithList\Dropreel.exe"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Applications\Dropreel.exe\SupportedTypes"; ValueType: string; ValueName: ".mov"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.mov\OpenWithList\Dropreel.exe"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Applications\Dropreel.exe\SupportedTypes"; ValueType: string; ValueName: ".wmv"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.wmv\OpenWithList\Dropreel.exe"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Applications\Dropreel.exe\SupportedTypes"; ValueType: string; ValueName: ".webm"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.webm\OpenWithList\Dropreel.exe"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Applications\Dropreel.exe\SupportedTypes"; ValueType: string; ValueName: ".ts"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.ts\OpenWithList\Dropreel.exe"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Applications\Dropreel.exe\SupportedTypes"; ValueType: string; ValueName: ".m2ts"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.m2ts\OpenWithList\Dropreel.exe"; Flags: uninsdeletekey

[Run]
Filename: "{app}\Dropreel.exe"; Description: "{cm:LaunchProgram,Dropreel}"; Flags: nowait postinstall skipifsilent

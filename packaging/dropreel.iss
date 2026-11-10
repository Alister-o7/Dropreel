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
; Offers Dropreel for video files without taking over existing defaults: it appears in "Open with"
; and under Settings > Apps > Default apps, where the user can make it the default player.
Root: HKA; Subkey: "Software\Classes\Applications\Dropreel.exe"; ValueType: string; ValueName: "FriendlyAppName"; ValueData: "Dropreel"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Applications\Dropreel.exe\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: """{app}\Dropreel.exe"",0"
Root: HKA; Subkey: "Software\Classes\Applications\Dropreel.exe\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\Dropreel.exe"" ""%1"""
; The file type Dropreel opens videos as (what a default for .mkv etc. points to).
Root: HKA; Subkey: "Software\Classes\Dropreel.Video"; ValueType: string; ValueName: ""; ValueData: "Video"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\Dropreel.Video"; ValueType: string; ValueName: "FriendlyTypeName"; ValueData: "Video (Dropreel)"
Root: HKA; Subkey: "Software\Classes\Dropreel.Video\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: """{app}\Dropreel.exe"",0"
Root: HKA; Subkey: "Software\Classes\Dropreel.Video\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\Dropreel.exe"" ""%1"""
; Lists Dropreel in Default apps.
Root: HKA; Subkey: "Software\Dropreel"; Flags: uninsdeletekeyifempty
Root: HKA; Subkey: "Software\Dropreel\Capabilities"; ValueType: string; ValueName: "ApplicationName"; ValueData: "Dropreel"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Dropreel\Capabilities"; ValueType: string; ValueName: "ApplicationDescription"; ValueData: "Movie and show library with a built-in video player."
Root: HKA; Subkey: "Software\Dropreel\Capabilities"; ValueType: string; ValueName: "ApplicationIcon"; ValueData: """{app}\Dropreel.exe"",0"
Root: HKA; Subkey: "Software\RegisteredApplications"; ValueType: string; ValueName: "Dropreel"; ValueData: "Software\Dropreel\Capabilities"; Flags: uninsdeletevalue
; .mkv
Root: HKA; Subkey: "Software\Classes\Applications\Dropreel.exe\SupportedTypes"; ValueType: string; ValueName: ".mkv"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.mkv\OpenWithList\Dropreel.exe"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\.mkv\OpenWithProgids"; ValueType: string; ValueName: "Dropreel.Video"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Dropreel\Capabilities\FileAssociations"; ValueType: string; ValueName: ".mkv"; ValueData: "Dropreel.Video"
; .mp4
Root: HKA; Subkey: "Software\Classes\Applications\Dropreel.exe\SupportedTypes"; ValueType: string; ValueName: ".mp4"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.mp4\OpenWithList\Dropreel.exe"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\.mp4\OpenWithProgids"; ValueType: string; ValueName: "Dropreel.Video"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Dropreel\Capabilities\FileAssociations"; ValueType: string; ValueName: ".mp4"; ValueData: "Dropreel.Video"
; .m4v
Root: HKA; Subkey: "Software\Classes\Applications\Dropreel.exe\SupportedTypes"; ValueType: string; ValueName: ".m4v"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.m4v\OpenWithList\Dropreel.exe"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\.m4v\OpenWithProgids"; ValueType: string; ValueName: "Dropreel.Video"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Dropreel\Capabilities\FileAssociations"; ValueType: string; ValueName: ".m4v"; ValueData: "Dropreel.Video"
; .avi
Root: HKA; Subkey: "Software\Classes\Applications\Dropreel.exe\SupportedTypes"; ValueType: string; ValueName: ".avi"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.avi\OpenWithList\Dropreel.exe"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\.avi\OpenWithProgids"; ValueType: string; ValueName: "Dropreel.Video"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Dropreel\Capabilities\FileAssociations"; ValueType: string; ValueName: ".avi"; ValueData: "Dropreel.Video"
; .mov
Root: HKA; Subkey: "Software\Classes\Applications\Dropreel.exe\SupportedTypes"; ValueType: string; ValueName: ".mov"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.mov\OpenWithList\Dropreel.exe"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\.mov\OpenWithProgids"; ValueType: string; ValueName: "Dropreel.Video"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Dropreel\Capabilities\FileAssociations"; ValueType: string; ValueName: ".mov"; ValueData: "Dropreel.Video"
; .wmv
Root: HKA; Subkey: "Software\Classes\Applications\Dropreel.exe\SupportedTypes"; ValueType: string; ValueName: ".wmv"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.wmv\OpenWithList\Dropreel.exe"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\.wmv\OpenWithProgids"; ValueType: string; ValueName: "Dropreel.Video"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Dropreel\Capabilities\FileAssociations"; ValueType: string; ValueName: ".wmv"; ValueData: "Dropreel.Video"
; .webm
Root: HKA; Subkey: "Software\Classes\Applications\Dropreel.exe\SupportedTypes"; ValueType: string; ValueName: ".webm"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.webm\OpenWithList\Dropreel.exe"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\.webm\OpenWithProgids"; ValueType: string; ValueName: "Dropreel.Video"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Dropreel\Capabilities\FileAssociations"; ValueType: string; ValueName: ".webm"; ValueData: "Dropreel.Video"
; .ts
Root: HKA; Subkey: "Software\Classes\Applications\Dropreel.exe\SupportedTypes"; ValueType: string; ValueName: ".ts"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.ts\OpenWithList\Dropreel.exe"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\.ts\OpenWithProgids"; ValueType: string; ValueName: "Dropreel.Video"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Dropreel\Capabilities\FileAssociations"; ValueType: string; ValueName: ".ts"; ValueData: "Dropreel.Video"
; .m2ts
Root: HKA; Subkey: "Software\Classes\Applications\Dropreel.exe\SupportedTypes"; ValueType: string; ValueName: ".m2ts"; ValueData: ""
Root: HKA; Subkey: "Software\Classes\.m2ts\OpenWithList\Dropreel.exe"; Flags: uninsdeletekey
Root: HKA; Subkey: "Software\Classes\.m2ts\OpenWithProgids"; ValueType: string; ValueName: "Dropreel.Video"; ValueData: ""; Flags: uninsdeletevalue
Root: HKA; Subkey: "Software\Dropreel\Capabilities\FileAssociations"; ValueType: string; ValueName: ".m2ts"; ValueData: "Dropreel.Video"

[Run]
Filename: "{app}\Dropreel.exe"; Description: "{cm:LaunchProgram,Dropreel}"; Flags: nowait postinstall skipifsilent

; Installer for Sparky PDF. Needs Inno Setup 6.3 or newer.
;
; It is built by scripts\build-release.ps1, which first puts SparkyPDF-arm64.exe and
; SparkyPDF-x64.exe into dist\. The installer carries both and installs the one that
; matches the PC. It installs for the current user only and never asks for
; administrator rights.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif

[Setup]
AppId={{6C0E5B0B-5D53-4C3A-9B43-6F2E7C0E91A4}
AppName=Sparky PDF
AppVersion={#AppVersion}
AppVerName=Sparky PDF {#AppVersion}
AppPublisher=Sparky PDF
AppPublisherURL=https://github.com/roberthendriksen/SparkyPDF
AppSupportURL=https://github.com/roberthendriksen/SparkyPDF/issues
VersionInfoVersion={#AppVersion}
PrivilegesRequired=lowest
DefaultDirName={autopf}\Sparky PDF
DisableProgramGroupPage=yes
DisableDirPage=auto
ArchitecturesAllowed=x64compatible arm64
ArchitecturesInstallIn64BitMode=x64compatible arm64
MinVersion=10.0.17763
SetupIconFile=..\assets\icon.ico
UninstallDisplayIcon={app}\SparkyPDF.exe
UninstallDisplayName=Sparky PDF
LicenseFile=..\LICENSE
OutputBaseFilename=SparkyPDF-Setup
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
CloseApplications=yes
ChangesAssociations=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Files]
Source: "..\dist\SparkyPDF-arm64.exe"; DestDir: "{app}"; DestName: "SparkyPDF.exe"; Check: IsArm64; Flags: ignoreversion
Source: "..\dist\SparkyPDF-x64.exe"; DestDir: "{app}"; DestName: "SparkyPDF.exe"; Check: not IsArm64; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Sparky PDF"; Filename: "{app}\SparkyPDF.exe"

[Run]
; Always list Sparky PDF under "Open with" for PDF files. This does not change the default app.
Filename: "{app}\SparkyPDF.exe"; Parameters: "--register-quiet"; Flags: runhidden
; Offered on the last page. Windows only lets the user pick the default, so this opens Settings.
Filename: "{app}\SparkyPDF.exe"; Parameters: "--register"; Description: "Choose Sparky PDF as my default PDF reader (opens Windows Settings)"; Flags: postinstall nowait skipifsilent
Filename: "{app}\SparkyPDF.exe"; Description: "Start Sparky PDF"; Flags: postinstall nowait skipifsilent unchecked

[UninstallRun]
Filename: "{app}\SparkyPDF.exe"; Parameters: "--unregister"; RunOnceId: "UnregisterPdfApp"; Flags: runhidden

[UninstallDelete]
; The recent-files list and the WebView2 profile folder.
Type: filesandordirs; Name: "{localappdata}\SparkyPDF"

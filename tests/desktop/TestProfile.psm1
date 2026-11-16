# Test profiles for the Windows desktop drivers (ADR-027).
#
# A desktop test never runs sotyware against the user's own data. It runs a
# test-profile build (Cargo feature test-profile), which refuses to start
# unless SOTYWARE_TEST_PROFILE names a directory holding the marker that
# New-TestProfile writes, keeps every database, its webview data and its
# credentials inside that directory (Credential Manager targets
# com.sotyware.desktop.test/<nonce>/...), and writes an acknowledgement once
# its stores are open.
#
# Why the driver must not compute the app's paths itself: on 2026-11-14 the
# driver ran inside the Claude desktop app's MSIX package, where
# %LOCALAPPDATA% is virtualized. The driver moved its (virtual) copy of the
# account database aside; the app then opened the user's real database
# through the merged view, the driver acted on the real account, and its
# cleanup deleted the real file. See docs/status/status.md.
#
# Wait-TestProfileAck compares the app's acknowledgement with what the
# driver set up and launched (run, nonce, process id, identifier, every path
# inside the profile, the credential prefix) and throws otherwise; drivers
# call it right after each launch, before they touch the window.
# Remove-TestProfile and Remove-TestCredential delete only what this run
# created. ASCII only (Windows PowerShell 5.1 reads BOM-less files as ANSI).
#
# Self-test without the app: tests/desktop/test-profile-selftest.ps1.

Set-StrictMode -Version 2

$script:Env = "SOTYWARE_TEST_PROFILE"
$script:MarkerFile = "sotyware-test-profile.json"
$script:AckFile = "sotyware-test-profile-ack.json"
# The databases the app reports in its acknowledgement (storage.rs).
$script:Stores = @("preferences.sqlite3", "cloud-accounts.sqlite3", "cloud-places.sqlite3", "operations-journal.sqlite3", "backup.sqlite3", "account.sqlite3", "updates.sqlite3", "window-state.sqlite3", "storage-usage.sqlite3")
$script:Purpose = "sotyware-desktop-test-profile"
$script:BuildMarker = "sotyware-test-profile-build"
$script:Identifier = "com.sotyware.desktop.test"
$script:NormalIdentifier = "com.sotyware.desktop"
$script:CredentialService = "com.sotyware.desktop.test"
$script:ExitRefused = 3

function Get-TestProfileConstants {
  [pscustomobject]@{
    Env = $script:Env; MarkerFile = $script:MarkerFile; AckFile = $script:AckFile; Stores = $script:Stores
    Purpose = $script:Purpose; BuildMarker = $script:BuildMarker; Identifier = $script:Identifier
    NormalIdentifier = $script:NormalIdentifier; CredentialService = $script:CredentialService
    ExitRefused = $script:ExitRefused
  }
}

# A comparable spelling of a path: no \\?\ prefix, full, no trailing
# separator, lower case (Windows file names ignore case).
function ConvertTo-ComparablePath([string]$Path) {
  $p = $Path
  if ($p.StartsWith('\\?\UNC\')) { $p = '\\' + $p.Substring(8) }
  elseif ($p.StartsWith('\\?\')) { $p = $p.Substring(4) }
  $p = [System.IO.Path]::GetFullPath($p)
  $p.TrimEnd('\').ToLowerInvariant()
}

function Test-PathWithin([string]$Path, [string]$Base) {
  $p = ConvertTo-ComparablePath $Path; $b = ConvertTo-ComparablePath $Base
  ($p -eq $b) -or $p.StartsWith($b + '\')
}

# The app's own folders (the real identifier), as this process sees them.
function Get-NormalProfileDirs {
  @(
    (Join-Path ([Environment]::GetFolderPath("LocalApplicationData")) $script:NormalIdentifier),
    (Join-Path ([Environment]::GetFolderPath("ApplicationData")) $script:NormalIdentifier)
  )
}

function Test-TestProfileBuild([string]$Exe) {
  $bytes = [System.IO.File]::ReadAllBytes($Exe)
  $text = [System.Text.Encoding]::ASCII.GetString($bytes)
  $text.Contains($script:BuildMarker)
}

function New-TestProfile([string]$Parent, [string]$Run) {
  if (-not ($Run -match '^[A-Za-z0-9_-]{1,64}$')) { throw "bad run id '$Run'" }
  # Checked before anything is created.
  $parentFull = [System.IO.Path]::GetFullPath($Parent)
  foreach ($n in Get-NormalProfileDirs) {
    if ((Test-PathWithin $parentFull $n) -or (Test-PathWithin $n $parentFull)) {
      throw "refusing a profile parent that overlaps the app's own folder $n"
    }
  }
  if (-not (Test-Path -LiteralPath $parentFull)) { New-Item -ItemType Directory -Path $parentFull | Out-Null }
  $parentFull = (Resolve-Path -LiteralPath $parentFull).ProviderPath
  $root = Join-Path $parentFull $Run
  if (Test-Path -LiteralPath $root) { throw "profile $root exists already; a profile is never reused" }
  New-Item -ItemType Directory -Path $root | Out-Null
  $bytes = New-Object byte[] 16
  $rng = [System.Security.Cryptography.RandomNumberGenerator]::Create()
  try { $rng.GetBytes($bytes) } finally { $rng.Dispose() }
  $nonce = -join ($bytes | ForEach-Object { $_.ToString("x2") })
  $json = '{"format":1,"purpose":"' + $script:Purpose + '","run":"' + $Run + '","nonce":"' + $nonce + '"}'
  [System.IO.File]::WriteAllText((Join-Path $root $script:MarkerFile), $json, (New-Object System.Text.UTF8Encoding($false)))
  [pscustomobject]@{ Parent = $parentFull; Root = $root; Run = $Run; Nonce = $nonce
    DataDir = (Join-Path $root "data"); TargetPrefix = "$($script:CredentialService)/$nonce/" }
}

# The profile a parent process created (and named in SOTYWARE_TEST_PROFILE),
# read back from its marker; for a driver that runs as a child process.
function Open-TestProfile([string]$Root) {
  $full = (Resolve-Path -LiteralPath $Root).ProviderPath
  $m = [System.IO.File]::ReadAllText((Join-Path $full $script:MarkerFile)) | ConvertFrom-Json
  if ($m.purpose -ne $script:Purpose -or -not ($m.nonce -cmatch '^[0-9a-f]{32}$') -or (Split-Path -Leaf $full) -cne $m.run) {
    throw "$full is not a test profile created by New-TestProfile"
  }
  [pscustomobject]@{ Parent = (Split-Path -Parent $full); Root = $full; Run = $m.run; Nonce = $m.nonce
    DataDir = (Join-Path $full "data"); TargetPrefix = "$($script:CredentialService)/$($m.nonce)/" }
}

# The Credential Manager target name of an account's entry in this profile.
function Get-TestCredentialTarget($Profile, [string]$AccountId) {
  "$($Profile.TargetPrefix)cloud-account/$AccountId"
}

# Throws unless $Ack (parsed JSON) is the acknowledgement of this launch.
function Assert-TestProfileAck($Profile, $Ack, [int]$ProcessId) {
  function Need([bool]$ok, [string]$what) { if (-not $ok) { throw "acknowledgement: $what" } }
  Need ($null -ne $Ack) "missing"
  Need (($Ack.format -eq 1) -and ($Ack.purpose -eq $script:Purpose)) "wrong format or purpose"
  Need ($Ack.build -eq $script:BuildMarker) "not from a test-profile build"
  Need ($Ack.run -ceq $Profile.Run) "run '$($Ack.run)', expected '$($Profile.Run)'"
  Need ($Ack.nonce -ceq $Profile.Nonce) "nonce differs from this profile's"
  Need ([int]$Ack.pid -eq $ProcessId) "pid $($Ack.pid), launched $ProcessId"
  Need ($Ack.identifier -ceq $script:Identifier) "identifier '$($Ack.identifier)'"
  Need ($Ack.credential_service -ceq $script:CredentialService) "credential service '$($Ack.credential_service)'"
  Need ($Ack.credential_target_prefix -ceq $Profile.TargetPrefix) "credential prefix '$($Ack.credential_target_prefix)'"
  Need ((ConvertTo-ComparablePath ([string]$Ack.root)) -eq (ConvertTo-ComparablePath $Profile.Root)) "root '$($Ack.root)'"
  $stores = @($Ack.stores)
  Need ($stores.Count -eq $script:Stores.Count) "$($script:Stores.Count) stores"
  $names = @($stores | ForEach-Object { [System.IO.Path]::GetFileName([string]$_) } | Sort-Object)
  Need (($names -join "|") -ceq ((@($script:Stores) | Sort-Object) -join "|")) "stores $($stores -join ', ')"
  foreach ($p in @($Ack.data_dir, $Ack.webview_dir) + $stores) {
    Need (($p -is [string]) -and (Test-PathWithin $p $Profile.Root)) "'$p' is outside the profile"
    foreach ($n in Get-NormalProfileDirs) { Need (-not (Test-PathWithin $p $n)) "'$p' is in the app's own folder" }
  }
}

function Clear-TestProfileAck($Profile) {
  $p = Join-Path $Profile.Root $script:AckFile
  if (-not (Test-PathWithin $p $Profile.Root)) { throw "$p is outside the profile" }
  if (Test-Path -LiteralPath $p) { Remove-Item -LiteralPath $p }
}

# Waits for this launch's acknowledgement and checks it; throws on anything
# else, also as soon as $Process (if given) has ended.
function Wait-TestProfileAck($Profile, [int]$ProcessId, [double]$Timeout = 30, $Process = $null) {
  $path = Join-Path $Profile.Root $script:AckFile
  $deadline = (Get-Date).AddSeconds($Timeout)
  $last = "no acknowledgement yet"
  while ((Get-Date) -lt $deadline) {
    if (Test-Path -LiteralPath $path) {
      try {
        $ack = [System.IO.File]::ReadAllText($path) | ConvertFrom-Json
        if ([int]$ack.pid -eq $ProcessId) { Assert-TestProfileAck $Profile $ack $ProcessId; return $ack }
        $last = "acknowledgement is from pid $($ack.pid), not $ProcessId"
      } catch [System.IO.IOException] { $last = "acknowledgement unreadable: $_" }
    }
    if ($Process -and $Process.HasExited) { throw "the app ended (exit $($Process.ExitCode)) before acknowledging its profile ($last)" }
    Start-Sleep -Milliseconds 100
  }
  throw "no acknowledgement from pid $ProcessId in $Timeout s ($last)"
}

# Deletes the profile and nothing else: only a directory directly under the
# parent it was created in, named like the run, whose marker holds its nonce.
function Remove-TestProfile($Profile) {
  $root = $Profile.Root
  $item = Get-Item -LiteralPath $root -Force -EA Stop
  if ($item.Attributes -band [System.IO.FileAttributes]::ReparsePoint) { throw "refusing to remove ${root}: a link" }
  if ((ConvertTo-ComparablePath (Split-Path -Parent $root)) -ne (ConvertTo-ComparablePath $Profile.Parent) -or (Split-Path -Leaf $root) -cne $Profile.Run) {
    throw "refusing to remove ${root}: not this run's profile directory"
  }
  $markerPath = Join-Path $root $script:MarkerFile
  if (-not (Test-Path -LiteralPath $markerPath)) { throw "refusing to remove ${root}: no marker" }
  $m = [System.IO.File]::ReadAllText($markerPath) | ConvertFrom-Json
  if ($m.nonce -cne $Profile.Nonce -or $m.purpose -ne $script:Purpose) { throw "refusing to remove ${root}: the marker is not this run's" }
  foreach ($n in Get-NormalProfileDirs) { if ((Test-PathWithin $root $n) -or (Test-PathWithin $n $root)) { throw "refusing to remove ${root}: overlaps $n" } }
  # .NET, not Remove-Item: test file names may hold unpaired surrogates.
  [System.IO.Directory]::Delete($root, $true)
}

# cmdkey lines of sotyware's own credentials (the real identifier only).
function Get-NormalCredentialLines {
  @(cmdkey /list | ForEach-Object { $_.Trim() } | Where-Object { $_ -like "Target:*target=$($script:NormalIdentifier)/*" })
}

# cmdkey lines of this profile's credentials.
function Get-TestCredentialLines($Profile) {
  @(cmdkey /list | ForEach-Object { $_.Trim() } | Where-Object { $_ -like "Target:*target=$($Profile.TargetPrefix)*" })
}

# Deletes one Credential Manager entry, only if it belongs to this profile.
function Remove-TestCredential($Profile, [string]$Target) {
  if (-not $Target.StartsWith($Profile.TargetPrefix, [System.StringComparison]::Ordinal) -or $Target.Contains('..')) {
    throw "refusing to delete credential '$Target': not in this profile's namespace $($Profile.TargetPrefix)"
  }
  cmdkey "/delete:$Target" | Out-Null
}

# Removes every entry of this profile (an aborted run may leave some).
function Remove-TestCredentials($Profile) {
  foreach ($line in Get-TestCredentialLines $Profile) {
    $t = ($line -split 'target=', 2)[1]
    if ($t) { Remove-TestCredential $Profile $t }
  }
}

# Names, sizes, write times and SHA-256 of every file in the app's own
# folders as this process sees them (read only).
function Get-NormalProfileFingerprint {
  $out = @()
  foreach ($d in Get-NormalProfileDirs) {
    if (-not (Test-Path -LiteralPath $d)) { $out += "$d absent"; continue }
    Get-ChildItem -LiteralPath $d -Recurse -Force -File -EA SilentlyContinue | Sort-Object FullName | ForEach-Object {
      $h = try { (Get-FileHash -LiteralPath $_.FullName -Algorithm SHA256 -EA Stop).Hash } catch { "locked" }
      $out += "$($_.FullName) $($_.Length) $($_.LastWriteTimeUtc.Ticks) $h"
    }
  }
  $out
}

# The same fingerprint, taken by a PowerShell that WMI starts outside this
# process tree: when this process runs inside an MSIX package (the Claude
# desktop app), %LOCALAPPDATA% is virtualized here but not there, so this is
# the user's real folder. Written to a file in $Dir (outside AppData). Returns
# $null if WMI could not start it.
function Get-RealNormalProfileFingerprint([string]$Dir) {
  $id = [guid]::NewGuid().ToString("n")
  $out = Join-Path $Dir "real-fingerprint-$id.txt"
  $script = @"
`$o = '$out'
`$lines = @()
foreach (`$d in @((Join-Path ([Environment]::GetFolderPath('LocalApplicationData')) '$($script:NormalIdentifier)'), (Join-Path ([Environment]::GetFolderPath('ApplicationData')) '$($script:NormalIdentifier)'))) {
  if (-not (Test-Path -LiteralPath `$d)) { `$lines += "`$d absent"; continue }
  Get-ChildItem -LiteralPath `$d -Recurse -Force -File -EA SilentlyContinue | Sort-Object FullName | ForEach-Object {
    `$h = try { (Get-FileHash -LiteralPath `$_.FullName -Algorithm SHA256 -EA Stop).Hash } catch { 'locked' }
    `$lines += "`$(`$_.FullName) `$(`$_.Length) `$(`$_.LastWriteTimeUtc.Ticks) `$h"
  }
}
[System.IO.File]::WriteAllLines("`$o.tmp", [string[]]`$lines)
[System.IO.File]::Move("`$o.tmp", `$o)
"@
  $enc = [Convert]::ToBase64String([System.Text.Encoding]::Unicode.GetBytes($script))
  try {
    $r = Invoke-CimMethod -ClassName Win32_Process -MethodName Create -Arguments @{ CommandLine = "powershell.exe -NoProfile -NonInteractive -EncodedCommand $enc" } -EA Stop
    if ($r.ReturnValue -ne 0) { return $null }
  } catch { return $null }
  for ($i = 0; $i -lt 300 -and -not (Test-Path -LiteralPath $out); $i++) { Start-Sleep -Milliseconds 100 }
  if (-not (Test-Path -LiteralPath $out)) { return $null }
  $lines = [System.IO.File]::ReadAllLines($out)
  Remove-Item -LiteralPath $out
  , $lines
}

Export-ModuleMember -Function Get-TestProfileConstants, ConvertTo-ComparablePath, Test-PathWithin,
  Get-NormalProfileDirs, Test-TestProfileBuild, New-TestProfile, Open-TestProfile, Get-TestCredentialTarget,
  Assert-TestProfileAck, Clear-TestProfileAck, Wait-TestProfileAck, Remove-TestProfile,
  Get-NormalCredentialLines, Get-TestCredentialLines, Remove-TestCredential, Remove-TestCredentials,
  Get-NormalProfileFingerprint, Get-RealNormalProfileFingerprint

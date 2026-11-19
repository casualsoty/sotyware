# Self-test of tests/desktop/TestProfile.psm1 (ADR-027): no app, no
# desktop, no credential written. Creates and removes folders under %TEMP%
# only.
#
#   powershell -NoProfile -ExecutionPolicy Bypass -File tests\desktop\test-profile-selftest.ps1
#
# Exit 0 when every check passed.

$ErrorActionPreference = "Stop"
Import-Module (Join-Path $PSScriptRoot "TestProfile.psm1") -Force
$C = Get-TestProfileConstants
$script:pass = 0; $script:fail = 0
function Check([string]$name, [bool]$ok, [string]$detail = "") {
  if ($ok) { $script:pass++; "PASS  $name" } else { $script:fail++; "FAIL  $name  $detail" }
}
# Not $b: the blocks given to it read the script's $b (the second profile),
# and inside Throws that name would be its own parameter, the block itself
# (check-ps-variables.ps1 checks for this).
function Throws([scriptblock]$attempt) { try { & $attempt; $false } catch { $true } }

$parent = Join-Path $env:TEMP ("sotyware-profile-selftest-" + [guid]::NewGuid().ToString("n"))
New-Item -ItemType Directory $parent | Out-Null
try {
  $a = New-TestProfile $parent "r1"; $b = New-TestProfile $parent "r2"
  Check "a profile gets a fresh 32-digit nonce" (($a.Nonce -cmatch '^[0-9a-f]{32}$') -and ($a.Nonce -ne $b.Nonce))
  $m = [System.IO.File]::ReadAllText((Join-Path $a.Root $C.MarkerFile))
  Check "the marker has no byte order mark and names run and nonce" (($m[0] -ne [char]0xFEFF) -and (($m | ConvertFrom-Json).nonce -eq $a.Nonce) -and (($m | ConvertFrom-Json).run -eq "r1"))
  Check "a profile is never reused" (Throws { New-TestProfile $parent "r1" })
  Check "a bad run id is refused" ((Throws { New-TestProfile $parent "a\b" }) -and (Throws { New-TestProfile $parent ".." }) -and (Throws { New-TestProfile $parent "" }))
  $normal = (Get-NormalProfileDirs)[0]
  Check "a parent inside the app's own folder is refused (nothing created there)" ((Throws { New-TestProfile (Join-Path $normal "x") "r" }) -and -not (Test-Path (Join-Path $normal "x")))
  Check "the credential target is in the test namespace" ((Get-TestCredentialTarget $a "acc") -ceq "com.sotyware.desktop.test/$($a.Nonce)/cloud-account/acc")
  Check "no test target starts with the app's own prefix" (-not (Get-TestCredentialTarget $a "acc").StartsWith("com.sotyware.desktop/"))

  function AckFor($p, [int]$pid_, [hashtable]$change = @{}) {
    $root = "\\?\" + $p.Root
    $h = [ordered]@{ format = 1; purpose = $C.Purpose; build = $C.BuildMarker; run = $p.Run; nonce = $p.Nonce; pid = $pid_
      identifier = $C.Identifier; root = $root; data_dir = "$root\data"; webview_dir = "$root\webview"
      credential_service = $C.CredentialService; credential_target_prefix = $p.TargetPrefix
      stores = @($C.Stores | ForEach-Object { "$root\data\$_" }) }
    foreach ($k in $change.Keys) { $h[$k] = $change[$k] }
    ([pscustomobject]$h | ConvertTo-Json) | ConvertFrom-Json
  }
  Check "a matching acknowledgement (with \\?\ paths, other case) passes" (-not (Throws { Assert-TestProfileAck $a (AckFor $a 42 @{ root = ("\\?\" + $a.Root).ToUpperInvariant() }) 42 }))
  $bad = @(
    @{ pid = 43 }, @{ nonce = $b.Nonce }, @{ run = "r2" }, @{ identifier = $C.NormalIdentifier }, @{ build = "x" },
    @{ purpose = "x" }, @{ format = 2 }, @{ credential_service = $C.NormalIdentifier },
    @{ credential_target_prefix = "com.sotyware.desktop/" }, @{ root = $b.Root },
    @{ data_dir = (Join-Path $normal "data") }, @{ webview_dir = $normal },
    @{ stores = @((Join-Path $normal "cloud-accounts.sqlite3"), "x", "y") }, @{ stores = @() }
  )
  $refused = 0
  foreach ($case in $bad) { if (Throws { Assert-TestProfileAck $a (AckFor $a 42 $case) 42 }) { $refused++ } else { "  not refused: $($case.Keys) = $($case.Values)" } }
  Check "every mismatch in the acknowledgement is refused ($($bad.Count) cases)" ($refused -eq $bad.Count)

  Check "no acknowledgement: the wait fails" (Throws { Wait-TestProfileAck $a 7 0.3 })
  [System.IO.File]::WriteAllText((Join-Path $a.Root $C.AckFile), ((AckFor $a 6) | ConvertTo-Json))
  Check "an earlier launch's acknowledgement does not count" (Throws { Wait-TestProfileAck $a 7 0.3 })
  [System.IO.File]::WriteAllText((Join-Path $a.Root $C.AckFile), ((AckFor $a 7) | ConvertTo-Json))
  Check "this launch's acknowledgement is accepted" ((Wait-TestProfileAck $a 7 2).pid -eq 7)
  Clear-TestProfileAck $a
  Check "the acknowledgement can be cleared" (-not (Test-Path (Join-Path $a.Root $C.AckFile)))

  Check "a credential outside the profile's namespace is never deleted" ((Throws { Remove-TestCredential $a "com.sotyware.desktop/cloud-account/real" }) -and (Throws { Remove-TestCredential $a "$($b.TargetPrefix)cloud-account/x" }) -and (Throws { Remove-TestCredential $a "$($a.TargetPrefix)../x" }))

  $keep = Join-Path $parent "keep.txt"; Set-Content $keep "k"
  $forged = [pscustomobject]@{ Parent = $parent; Root = $parent; Run = (Split-Path -Leaf $parent); Nonce = $a.Nonce }
  Check "a forged profile pointing at the parent is not removed" ((Throws { Remove-TestProfile $forged }) -and (Test-Path $keep))
  $other = [pscustomobject]@{ Parent = $parent; Root = $b.Root; Run = "r2"; Nonce = $a.Nonce }
  Check "a profile with another nonce is not removed" ((Throws { Remove-TestProfile $other }) -and (Test-Path $b.Root))
  Remove-Item -LiteralPath (Join-Path $b.Root $C.MarkerFile)
  Check "a profile without its marker is not removed" ((Throws { Remove-TestProfile $b }) -and (Test-Path $b.Root))
  Remove-TestProfile $a
  Check "this run's profile is removed, nothing else" ((-not (Test-Path $a.Root)) -and (Test-Path $keep) -and (Test-Path $b.Root))

  $f1 = Get-NormalProfileFingerprint; $f2 = Get-NormalProfileFingerprint
  Check "the normal profile's fingerprint is stable when nothing changes" (($f1 -join "|") -eq ($f2 -join "|"))
  $bin = Join-Path $parent "bin"; [System.IO.File]::WriteAllBytes($bin, [byte[]](1..100))
  $noMarker = -not (Test-TestProfileBuild $bin)
  [System.IO.File]::WriteAllBytes($bin, [System.Text.Encoding]::ASCII.GetBytes("xx$($C.BuildMarker)xx"))
  Check "a test-profile build is told apart from a normal build" ($noMarker -and (Test-TestProfileBuild $bin))
} finally {
  [System.IO.Directory]::Delete($parent, $true)
}
""
"summary: $script:pass passed, $script:fail failed"
if ($script:fail -gt 0) { exit 1 } else { exit 0 }

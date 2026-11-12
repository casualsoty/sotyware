# Static check of the Windows test scripts: PowerShell variable names ignore
# case, so `$out` and `$Out` (or `$a` and the script's `$A`) are one variable.
# That clash broke a real-provider run of windows-cloud-ui.ps1 on 2026-11-12.
# This parses each script (it runs nothing, opens no window, sends no input)
# and fails if one scope spells a variable in more than one way.
#
# Scopes: the script itself, and each function. A variable a function
# assigns (or takes as a parameter) belongs to that function; one it only
# reads belongs to the script, as at run time. `$script:x` is the script's.
# Script blocks that are not functions (WaitFor, ForEach-Object) are counted
# with the scope they appear in. Automatic variables ($_, $true, $null, ...)
# and drive variables ($env:x) are ignored.
#
# usage: powershell -NoProfile -ExecutionPolicy Bypass -File tests\desktop\check-ps-variables.ps1 [<script.ps1> ...]
#   (default: windows.ps1, windows-isolated.ps1, windows-cloud-ui.ps1)
# Exit code: 0 no clash, 1 a clash or a parse error.
param([Parameter(ValueFromRemainingArguments = $true)][string[]]$Paths)
$ErrorActionPreference = "Stop"
if (-not $Paths) { $Paths = "windows.ps1", "windows-isolated.ps1", "windows-cloud-ui.ps1" | ForEach-Object { Join-Path $PSScriptRoot $_ } }

$Automatic = @("_", "true", "false", "null", "args", "input", "matches", "this", "error", "host",
  "lastexitcode", "psscriptroot", "pscommandpath", "psitem", "erroractionpreference", "pwd", "home", "pid")

# The function that owns `$ast`, or $null for the script.
function OwnerOf($ast) {
  for ($p = $ast.Parent; $p; $p = $p.Parent) {
    if ($p -is [System.Management.Automation.Language.FunctionDefinitionAst]) { return $p }
  }
  $null
}
# Variables an assignment, foreach or parameter introduces.
function AssignedIn($fn) {
  $names = @{}
  $targets = $fn.Body.FindAll({ param($n)
      ($n -is [System.Management.Automation.Language.AssignmentStatementAst]) -or
      ($n -is [System.Management.Automation.Language.ForEachStatementAst]) -or
      ($n -is [System.Management.Automation.Language.ParameterAst]) }, $true)
  foreach ($t in @($targets) + @($fn.Parameters | Where-Object { $_ })) {
    $v = if ($t -is [System.Management.Automation.Language.AssignmentStatementAst]) { $t.Left }
      elseif ($t -is [System.Management.Automation.Language.ForEachStatementAst]) { $t.Variable }
      else { $t.Name }
    if ($v -is [System.Management.Automation.Language.ConvertExpressionAst]) { $v = $v.Child }
    if ($v -is [System.Management.Automation.Language.VariableExpressionAst] -and -not $v.VariablePath.IsDriveQualified -and -not $v.VariablePath.IsScript -and (OwnerOf $v) -eq $fn) {
      $names[$v.VariablePath.UserPath.ToLowerInvariant()] = $true
    }
  }
  $names
}

$clashes = 0
foreach ($path in $Paths) {
  $tokens = $null; $errors = $null
  $ast = [System.Management.Automation.Language.Parser]::ParseFile((Resolve-Path $path).ProviderPath, [ref]$tokens, [ref]$errors)
  if ($errors.Count -gt 0) { "FAIL  $path does not parse: $($errors[0].Message) (line $($errors[0].Extent.StartLineNumber))"; $clashes++; continue }
  $locals = @{}
  # scope -> lower-case name -> spelling -> first line
  $seen = @{}
  foreach ($v in $ast.FindAll({ param($n) $n -is [System.Management.Automation.Language.VariableExpressionAst] }, $true)) {
    $vp = $v.VariablePath
    if ($vp.IsDriveQualified) { continue }
    $name = $vp.UserPath
    if ($vp.IsScript -or $vp.IsGlobal -or $vp.IsPrivate -or $vp.IsLocal) { $name = $name.Substring($name.IndexOf(":") + 1) }
    $key = $name.ToLowerInvariant()
    if ($Automatic -contains $key) { continue }
    $scope = "script"
    $fn = if ($vp.IsScript -or $vp.IsGlobal) { $null } else { OwnerOf $v }
    if ($fn) {
      if (-not $locals.ContainsKey($fn)) { $locals[$fn] = AssignedIn $fn }
      if ($locals[$fn].ContainsKey($key)) { $scope = "function $($fn.Name)" }
    }
    if (-not $seen.ContainsKey($scope)) { $seen[$scope] = @{} }
    # Case-sensitive on purpose: PowerShell's own @{} and [ordered]@{} ignore
    # case, which would merge exactly the spellings looked for here.
    if (-not $seen[$scope].ContainsKey($key)) { $seen[$scope][$key] = New-Object 'System.Collections.Generic.Dictionary[string,int]' ([StringComparer]::Ordinal) }
    if (-not $seen[$scope][$key].ContainsKey($name)) { $seen[$scope][$key][$name] = $v.Extent.StartLineNumber }
  }
  $here = 0
  foreach ($scope in $seen.Keys | Sort-Object) {
    foreach ($key in $seen[$scope].Keys | Sort-Object) {
      $spellings = $seen[$scope][$key]
      if ($spellings.Count -gt 1) {
        $here++
        "FAIL  $(Split-Path -Leaf $path), ${scope}: one variable spelled " + (($spellings.Keys | ForEach-Object { "`$$_ (line $($spellings[$_]))" }) -join ", ")
      }
    }
  }
  if ($here -eq 0) { "PASS  $(Split-Path -Leaf $path): no variable spelled two ways in one scope" }
  $clashes += $here
}
if ($clashes -gt 0) { exit 1 } else { exit 0 }

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
# It also fails where a name is hidden at run time. PowerShell looks a
# variable up through the scopes of the callers, not where the code was
# written, so a function's own variable (parameter, assignment, foreach)
# hides the script's variable of that name from every function it calls,
# and a script block given to a function sees that function's own variables
# first. Both broke windows-autoclicker.ps1 before its first run (found on a
# private desktop, 2026-11-19): Says([string]$s) hid $S (TreeScope) from
# Names, so every Says read an empty page; and a block given to WaitFor that
# called $pred called WaitFor's own $pred, itself, until the call depth
# overflowed.
#
# usage: powershell -NoProfile -ExecutionPolicy Bypass -File tests\desktop\check-ps-variables.ps1 [<script.ps1> ...]
#   (default: windows.ps1, windows-isolated.ps1, windows-cloud-ui.ps1, windows-autoclicker.ps1)
# Exit code: 0 no clash, 1 a clash, a hidden name or a parse error.
param([Parameter(ValueFromRemainingArguments = $true)][string[]]$Paths)
$ErrorActionPreference = "Stop"
if (-not $Paths) { $Paths = "windows.ps1", "windows-isolated.ps1", "windows-cloud-ui.ps1", "windows-autoclicker.ps1" | ForEach-Object { Join-Path $PSScriptRoot $_ } }

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

# ---- names hidden at run time ----
# The innermost function or script block whose scope `$node` is in.
function ScopeOf($node) {
  for ($p = $node.Parent; $p; $p = $p.Parent) {
    if ($p -is [System.Management.Automation.Language.ScriptBlockExpressionAst] -or
        $p -is [System.Management.Automation.Language.FunctionDefinitionAst]) { return $p }
  }
  $null
}
function BodyOf($scope) { if ($scope -is [System.Management.Automation.Language.FunctionDefinitionAst]) { $scope.Body } else { $scope.ScriptBlock } }
# Lower-case names a function or script block makes its own: its parameters,
# and what it assigns or loops over itself (not inside a block within it).
function OwnedBy($scope) {
  $names = @{}
  $body = BodyOf $scope
  $params = @()
  if ($scope -is [System.Management.Automation.Language.FunctionDefinitionAst] -and $scope.Parameters) { $params += $scope.Parameters }
  if ($body.ParamBlock) { $params += $body.ParamBlock.Parameters }
  foreach ($pa in $params) { $names[$pa.Name.VariablePath.UserPath.ToLowerInvariant()] = $true }
  $made = $body.FindAll({ param($n)
      ($n -is [System.Management.Automation.Language.AssignmentStatementAst]) -or
      ($n -is [System.Management.Automation.Language.ForEachStatementAst]) }, $true)
  foreach ($t in @($made)) {
    if ((ScopeOf $t) -ne $scope) { continue }
    $v = if ($t -is [System.Management.Automation.Language.AssignmentStatementAst]) { $t.Left } else { $t.Variable }
    if ($v -is [System.Management.Automation.Language.ConvertExpressionAst]) { $v = $v.Child }
    if ($v -is [System.Management.Automation.Language.VariableExpressionAst]) {
      $vp = $v.VariablePath
      if (-not ($vp.IsDriveQualified -or $vp.IsScript -or $vp.IsGlobal)) { $names[$vp.UserPath.ToLowerInvariant()] = $true }
    }
  }
  $names
}
# Lower-case names a function or block reads that neither it nor a block
# around the read owns: at run time they come from its callers' scopes.
# `$ownedOf` caches OwnedBy per function or block.
function FreeIn($scope, $ownedOf) {
  $free = @{}
  foreach ($v in @((BodyOf $scope).FindAll({ param($n) $n -is [System.Management.Automation.Language.VariableExpressionAst] }, $true))) {
    $vp = $v.VariablePath
    if ($vp.IsDriveQualified -or $vp.IsScript -or $vp.IsGlobal) { continue }
    $key = $vp.UserPath.ToLowerInvariant()
    if ($Automatic -contains $key) { continue }
    $inner = $false
    for ($around = ScopeOf $v; $around; $around = ScopeOf $around) {
      if (-not $ownedOf.ContainsKey($around)) { $ownedOf[$around] = OwnedBy $around }
      if ($ownedOf[$around].ContainsKey($key)) { $inner = $true; break }
      if ($around -eq $scope) { break }
    }
    if (-not $inner) { $free[$key] = $true }
  }
  $free
}
# The script's functions that `$node` calls (lower-case names).
function CallsIn($node, $fns) {
  @($node.FindAll({ param($n) $n -is [System.Management.Automation.Language.CommandAst] }, $true) |
    ForEach-Object { $_.GetCommandName() } | Where-Object { $_ -and $fns.ContainsKey($_.ToLowerInvariant()) } |
    ForEach-Object { $_.ToLowerInvariant() } | Sort-Object -Unique)
}
# One line per name a function hides from what it calls, and per block that
# reads a name the function it is given to owns.
function HiddenNames($ast) {
  $fns = @{}
  foreach ($f in @($ast.FindAll({ param($n) $n -is [System.Management.Automation.Language.FunctionDefinitionAst] }, $true))) { $fns[$f.Name.ToLowerInvariant()] = $f }
  $ownedOf = @{}; $calls = @{}; $reach = @{}
  foreach ($k in @($fns.Keys)) { $ownedOf[$fns[$k]] = OwnedBy $fns[$k] }
  foreach ($k in @($fns.Keys)) {
    $calls[$k] = CallsIn $fns[$k].Body $fns
    # name -> the function that reads it from its callers
    $reach[$k] = @{}; foreach ($x in @((FreeIn $fns[$k] $ownedOf).Keys)) { $reach[$k][$x] = $k }
  }
  # What a call reads from its callers, through what it calls in turn.
  do {
    $grew = $false
    foreach ($k in @($fns.Keys)) {
      foreach ($c in $calls[$k]) {
        foreach ($x in @($reach[$c].Keys)) {
          if (-not $ownedOf[$fns[$k]].ContainsKey($x) -and -not $reach[$k].ContainsKey($x)) { $reach[$k][$x] = $reach[$c][$x]; $grew = $true }
        }
      }
    }
  } while ($grew)
  $found = @()
  foreach ($k in @($fns.Keys | Sort-Object)) {
    foreach ($c in $calls[$k]) {
      foreach ($x in @($ownedOf[$fns[$k]].Keys | Sort-Object)) {
        if ($reach[$c].ContainsKey($x)) {
          $found += "function $($fns[$k].Name) (line $($fns[$k].Extent.StartLineNumber)) has its own `$$x, which hides the script's from $($fns[$reach[$c][$x]].Name) (called through $($fns[$c].Name))"
        }
      }
    }
  }
  foreach ($cmd in @($ast.FindAll({ param($n) $n -is [System.Management.Automation.Language.CommandAst] }, $true))) {
    $name = $cmd.GetCommandName()
    if (-not $name -or -not $fns.ContainsKey($name.ToLowerInvariant())) { continue }
    $g = $fns[$name.ToLowerInvariant()]
    foreach ($el in $cmd.CommandElements) {
      if ($el -isnot [System.Management.Automation.Language.ScriptBlockExpressionAst]) { continue }
      if (-not $ownedOf.ContainsKey($el)) { $ownedOf[$el] = OwnedBy $el }
      $reads = FreeIn $el $ownedOf
      foreach ($c in (CallsIn $el $fns)) { foreach ($x in @($reach[$c].Keys)) { if (-not $ownedOf[$el].ContainsKey($x)) { $reads[$x] = $true } } }
      foreach ($x in @($reads.Keys | Sort-Object)) {
        if ($ownedOf[$g].ContainsKey($x)) { $found += "a block given to $($g.Name) on line $($el.Extent.StartLineNumber) reads `$$x, which is $($g.Name)'s own there" }
      }
    }
  }
  $found
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
  $hidden = @(HiddenNames $ast)
  foreach ($h in $hidden) { "FAIL  $(Split-Path -Leaf $path): $h" }
  if ($hidden.Count -eq 0) { "PASS  $(Split-Path -Leaf $path): no name hidden from a function or block at run time" }
  $clashes += $hidden.Count
}
if ($clashes -gt 0) { exit 1 } else { exit 0 }

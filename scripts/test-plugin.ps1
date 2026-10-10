#requires -Version 7
# Plugin seam: runs the Lua tests in plugin/tests in a headless Aseprite. Needs $env:ASEPRITE.
$ErrorActionPreference = 'Stop'
if (-not $env:ASEPRITE) { throw 'Set ASEPRITE to the aseprite executable path.' }
$plugin = Join-Path $PSScriptRoot '..\plugin' | Resolve-Path
foreach ($test in Get-ChildItem "$plugin\tests\test_*.lua") {
  $log = New-TemporaryFile
  # aseprite.exe is a GUI-subsystem binary: only Start-Process -Wait reliably waits and captures stdout.
  $p = Start-Process $env:ASEPRITE -Wait -PassThru -NoNewWindow -RedirectStandardOutput $log -ArgumentList @(
    '--batch', '--script-param', "executor=$plugin\executor.lua", '--script-param', "settings=$plugin\settings.lua",
    '--script', $test.FullName)
  $out = Get-Content $log -Raw
  Remove-Item $log
  Write-Host $out
  if ($p.ExitCode -ne 0 -or $out -notmatch 'ALL PASS') { throw "Plugin tests failed: $($test.Name) (exit $($p.ExitCode))" }
}

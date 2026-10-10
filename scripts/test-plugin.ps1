#requires -Version 7
# Plugin seam: runs the Lua executor tests in a headless Aseprite. Needs $env:ASEPRITE.
$ErrorActionPreference = 'Stop'
if (-not $env:ASEPRITE) { throw 'Set ASEPRITE to the aseprite executable path.' }
$plugin = Join-Path $PSScriptRoot '..\plugin' | Resolve-Path
$log = New-TemporaryFile
# aseprite.exe is a GUI-subsystem binary: only Start-Process -Wait reliably waits and captures stdout.
$p = Start-Process $env:ASEPRITE -Wait -PassThru -NoNewWindow -RedirectStandardOutput $log -ArgumentList @(
  '--batch', '--script-param', "executor=$plugin\executor.lua", '--script', "$plugin\tests\test_executor.lua")
$out = Get-Content $log -Raw
Remove-Item $log
Write-Host $out
if ($p.ExitCode -ne 0 -or $out -notmatch 'ALL PASS') { throw "Plugin tests failed (exit $($p.ExitCode))" }

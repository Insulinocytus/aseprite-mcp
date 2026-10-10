#requires -Version 7
# Builds the release Server and zips it with the Plugin into dist/aseprite-mcp-<version>.aseprite-extension.
$ErrorActionPreference = 'Stop'
$root = Resolve-Path (Join-Path $PSScriptRoot '..')
$version = (Get-Content "$root\plugin\package.json" -Raw | ConvertFrom-Json).version

cargo build --release --manifest-path "$root\server\Cargo.toml"
if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }

$stage = Join-Path ([IO.Path]::GetTempPath()) "aseprite-mcp-$([guid]::NewGuid())"
New-Item -ItemType Directory $stage | Out-Null
try {
  Copy-Item "$root\plugin\package.json", "$root\plugin\plugin.lua", "$root\plugin\executor.lua" $stage
  Copy-Item "$root\server\target\release\aseprite-mcp-server.exe" $stage
  New-Item -ItemType Directory "$root\dist" -Force | Out-Null
  $out = "$root\dist\aseprite-mcp-$version.aseprite-extension"
  Remove-Item $out -ErrorAction SilentlyContinue
  [IO.Compression.ZipFile]::CreateFromDirectory($stage, $out)
  Write-Host $out
} finally {
  Remove-Item $stage -Recurse -Force
}

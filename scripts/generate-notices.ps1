#Requires -Version 7.0
[CmdletBinding()]
param([string] $OutputPath)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$temporaryPath = $null
Push-Location -LiteralPath $repoRoot
try {
    $toolVersion = & cargo about --version
    if ($LASTEXITCODE -ne 0 -or "$toolVersion" -ne 'cargo-about 0.9.2') {
        throw 'Install cargo-about 0.9.2: cargo install --locked --version 0.9.2 --features cli cargo-about'
    }
    if (-not $OutputPath) {
        $metadataJson = & cargo metadata --format-version 1 --no-deps --locked
        if ($LASTEXITCODE -ne 0) { throw 'Cargo metadata failed.' }
        $metadata = $metadataJson | ConvertFrom-Json
        $OutputPath = Join-Path $metadata.target_directory 'release/THIRD-PARTY-NOTICES.html'
    }
    $OutputPath = [IO.Path]::GetFullPath($OutputPath)
    $outputDirectory = [IO.Path]::GetDirectoryName($OutputPath)
    [IO.Directory]::CreateDirectory($outputDirectory) | Out-Null
    $temporaryPath = Join-Path $outputDirectory ('.notices-' + [guid]::NewGuid().ToString('N') + '.html')
    & cargo about generate --locked --workspace --fail --config about.toml --output-file $temporaryPath about.hbs
    if ($LASTEXITCODE -ne 0) { throw 'Dependency notice generation failed. Resolve the reported licenses before packaging.' }
    if (-not (Test-Path -LiteralPath $temporaryPath -PathType Leaf) -or
        (Get-Item -LiteralPath $temporaryPath).Length -lt 1024) {
        throw 'Dependency notice generation produced no usable document.'
    }
    Move-Item -LiteralPath $temporaryPath -Destination $OutputPath -Force
    Write-Host "Generated $OutputPath"
} finally {
    Pop-Location
    if ($temporaryPath -and (Test-Path -LiteralPath $temporaryPath -PathType Leaf)) {
        Remove-Item -LiteralPath $temporaryPath -Force
    }
}

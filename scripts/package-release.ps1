#Requires -Version 7.0
[CmdletBinding()]
param(
    # On tagged builds this must exactly match the workspace version.
    [string] $Tag,
    # Use only after building both release binaries from the current checkout.
    [switch] $SkipBuild
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

if (-not $IsWindows) {
    throw 'The release archive currently supports Windows x64 only.'
}

$repoRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$distRoot = Join-Path $repoRoot 'dist'
$stagingRoot = $null
Push-Location -LiteralPath $repoRoot
try {
    $rustInfo = & rustc -vV
    if ($LASTEXITCODE -ne 0) { throw 'Unable to read the Rust toolchain.' }
    if ($rustInfo -notcontains 'host: x86_64-pc-windows-msvc') {
        throw 'Use an x86_64-pc-windows-msvc Rust toolchain to package this release.'
    }

    $metadataJson = & cargo metadata --format-version 1 --no-deps --locked
    if ($LASTEXITCODE -ne 0) { throw 'Cargo metadata failed.' }
    $metadata = $metadataJson | ConvertFrom-Json
    $editor = @($metadata.packages | Where-Object name -EQ 'clockwork_editor')
    $agent = @($metadata.packages | Where-Object name -EQ 'clockwork_agent')
    if ($editor.Count -ne 1 -or $agent.Count -ne 1 -or $editor[0].version -ne $agent[0].version) {
        throw 'Editor and MCP server must have the same package version.'
    }
    $version = $editor[0].version
    if ($version -notmatch '^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$') {
        throw "Unsupported release version: $version"
    }
    if ($Tag -and $Tag -cne "v$version") {
        throw "Tag '$Tag' does not match workspace version 'v$version'."
    }

    $binaryPaths = @{}
    if (-not $SkipBuild) {
        $buildMessages = & cargo build --release --locked -p clockwork_editor -p clockwork_agent --message-format=json-render-diagnostics
        if ($LASTEXITCODE -ne 0) { throw 'Release build failed.' }
        foreach ($line in $buildMessages) {
            $message = $line | ConvertFrom-Json
            if ($message.reason -eq 'compiler-artifact' -and $message.executable) {
                $binaryPaths[$message.target.name + '.exe'] = $message.executable
            }
        }
    }

    $releaseRoot = Join-Path $metadata.target_directory 'release'
    $binaries = @('clockwork.exe', 'clockwork-mcp.exe')
    $documents = @(
        'README.md', 'CHANGELOG.md', 'DESIGN.md', 'CONTRIBUTING.md',
        'docs/RELEASING.md', 'LICENSE-MIT', 'LICENSE-APACHE'
    )
    foreach ($binary in $binaries) {
        $binaryPath = if ($SkipBuild) { Join-Path $releaseRoot $binary } else { $binaryPaths[$binary] }
        if (-not $binaryPath) { throw "Cargo did not produce $binary." }
        $binaryPaths[$binary] = $binaryPath
        if (-not (Test-Path -LiteralPath $binaryPath -PathType Leaf)) {
            throw "Missing release binary: $binaryPath"
        }
        # Reject accidental cross-target or non-executable inputs, including with -SkipBuild.
        $reader = [IO.BinaryReader]::new([IO.File]::OpenRead($binaryPath))
        try {
            if ($reader.ReadUInt16() -ne 0x5A4D) { throw "Not a Windows executable: $binary" }
            $reader.BaseStream.Position = 0x3C
            $reader.BaseStream.Position = $reader.ReadInt32()
            if ($reader.ReadUInt32() -ne 0x00004550 -or $reader.ReadUInt16() -ne 0x8664) {
                throw "Not an x64 Windows executable: $binary"
            }
        } finally {
            $reader.Dispose()
        }
    }
    foreach ($document in $documents) {
        if (-not (Test-Path -LiteralPath (Join-Path $repoRoot $document) -PathType Leaf)) {
            throw "Missing release document: $document"
        }
    }

    if (Test-Path -LiteralPath $distRoot) {
        $distItem = Get-Item -LiteralPath $distRoot -Force
        if (-not $distItem.PSIsContainer -or ($distItem.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
            throw 'dist must be a normal directory inside the repository.'
        }
    } else {
        New-Item -ItemType Directory -Path $distRoot | Out-Null
    }
    $archiveName = "clockwork-$version-windows-x86_64"
    $stagingRoot = Join-Path $distRoot ('.package-' + [guid]::NewGuid().ToString('N'))
    $packageRoot = Join-Path $stagingRoot $archiveName
    New-Item -ItemType Directory -Path $packageRoot | Out-Null
    foreach ($binary in $binaries) {
        Copy-Item -LiteralPath $binaryPaths[$binary] -Destination $packageRoot
    }
    foreach ($document in $documents) {
        $documentDestination = Join-Path $packageRoot $document
        New-Item -ItemType Directory -Path (Split-Path -Parent $documentDestination) -Force | Out-Null
        Copy-Item -LiteralPath (Join-Path $repoRoot $document) -Destination $documentDestination
    }
    $mcpConfig = @{ mcpServers = @{ clockwork = @{ type = 'stdio'; command = './clockwork-mcp.exe' } } } | ConvertTo-Json -Depth 4
    [IO.File]::WriteAllText((Join-Path $packageRoot '.mcp.json'), "$mcpConfig`n", [Text.UTF8Encoding]::new($false))
    & (Join-Path $PSScriptRoot 'generate-notices.ps1') -OutputPath (Join-Path $packageRoot 'THIRD-PARTY-NOTICES.html')
    if (-not (Test-Path -LiteralPath (Join-Path $packageRoot 'THIRD-PARTY-NOTICES.html') -PathType Leaf)) {
        throw 'Third-party notice generation did not produce a file.'
    }

    $temporaryArchive = Join-Path $stagingRoot "$archiveName.zip"
    [IO.Compression.ZipFile]::CreateFromDirectory(
        $packageRoot, $temporaryArchive, [IO.Compression.CompressionLevel]::Optimal, $true
    )
    $checksum = (Get-FileHash -LiteralPath $temporaryArchive -Algorithm SHA256).Hash.ToLowerInvariant()
    Copy-Item -LiteralPath $temporaryArchive -Destination (Join-Path $distRoot "$archiveName.zip") -Force
    [IO.File]::WriteAllText((Join-Path $distRoot 'SHA256SUMS.txt'), "$checksum  $archiveName.zip`n", [Text.UTF8Encoding]::new($false))
    Write-Host "Created dist/$archiveName.zip"
    Write-Host "SHA256: $checksum"
} finally {
    Pop-Location
    if ($stagingRoot -and (Test-Path -LiteralPath $stagingRoot)) {
        $resolvedStaging = (Resolve-Path -LiteralPath $stagingRoot).Path
        $allowedPrefix = [IO.Path]::GetFullPath($distRoot) + [IO.Path]::DirectorySeparatorChar
        $stagingItem = Get-Item -LiteralPath $resolvedStaging -Force
        if (-not $resolvedStaging.StartsWith($allowedPrefix, [StringComparison]::OrdinalIgnoreCase) -or
            ($stagingItem.Attributes -band [IO.FileAttributes]::ReparsePoint)) {
            throw "Refusing to remove staging directory outside dist: $resolvedStaging"
        }
        Remove-Item -LiteralPath $resolvedStaging -Recurse -Force
    }
}

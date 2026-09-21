param(
    [string]$InstallDir = $env:SYNAPTIC_INSTALL_DIR,
    [switch]$NoPathUpdate
)

$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"

if ($env:OS -ne "Windows_NT") {
    throw "This installer is for Windows. Use install.sh on macOS or Linux."
}
$architecture = if ($env:PROCESSOR_ARCHITEW6432) { $env:PROCESSOR_ARCHITEW6432 } else { $env:PROCESSOR_ARCHITECTURE }
if ($architecture -ne "AMD64") {
    throw "Synaptic has no prebuilt Windows release for $architecture."
}
if (-not $InstallDir) {
    if (-not $env:LOCALAPPDATA) { throw "LOCALAPPDATA is not set." }
    $InstallDir = Join-Path $env:LOCALAPPDATA "Programs\Synaptic"
}

$target = "x86_64-pc-windows-msvc"
$archive = "synaptic-$target.zip"
$base = "https://github.com/ColinVaughn/Synaptic/releases/latest/download"
$tempDir = Join-Path ([IO.Path]::GetTempPath()) ("synaptic-install-" + [guid]::NewGuid())

try {
    New-Item -ItemType Directory -Path $tempDir | Out-Null
    $archivePath = Join-Path $tempDir $archive
    $checksumPath = "$archivePath.sha256"
    Write-Host "Downloading the latest Synaptic release..."
    Invoke-WebRequest -UseBasicParsing "$base/$archive" -OutFile $archivePath
    Invoke-WebRequest -UseBasicParsing "$base/$archive.sha256" -OutFile $checksumPath

    $expected = ((Get-Content -Raw $checksumPath) -split '\s+')[0].ToLowerInvariant()
    $actual = (Get-FileHash -Algorithm SHA256 $archivePath).Hash.ToLowerInvariant()
    if ($expected.Length -ne 64 -or $actual -ne $expected) {
        throw "Checksum verification failed; nothing was installed."
    }

    Expand-Archive -LiteralPath $archivePath -DestinationPath $tempDir
    $bundle = Join-Path $tempDir "synaptic-$target"
    if (-not (Test-Path -LiteralPath (Join-Path $bundle "synaptic.exe"))) {
        throw "The release archive does not contain synaptic.exe."
    }
    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
    foreach ($name in @("synaptic.exe", "syn.exe", "synaptic-ui.exe")) {
        $source = Join-Path $bundle $name
        if (Test-Path -LiteralPath $source) {
            Copy-Item -Force -LiteralPath $source -Destination (Join-Path $InstallDir $name)
        }
    }

    if (-not $NoPathUpdate) {
        $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
        $entries = @($userPath -split ";" | Where-Object { $_ })
        $normalized = $InstallDir.TrimEnd("\")
        $present = $entries | Where-Object { $_.TrimEnd("\") -ieq $normalized }
        if (-not $present) {
            $newPath = (@($InstallDir) + $entries) -join ";"
            [Environment]::SetEnvironmentVariable("Path", $newPath, "User")
        }
        if (($env:Path -split ";") -notcontains $InstallDir) {
            $env:Path = "$InstallDir;$env:Path"
        }
    }

    & (Join-Path $InstallDir "synaptic.exe") --version
    Write-Host "Installed in $InstallDir"
    Write-Host "From a repository, run: synaptic extract .; synaptic install <assistant>"
}
finally {
    if (Test-Path -LiteralPath $tempDir) {
        Remove-Item -Recurse -Force -LiteralPath $tempDir
    }
}

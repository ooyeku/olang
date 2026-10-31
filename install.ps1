# The olang installer for Windows.
#
#   irm https://raw.githubusercontent.com/ooyeku/olang/main/install.ps1 | iex
#
# (https://raw.githubusercontent.com/ooyeku/olang/main/install.ps1 is the
# same file.) It downloads the ready-made olang for Windows from the
# newest GitHub release, checks the release's signature and the
# download's checksum, installs olang.exe and otc.exe into
# %USERPROFILE%\.olang\bin, and adds that folder to your PATH.
#
# Options, as environment variables set before the line above:
#   $env:OLANG_VERSION = 'v1.0.0'     install this release (default: the newest)
#   $env:OLANG_INSTALL_DIR = 'D:\bin' install there instead
#   $env:OLANG_NO_MODIFY_PATH = '1'   leave PATH alone
#   $env:OLANG_FORCE = '1'            reinstall the same version
#   $env:OLANG_QUIET = '1'            print nothing but errors
#   $env:OLANG_UNINSTALL = '1'        remove olang instead
# or as parameters when the script is saved first:
#   .\install.ps1 -Version v1.0.0 -Dir D:\bin -NoModifyPath -Force -Quiet -Uninstall
#
# It writes only to the install folder and your user PATH, and deletes
# its temporary files when it ends. It never asks for administrator
# rights. It works in Windows PowerShell 5.1 and PowerShell 7.

param(
    [string]$Version = $env:OLANG_VERSION,
    [string]$Dir = $env:OLANG_INSTALL_DIR,
    [switch]$NoModifyPath = ($env:OLANG_NO_MODIFY_PATH -eq '1'),
    [switch]$Force = ($env:OLANG_FORCE -eq '1'),
    [switch]$Quiet = ($env:OLANG_QUIET -eq '1'),
    [switch]$Uninstall = ($env:OLANG_UNINSTALL -eq '1'),
    [switch]$Help
)

$Repo = 'ooyeku/olang'
$GitHub = if ($env:OLANG_GITHUB) { $env:OLANG_GITHUB } else { 'https://github.com' }
$GitHubApi = if ($env:OLANG_GITHUB_API) { $env:OLANG_GITHUB_API } else { 'https://api.github.com' }
$HelpUrl = 'https://olang.dev/book/installation/'

# The public half of olang's release signing key (ECDSA P-256), the same
# key as install.sh's SIGNING_KEY. dist/signing/README.md says how it is
# made. Empty until signing is set up; then every release from
# $SignedFrom on must carry a good SHA256SUMS.sig.
$SigningKey = ''
$SignedFrom = ''

function Say([string]$Text = '') { if (-not $Quiet) { Write-Host $Text } }
function Row([string]$Label, [string]$Value) {
    if (-not $Quiet) { Write-Host ('  {0,-9} ' -f $Label) -NoNewline -ForegroundColor DarkGray; Write-Host $Value }
}
function Done([string]$Text) {
    if (-not $Quiet) { Write-Host '  ok ' -NoNewline -ForegroundColor Green; Write-Host $Text }
}
function Note([string[]]$Lines) {
    if ($Quiet) { return }
    Write-Host '  !  ' -NoNewline -ForegroundColor Yellow
    Write-Host $Lines[0]
    foreach ($l in $Lines[1..($Lines.Length)]) { if ($l) { Write-Host "     $l" } }
}
# Stop: the problem as a plain sentence, then the next step.
function Stop-Install([string]$Problem, [string[]]$Next = @()) {
    Write-Host ''
    Write-Host "  olang was not installed: $Problem" -ForegroundColor Red
    if ($Next.Count) { Write-Host '' }
    foreach ($l in $Next) { Write-Host "    $l" }
    Write-Host ''
    Write-Host "  Help: $HelpUrl" -ForegroundColor DarkGray
    Write-Host ''
    throw 'olang-install-stopped'
}
function Tilde([string]$Path) {
    if ($Path.StartsWith($HOME, [StringComparison]::OrdinalIgnoreCase)) { return '~' + $Path.Substring($HOME.Length) }
    return $Path
}

function Get-Url([string]$Url, [string]$OutFile) {
    # HTTPS only: every URL here starts at https://, and Windows PowerShell
    # 5.1 needs TLS 1.2 switched on by hand.
    if (-not $Url.StartsWith('https://')) { Stop-Install "refusing to download over an insecure connection: $Url" }
    try {
        if ($OutFile) { Invoke-WebRequest -UseBasicParsing -Uri $Url -OutFile $OutFile | Out-Null; return $true }
        return Invoke-RestMethod -UseBasicParsing -Uri $Url
    } catch {
        $status = $null
        if ($_.Exception.Response) { $status = [int]$_.Exception.Response.StatusCode }
        if ($status -eq 404) { return $null }
        if ($status) {
            Stop-Install "a download from GitHub failed (HTTP $status): $Url" @(
                'Wait a minute and run the install command again. If it keeps failing,',
                "please report it at $GitHub/$Repo/issues")
        }
        Stop-Install 'it could not reach GitHub, where olang is downloaded from.' @(
            'Check that this computer is online (try opening https://github.com),',
            'then run the install command again.',
            "($($_.Exception.Message))")
    }
}

# Verifies an openssl ECDSA P-256 signature (DER) over $Data with the PEM
# public key, using only .NET (Framework 4.7 or newer, or .NET Core).
function Test-Signature([byte[]]$Data, [byte[]]$Signature, [string]$Pem) {
    $b64 = ($Pem -replace '-----[A-Z ]+-----', '') -replace '\s', ''
    $spki = [Convert]::FromBase64String($b64)
    # A P-256 SubjectPublicKeyInfo is 91 bytes ending in 04 || X || Y.
    if ($spki.Length -ne 91 -or $spki[26] -ne 4) { throw 'the embedded signing key is not a P-256 public key' }
    $params = New-Object Security.Cryptography.ECParameters
    $params.Curve = [Security.Cryptography.ECCurve+NamedCurves]::nistP256
    $q = New-Object Security.Cryptography.ECPoint
    $q.X = [byte[]]$spki[27..58]
    $q.Y = [byte[]]$spki[59..90]
    $params.Q = $q
    $ecdsa = [Security.Cryptography.ECDsa]::Create($params)
    # DER SEQUENCE { INTEGER r, INTEGER s } to the 64-byte r || s .NET wants
    if ($Signature.Length -lt 8 -or $Signature[0] -ne 0x30) { return $false }
    $i = 2
    $raw = New-Object byte[] 64
    foreach ($half in 0, 1) {
        if ($Signature[$i] -ne 2) { return $false }
        $len = $Signature[$i + 1]
        $int = [byte[]]$Signature[($i + 2)..($i + 1 + $len)]
        $i += 2 + $len
        while ($int.Length -gt 32 -and $int[0] -eq 0) { $int = [byte[]]$int[1..($int.Length - 1)] }
        if ($int.Length -gt 32) { return $false }
        [Array]::Copy($int, 0, $raw, $half * 32 + (32 - $int.Length), $int.Length)
    }
    return $ecdsa.VerifyData($Data, $raw, [Security.Cryptography.HashAlgorithmName]::SHA256)
}

# release-at-least v1.0.0-rc.6 v1.0.0-rc.5 -> true (1.0.0 is newer than 1.0.0-rc.9)
function Test-ReleaseAtLeast([string]$A, [string]$B) {
    $a = $A.TrimStart('v').Split('-', 2); $b = $B.TrimStart('v').Split('-', 2)
    $c = [version]$a[0]; $d = [version]$b[0]
    if ($c -ne $d) { return $c -gt $d }
    if ($a.Count -eq 1) { return $true }
    if ($b.Count -eq 1) { return $false }
    $p = $a[1].Split('.'); $q = $b[1].Split('.')
    if ($p[0] -ne $q[0]) { return $p[0] -gt $q[0] }
    return [int]$p[-1] -ge [int]$q[-1]
}

function Resolve-Version {
    if ($Version) {
        $v = 'v' + $Version.TrimStart('v')
        if ($v -notmatch '^v\d+\.\d+\.\d+') {
            Stop-Install "'$Version' is not a version number." @("Use the form v1.0.0 (see $GitHub/$Repo/releases), or leave it out to get the newest.")
        }
        if (-not (Get-Url "$GitHubApi/repos/$Repo/releases/tags/$v")) {
            Stop-Install "there is no olang release called $v." @("See the list of releases at $GitHub/$Repo/releases.")
        }
        return @($v, '')
    }
    $latest = Get-Url "$GitHubApi/repos/$Repo/releases/latest"
    if ($latest) { return @($latest.tag_name, ' (the newest)') }
    # no full release yet: the newest prerelease
    $list = Get-Url "$GitHubApi/repos/$Repo/releases?per_page=10"
    if (-not $list -or -not $list[0].tag_name) {
        Stop-Install 'olang has no releases to install yet.' @("You can build it from source instead: see $HelpUrl")
    }
    return @($list[0].tag_name, ' (the newest)')
}

function Set-UserPath([string]$Add, [string]$Remove) {
    # Read and write the raw value, so entries like %USERPROFILE%\bin stay
    # unexpanded, and keep its type.
    $key = [Microsoft.Win32.Registry]::CurrentUser.OpenSubKey('Environment', $true)
    $raw = [string]$key.GetValue('Path', '', [Microsoft.Win32.RegistryValueOptions]::DoNotExpandEnvironmentNames)
    $parts = @($raw.Split(';') | Where-Object { $_ -and $_.TrimEnd('\') -ne $Remove.TrimEnd('\') -and $_.TrimEnd('\') -ne $Add.TrimEnd('\') })
    if ($Add) { $parts = @($Add) + $parts }
    $key.SetValue('Path', ($parts -join ';'), [Microsoft.Win32.RegistryValueKind]::ExpandString)
    $key.Close()
    # Tell open programs (Explorer, new terminals) that the environment changed.
    [Environment]::SetEnvironmentVariable('OLANG_INSTALLER_REFRESH', '1', 'User')
    [Environment]::SetEnvironmentVariable('OLANG_INSTALLER_REFRESH', $null, 'User')
}
function Test-OnUserPath([string]$Folder) {
    $raw = [string](Get-Item 'HKCU:\Environment').GetValue('Path', '', 'DoNotExpandEnvironmentNames')
    $expanded = [Environment]::ExpandEnvironmentVariables($raw)
    return @($expanded.Split(';') | Where-Object { $_.TrimEnd('\') -eq $Folder.TrimEnd('\') }).Count -gt 0
}

function Install-Olang {
    $olangHome = if ($env:OLANG_HOME) { $env:OLANG_HOME } else { Join-Path $HOME '.olang' }
    $bin = if ($Dir) { $Dir } else { Join-Path $olangHome 'bin' }

    if ($Uninstall) {
        Say ''; Say '  Removing olang'; Say ''
        $removed = @()
        foreach ($b in 'olang.exe', 'otc.exe') {
            $p = Join-Path $bin $b
            if (Test-Path $p) {
                try { Remove-Item -Force $p } catch {
                    Stop-Install "it could not remove $(Tilde $p), probably because olang is running." @('Close every olang window and terminal running it, then run the uninstall again.')
                }
                $removed += $b
            }
        }
        if ($removed.Count) { Done "Removed $($removed -join ' and ') from $(Tilde $bin)" } else { Done "olang was not installed in $(Tilde $bin)" }
        if (Test-OnUserPath $bin) { Set-UserPath -Add '' -Remove $bin; Done "Removed $(Tilde $bin) from your PATH" }
        if ((Test-Path $bin) -and -not (Get-ChildItem $bin -Force)) { Remove-Item $bin }
        if (Test-Path $olangHome) {
            Say ''
            Say "  Your packages, REPL history and toolchains in $(Tilde $olangHome) were kept."
            Say "  To delete them too:  Remove-Item -Recurse -Force $(Tilde $olangHome)"
        }
        Say ''; Say '  olang is removed. Open a new terminal window to finish.'; Say ''
        return
    }

    Say ''; Say '  Installing olang'; Say ''

    $arch = if ($env:PROCESSOR_ARCHITEW6432) { $env:PROCESSOR_ARCHITEW6432 } else { $env:PROCESSOR_ARCHITECTURE }
    switch ($arch) {
        'AMD64' { $nice = 'Windows, x64' }
        'ARM64' { $nice = 'Windows, ARM64 (olang runs as x64 under emulation)' }
        default {
            Stop-Install "olang's Windows download is for 64-bit Windows, and this is $arch." @(
                'On 32-bit Windows, use olang inside WSL instead: see https://learn.microsoft.com/windows/wsl/install')
        }
    }
    Row 'Computer' $nice

    try { New-Item -ItemType Directory -Force -Path $bin | Out-Null } catch {
        Stop-Install "you do not have permission to write to $bin." @('Choose a folder you own with $env:OLANG_INSTALL_DIR, then run the install command again.')
    }
    $v, $vnote = Resolve-Version
    $plain = $v.TrimStart('v')
    Row 'Version' "$plain$vnote"
    Row 'Folder' (Tilde $bin)
    Say ''

    $olangExe = Join-Path $bin 'olang.exe'
    $previous = ''
    if (Test-Path $olangExe) {
        try { $previous = ((& $olangExe --version 2>$null) -replace '^olang ', '').Trim() } catch { }
    }
    if ($previous -eq $plain -and -not $Force) {
        Done "olang $plain is already installed here"
    } else {
        $tmp = Join-Path ([IO.Path]::GetTempPath()) ('olang-install-' + [Guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Path $tmp | Out-Null
        try {
            $base = "$GitHub/$Repo/releases/download/$v"
            $zip = "olang-$plain-windows-x64.zip"
            $sums = Join-Path $tmp 'SHA256SUMS'
            if (-not (Get-Url "$base/SHA256SUMS" $sums)) {
                Stop-Install "release $v has no SHA256SUMS file, so its download cannot be checked." @(
                    "Try another release from $GitHub/$Repo/releases, and please report this at $GitHub/$Repo/issues")
            }
            if ($SigningKey) {
                $sig = Join-Path $tmp 'SHA256SUMS.sig'
                if (-not (Get-Url "$base/SHA256SUMS.sig" $sig)) {
                    if ($SignedFrom -and -not (Test-ReleaseAtLeast $v $SignedFrom)) {
                        Note "Release $v is older than olang's release signatures (they start at $SignedFrom),", 'so only its checksum was checked.'
                    } elseif ($env:OLANG_ALLOW_UNSIGNED -eq '1') {
                        Note "Release $v has no signature. Going on because OLANG_ALLOW_UNSIGNED=1."
                    } else {
                        Stop-Install "release $v has no signature, so the installer cannot tell that it came from olang." @(
                            "Every release from $SignedFrom on is signed, so this is unexpected.",
                            "Please report it at $GitHub/$Repo/issues and install another release with OLANG_VERSION.")
                    }
                } elseif (Test-Signature ([IO.File]::ReadAllBytes($sums)) ([IO.File]::ReadAllBytes($sig)) $SigningKey) {
                    Done "Signature verified: release $v was signed by olang's release key"
                } else {
                    Stop-Install "the release's signature does not match olang's signing key, so nothing was installed." @(
                        'The files on the download server may have been changed by someone other than olang.',
                        "Please report this at $GitHub/$Repo/issues and do not install this release by hand.")
                }
            }
            $line = Get-Content $sums | Where-Object { $_ -match "^([0-9a-f]{64})  $([regex]::Escape($zip))$" } | Select-Object -First 1
            if (-not $line) {
                Stop-Install "release $v has no ready-made download for Windows." @(
                    'Install a newer release, or use olang inside WSL: run  wsl --install  in PowerShell,',
                    'restart, open the Ubuntu app, and run:  curl -fsSL https://raw.githubusercontent.com/ooyeku/olang/main/install.sh | sh')
            }
            $expected = $line.Substring(0, 64)
            $zipPath = Join-Path $tmp $zip
            Get-Url "$base/$zip" $zipPath | Out-Null
            $actual = (Get-FileHash -Algorithm SHA256 $zipPath).Hash.ToLower()
            if ($actual -ne $expected) {
                Stop-Install 'the download did not match its published checksum, so it was not installed.' @(
                    'This is usually a download cut short. Run the install command again.',
                    "(expected $expected, got $actual)")
            }
            $mb = [math]::Round((Get-Item $zipPath).Length / 1MB)
            Done "Downloaded olang $plain ($mb MB), checksum matches"
            Expand-Archive -Path $zipPath -DestinationPath $tmp -Force
            $src = Join-Path $tmp "olang-$plain-windows-x64"
            foreach ($b in 'olang.exe', 'otc.exe') {
                $dest = Join-Path $bin $b
                # a running olang.exe cannot be overwritten, but it can be renamed
                if (Test-Path $dest) {
                    Remove-Item -Force "$dest.old" -ErrorAction SilentlyContinue
                    try { Move-Item -Force $dest "$dest.old" } catch {
                        Stop-Install "it could not replace $(Tilde $dest)." @('Close every terminal running olang or otc, then run the install command again.')
                    }
                }
                Copy-Item (Join-Path $src $b) $dest
                Remove-Item -Force "$dest.old" -ErrorAction SilentlyContinue
            }
            $check = & $olangExe --version 2>&1
            if ($LASTEXITCODE -ne 0) {
                Stop-Install 'the downloaded olang would not start on this computer.' @(
                    "Please report this at $GitHub/$Repo/issues with this message:", "$check")
            }
            if ($previous -eq $plain) { Done "Reinstalled olang $plain" }
            elseif ($previous) { Done "Updated olang and otc from $previous to $plain" }
            else { Done 'Installed olang and otc' }
        } finally {
            Remove-Item -Recurse -Force $tmp -ErrorAction SilentlyContinue
        }
    }

    $newShell = $false
    if (-not (Test-OnUserPath $bin)) {
        if ($NoModifyPath) {
            Say ''
            Say '  To use olang in new terminals, add this folder to your PATH:'
            Say "    $bin"
        } else {
            Set-UserPath -Add $bin -Remove ''
            Done "Added $(Tilde $bin) to your PATH"
        }
        $newShell = $true
    }
    if (-not (($env:Path -split ';') -contains $bin)) { $env:Path = "$bin;$env:Path"; $newShell = $true }

    Say ''
    Say '  olang is ready. Run:'
    Say ''
    Say '    olang --version'
    Say ''
    if ($newShell) { Say '  (It works in this window now. Other windows that were already open need restarting.)' }
    Say '  Start with the tour at https://olang.dev/book/tour/'
    Say "  To remove olang later: `$env:OLANG_UNINSTALL='1'; irm https://raw.githubusercontent.com/ooyeku/olang/main/install.ps1 | iex"
    Say ''
}

if ($Help) {
    Write-Host @"
The olang installer for Windows

Usage:
  irm https://raw.githubusercontent.com/ooyeku/olang/main/install.ps1 | iex
  .\install.ps1 [-Version v1.0.0] [-Dir FOLDER] [-NoModifyPath] [-Force] [-Quiet] [-Uninstall]

Each option also has an environment variable: OLANG_VERSION,
OLANG_INSTALL_DIR, OLANG_NO_MODIFY_PATH=1, OLANG_FORCE=1, OLANG_QUIET=1,
OLANG_UNINSTALL=1.

The installer writes to the install folder (%USERPROFILE%\.olang\bin) and
your user PATH, downloads only over HTTPS, and checks each download
against the release's signed checksums.

More: $HelpUrl
"@
    return
}

# Under `irm | iex` this runs in the caller's session: put back what it changes.
$previousProtocols = [Net.ServicePointManager]::SecurityProtocol
$previousProgress = $ProgressPreference
$previousErrors = $ErrorActionPreference
try {
    [Net.ServicePointManager]::SecurityProtocol = $previousProtocols -bor [Net.SecurityProtocolType]::Tls12
    $ProgressPreference = 'SilentlyContinue'  # the progress bar makes downloads ten times slower in 5.1
    $ErrorActionPreference = 'Stop'
    Install-Olang
} catch {
    if ($_.Exception.Message -ne 'olang-install-stopped') {
        Write-Host ''
        Write-Host "  olang was not installed: something unexpected went wrong." -ForegroundColor Red
        Write-Host ''
        Write-Host "    $($_.Exception.Message)"
        Write-Host "    Please report this at $GitHub/$Repo/issues with the message above."
        Write-Host ''
    }
    # under `irm | iex` an exit would close the window: just stop
    if ($MyInvocation.MyCommand.Path) { exit 1 }
} finally {
    [Net.ServicePointManager]::SecurityProtocol = $previousProtocols
    $ProgressPreference = $previousProgress
    $ErrorActionPreference = $previousErrors
}

param(
    [Parameter(Position=0)][string]$Version,
    [Parameter(Position=1)][string]$PublicURL,
    [Parameter(Position=2)][string]$Code,
    [switch]$DryRun
)
$ErrorActionPreference = 'Stop'

function Get-WindowsTarget([string]$Architecture) {
    switch ($Architecture.ToLowerInvariant()) {
        'x64' { return 'x86_64-pc-windows-msvc' }
        'arm64' { return 'aarch64-pc-windows-msvc' }
        default { throw "Unsupported Windows architecture: $Architecture" }
    }
}
function Get-MeshTarget {
    return Get-WindowsTarget ([Runtime.InteropServices.RuntimeInformation]::OSArchitecture.ToString())
}
function Assert-HTTPS([string]$Url) {
    $uri = [Uri]$Url
    if (!$uri.IsAbsoluteUri -or $uri.Scheme -ne 'https' -or !$uri.Host -or $uri.UserInfo) {
        throw 'Artifact and registry URLs must use HTTPS without embedded credentials'
    }
    return $uri
}
function Invoke-HTTPS([string]$Url, [string]$Body = '') {
    Add-Type -AssemblyName System.Net.Http
    [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
    $handler = [Net.Http.HttpClientHandler]::new()
    $handler.AllowAutoRedirect = $false
    $client = [Net.Http.HttpClient]::new($handler)
    $client.Timeout = [TimeSpan]::FromMinutes(5)
    try {
        for ($redirect = 0; $redirect -le 5; $redirect++) {
            $uri = Assert-HTTPS $Url
            $method = [Net.Http.HttpMethod]::Get
            if ($Body) { $method = [Net.Http.HttpMethod]::Post }
            $request = [Net.Http.HttpRequestMessage]::new($method, $uri)
            if ($Body) { $request.Content = [Net.Http.StringContent]::new($Body, [Text.Encoding]::UTF8, 'application/json') }
            try {
                $response = $client.SendAsync($request).GetAwaiter().GetResult()
                try {
                    if ([int]$response.StatusCode -in @(301,302,303,307,308)) {
                        if (!$response.Headers.Location) { throw 'Missing HTTPS redirect location' }
                        $Url = [Uri]::new($uri, $response.Headers.Location).AbsoluteUri
                        continue
                    }
                    $null = $response.EnsureSuccessStatusCode()
                    return ,$response.Content.ReadAsByteArrayAsync().GetAwaiter().GetResult()
                } finally { $response.Dispose() }
            } finally { $request.Dispose() }
        }
        throw 'Too many HTTPS redirects'
    } finally { $client.Dispose(); $handler.Dispose() }
}
function Get-MeshResponse([string]$Version, [string]$Target) {
    $request = @{worker='mesh';version="=$Version";target=$Target} | ConvertTo-Json -Compress
    $bytes = Invoke-HTTPS 'https://api.workers.iii.dev/resolve' $request
    return [Text.Encoding]::UTF8.GetString($bytes) | ConvertFrom-Json
}
function Select-MeshArtifact($Response, [string]$Version, [string]$Target) {
    $entries = @($Response.graph | Where-Object { $_.name -eq 'mesh' })
    if ($entries.Count -ne 1 -or $entries[0].type -ne 'binary' -or $entries[0].version -ne $Version) {
        throw 'Registry must resolve the exact mesh binary version once'
    }
    $artifact = $entries[0].binaries.$Target
    if (!$artifact -or $artifact.sha256 -notmatch '^[a-fA-F0-9]{64}$') { throw 'Missing target SHA256' }
    $null = Assert-HTTPS $artifact.url
    return $artifact
}
function Get-MeshArtifact([string]$Url, [string]$Path) {
    [IO.File]::WriteAllBytes($Path, (Invoke-HTTPS $Url))
}
function Invoke-MeshJoin([string]$Path, [string]$PublicURL, [string]$Code) {
    & $Path join $PublicURL $Code
    if ($LASTEXITCODE -ne 0) { throw "mesh join exited with $LASTEXITCODE" }
}
function Install-Mesh([string]$Version, [string]$PublicURL, [string]$Code, [switch]$DryRun) {
    if ($Version -notmatch '^[0-9]+\.[0-9]+\.[0-9]+([+-][A-Za-z0-9.+-]+)?$') { throw 'Invalid mesh version' }
    $hub = [Uri]$PublicURL
    if (!$hub.IsAbsoluteUri -or $hub.Scheme -notin @('http','https') -or !$hub.Host -or $hub.UserInfo) { throw 'Invalid hub URL' }
    if ($Code -notmatch '^[0-9A-Za-z]{4}-?[0-9A-Za-z]{4}$') { throw 'Invalid device code' }
    $target = Get-MeshTarget
    $artifact = Select-MeshArtifact (Get-MeshResponse $Version $target) $Version $target
    if ($DryRun) { Write-Output $artifact.url; return }
    $directory = Join-Path ([IO.Path]::GetTempPath()) ('iii-mesh-install-' + [Guid]::NewGuid().ToString('N'))
    $null = New-Item -ItemType Directory -Path $directory
    try {
        $binary = Join-Path $directory 'mesh.exe'
        Get-MeshArtifact $artifact.url $binary
        $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $binary).Hash
        if ($actual -ne $artifact.sha256) { throw 'mesh SHA256 mismatch; refusing installation' }
        # mesh authenticates this caller's SID across its own UAC prompt.
        Invoke-MeshJoin $binary $PublicURL $Code
        Write-Output 'Installed. Use the mesh.exe in Program Files\iii-mesh for status and leave.'
    } finally { Remove-Item -LiteralPath $directory -Recurse -Force }
}
if ($MyInvocation.InvocationName -ne '.') {
    Install-Mesh -Version $Version -PublicURL $PublicURL -Code $Code -DryRun:$DryRun
}

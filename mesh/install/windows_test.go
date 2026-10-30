package install

import (
	"os/exec"
	"testing"
)

func TestPowerShellInstallerBoundaries(t *testing.T) {
	pwsh, err := exec.LookPath("pwsh")
	if err != nil {
		t.Skip("PowerShell required for portable installer tests")
	}
	cmd := exec.Command(pwsh, "-NoProfile", "-NonInteractive", "-Command", `
$ErrorActionPreference='Stop'
. ./install.ps1
$payload=[Text.Encoding]::UTF8.GetBytes('fixture executable')
$sha=[Security.Cryptography.SHA256]::Create()
$digest=([BitConverter]::ToString($sha.ComputeHash($payload))).Replace('-','').ToLowerInvariant()
$script:record=@()
function Get-MeshTarget { 'x86_64-pc-windows-msvc' }
function Get-MeshResponse($Version,$Target) {
 $script:record += "resolve:$Version`+"`"+`:$Target"
 $versionValue=$Version;if($script:mode -eq 'version'){$versionValue='0.2.0'}
 $url='https://artifacts.example/mesh.exe';if($script:mode -eq 'http'){$url='http://artifacts.example/mesh.exe'}
 $hash=$digest;if($script:mode -eq 'digest'){$hash='0'*64}
 $entry=[pscustomobject]@{name='mesh';version=$versionValue;type='binary';binaries=@{'x86_64-pc-windows-msvc'=[pscustomobject]@{url=$url;sha256=$hash}}}
 $graph=@($entry);if($script:mode -eq 'duplicate'){$graph+=@($entry)}
 return [pscustomobject]@{graph=$graph}
}
function Get-MeshArtifact($Url,$Path) { $script:record+='download';[IO.File]::WriteAllBytes($Path,$payload) }
function Invoke-MeshJoin($Path,$PublicURL,$Code){$script:record+='join';if($PublicURL -ne 'https://hub.example/path with space' -or $Code -ne 'ABCD-EFGH'){throw 'argument loss'}}
foreach($mode in @('good','digest','http','version','duplicate')){
 $script:mode=$mode;$script:record=@();$failed=$false
 try{Install-Mesh -Version '0.1.0' -PublicURL 'https://hub.example/path with space' -Code 'ABCD-EFGH'}catch{$failed=$true}
 if($mode -eq 'good') {if($failed -or $script:record[-1] -ne 'join'){throw "valid install failed: $script:record"}}
 else {if(!$failed -or $script:record -contains 'join'){throw "unsafe install accepted: $mode"}}
}
if((Get-WindowsTarget 'Arm64') -ne 'aarch64-pc-windows-msvc'){throw 'ARM64 target'}
if((Get-WindowsTarget 'X64') -ne 'x86_64-pc-windows-msvc'){throw 'AMD64 target'}
$failed=$false;try{Get-WindowsTarget 'X86'}catch{$failed=$true};if(!$failed){throw 'unsupported architecture'}
Write-Output 'PowerShell installer boundaries passed'
`)
	if out, err := cmd.CombinedOutput(); err != nil {
		t.Fatalf("%v\n%s", err, out)
	}
}

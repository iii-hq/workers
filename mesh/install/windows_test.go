package install

import (
	"crypto/rand"
	"encoding/base64"
	"encoding/json"
	"github.com/iii-hq/workers/mesh/internal/cli"
	"github.com/iii-hq/workers/mesh/internal/hub"
	"os"
	"os/exec"
	"path/filepath"
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

func TestReturnedPowerShellTokenLineRunsCLI(t *testing.T) {
	pwsh, err := exec.LookPath("pwsh")
	if err != nil {
		t.Skip("PowerShell required for portable installer tests")
	}
	for _, mode := range []string{"valid", "digest", "ambiguous", "malformed"} {
		t.Run(mode, func(t *testing.T) {
			fixture, dir, raw := executableInstallerFixture(t, mode)
			_, line := hub.TokenInstallLines("0.1.0", raw)
			if mode == "ambiguous" {
				line += " -PublicURL https://hub.example"
			}
			if mode == "malformed" {
				_, line = hub.TokenInstallLines("0.1.0", "iiimj_"+base64.RawURLEncoding.EncodeToString([]byte(rand.Text())))
			}
			path, err := filepath.Abs("install.ps1")
			if err != nil {
				t.Fatal(err)
			}
			cmd := exec.Command(pwsh, "-NoProfile", "-NonInteractive", "-Command", `
$ErrorActionPreference='Stop'
function Invoke-RestMethod {
 $tokens=$null;$errors=$null
 $ast=[System.Management.Automation.Language.Parser]::ParseFile($env:MESH_INSTALLER_SCRIPT,[ref]$tokens,[ref]$errors)
 return $ast.ParamBlock.Extent.Text + @'

$enrollArguments=$PSBoundParameters
. $env:MESH_INSTALLER_SCRIPT
function Get-MeshTarget { 'x86_64-pc-windows-msvc' }
function Get-MeshResponse($Version,$Target) {
 [IO.File]::WriteAllText((Join-Path $env:FIXTURE 'resolved'),'yes')
 $response=Get-Content (Join-Path $env:FIXTURE 'response') -Raw | ConvertFrom-Json
 $artifact=$response.graph[0].binaries.'x86_64-unknown-linux-gnu'
 return [pscustomobject]@{graph=@([pscustomobject]@{name='mesh';version=$Version;type='binary';binaries=@{$Target=$artifact}})}
}
function Get-MeshArtifact($Url,$Path) { Copy-Item (Join-Path $env:FIXTURE 'artifact') $Path; & /bin/chmod 700 $Path }
Install-Mesh @enrollArguments
'@
}
Invoke-Expression $env:MESH_INSTALLER_LINE
`)
			cmd.Env = append(fixture.Env, "MESH_INSTALLER_SCRIPT="+path, "MESH_INSTALLER_LINE="+line)
			out, err := cmd.CombinedOutput()
			b, readErr := os.ReadFile(filepath.Join(dir, "parsed"))
			if mode != "valid" {
				if err == nil || !os.IsNotExist(readErr) {
					t.Fatal("invalid installer reached CLI")
				}
				if mode != "digest" {
					if _, err := os.Stat(filepath.Join(dir, "resolved")); !os.IsNotExist(err) {
						t.Fatal("malformed options reached registry")
					}
				}
				return
			}
			if err != nil || readErr != nil {
				t.Fatalf("installer did not execute Go CLI: %v %v %s", err, readErr, out)
			}
			var parsed cli.JoinArgs
			if err = json.Unmarshal(b, &parsed); err != nil {
				t.Fatal(err)
			}
			if parsed.Token != raw || parsed.PublicURL != "https://hub.example/prefix" || parsed.Port != 41643 {
				t.Fatal("PowerShell lost token arguments")
			}
		})
	}
}

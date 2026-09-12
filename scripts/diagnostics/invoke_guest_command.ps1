#Requires -Version 7.0
<#
.SYNOPSIS
Runs a bounded guest diagnostic and releases its VirtualBox session on exit.
.DESCRIPTION
Use this instead of detached guestcontrol start calls for WAN diagnostics.
The attached run waits for the guest process, then closes its guest session.
Credentials are supplied by the caller, never stored in this script.
#>
[CmdletBinding()]
param(
    [string]$VmName = 'clawdbot',
    [Parameter(Mandatory)][pscredential]$Credential,
    [Parameter(Mandatory)][string]$Executable,
    [string[]]$GuestArguments = @(),
    [ValidateRange(1000, 300000)][int]$TimeoutMilliseconds = 75000,
    [string]$VBoxManage = 'C:\Program Files\Oracle\VirtualBox\VBoxManage.exe'
)
$ErrorActionPreference = 'Stop'
if (!(Test-Path -LiteralPath $VBoxManage -PathType Leaf)) {
    throw 'VBoxManage executable not found.'
}
# VBoxManage accepts the password as a native argument; do not print the
# expanded command or persist the credential in a command/transcript file.
$password = $Credential.GetNetworkCredential().Password
& $VBoxManage guestcontrol $VmName run --exe $Executable `
    --username $Credential.UserName --password $password `
    --timeout $TimeoutMilliseconds --wait-stdout --wait-stderr -- @GuestArguments
$code = $LASTEXITCODE
if ($code -ne 0) {
    throw "Attached guest diagnostic failed (VBoxManage exit $code)."
}

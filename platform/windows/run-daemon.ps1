param([Parameter(Mandatory=$true)][string]$Daemon, [Parameter(Mandatory=$true)][string]$DataDir)
$ErrorActionPreference = 'Stop'
# Keep the login task alive for the service lifetime, without opening a console.
# Propagate failure so Task Scheduler can apply its configured restart policy.
$process = Start-Process -FilePath $Daemon -ArgumentList ('--data-dir "' + $DataDir + '" serve') -WindowStyle Hidden -Wait -PassThru -RedirectStandardError (Join-Path $DataDir 'daemon-login.log') -RedirectStandardOutput (Join-Path $DataDir 'daemon-login-stdout.log')
exit $process.ExitCode

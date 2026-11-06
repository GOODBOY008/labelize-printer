param([string]$ConfigPath, [switch]$Remove)
$ErrorActionPreference = 'Stop'
$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$admin = (New-Object Security.Principal.WindowsPrincipal($identity)).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
if (!$admin) {
    $arguments = '-NoProfile -ExecutionPolicy Bypass -File "' + $PSCommandPath + '"'
    if ($Remove) { $arguments += ' -Remove' } else { $arguments += ' -ConfigPath "' + $ConfigPath + '"' }
    $p = Start-Process powershell.exe -Verb RunAs -ArgumentList $arguments -Wait -PassThru
    exit $p.ExitCode
}
$queue = 'LabelizePrinter'
$port = 'LabelizePrinter_RAW'
$existing = Get-Printer -Name $queue -ErrorAction SilentlyContinue
$existingPort = Get-PrinterPort -Name $port -ErrorAction SilentlyContinue
if ($existing -and ($existing.PortName -ne $port -or $existing.DriverName -notmatch 'ZDesigner')) {
    throw 'A different printer already owns the LabelizePrinter queue name.'
}
if ($existingPort -and $existingPort.PrinterHostAddress -ne '127.0.0.1') {
    throw 'A different printer already owns the LabelizePrinter_RAW port name.'
}
if ($Remove) {
    if (Get-Printer -Name $queue -ErrorAction SilentlyContinue) { Remove-Printer -Name $queue }
    if (Get-PrinterPort -Name $port -ErrorAction SilentlyContinue) { Remove-PrinterPort -Name $port }
    exit 0
}
$config = Get-Content -LiteralPath $ConfigPath -Raw | ConvertFrom-Json
if ($config.raw_port -lt 1024 -or $config.raw_port -gt 65535) { throw 'Invalid RAW port' }
$protected = Get-ItemPropertyValue -Path 'HKLM:\SOFTWARE\Policies\Microsoft\Windows NT\Printers\WPP' -Name 'WindowsProtectedPrintGroupPolicyState' -ErrorAction SilentlyContinue
if ($protected -eq 1) { throw 'Windows protected print mode blocks ZDesigner. This app does not change that policy.' }
$driver = Get-PrinterDriver -Name $config.windows_driver -ErrorAction Stop
if ($driver.Name -notmatch 'ZDesigner' -or $driver.Name -notmatch 'ZPL') { throw 'Select a Zebra ZDesigner ZPL driver' }
if ($existingPort) {
    $tcpPort = Get-CimInstance Win32_TCPIPPrinterPort -Filter "Name='LabelizePrinter_RAW'"
    Set-CimInstance -InputObject $tcpPort -Property @{ PortNumber = [uint32]$config.raw_port; SNMPEnabled = $false }
} else { Add-PrinterPort -Name $port -PrinterHostAddress '127.0.0.1' -PortNumber $config.raw_port -SNMP 0 }
if ($existing) { Set-Printer -Name $queue -DriverName $driver.Name -PortName $port -Shared:$false }
else { Add-Printer -Name $queue -DriverName $driver.Name -PortName $port -Shared:$false }
$printer = Get-CimInstance Win32_Printer -Filter "Name='LabelizePrinter'"
if ($printer) { Set-CimInstance -InputObject $printer -Property @{ EnableBIDI = $false } }
Write-Output 'Queue installed. In ZDesigner Driver Settings, set Status checking > Status check type to Disabled.'

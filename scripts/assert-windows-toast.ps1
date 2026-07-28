param(
    [Parameter(Mandatory = $true)][string]$TaskName,
    [Parameter(Mandatory = $true)][string]$Status,
    [int]$TimeoutSeconds = 20
)

$ErrorActionPreference = "Stop"
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes

$allowedShellProcesses = @(
    "ShellExperienceHost",
    "StartMenuExperienceHost",
    "explorer"
)
$deadline = [DateTimeOffset]::UtcNow.AddSeconds($TimeoutSeconds)
$root = [Windows.Automation.AutomationElement]::RootElement
$trueCondition = [Windows.Automation.Condition]::TrueCondition

while ([DateTimeOffset]::UtcNow -lt $deadline) {
    $elements = $root.FindAll(
        [Windows.Automation.TreeScope]::Descendants,
        $trueCondition
    )
    for ($index = 0; $index -lt $elements.Count; $index++) {
        $element = $elements.Item($index)
        $name = $element.Current.Name
        if ([string]::IsNullOrWhiteSpace($name) -or -not $name.Contains($TaskName)) {
            continue
        }
        try {
            $process = Get-Process -Id $element.Current.ProcessId -ErrorAction Stop
        } catch {
            continue
        }
        if ($allowedShellProcesses -notcontains $process.ProcessName) {
            continue
        }

        $candidate = $element
        for ($depth = 0; $depth -lt 8 -and $null -ne $candidate; $depth++) {
            $subtree = $candidate.FindAll(
                [Windows.Automation.TreeScope]::Subtree,
                $trueCondition
            )
            $text = for ($child = 0; $child -lt $subtree.Count; $child++) {
                $childName = $subtree.Item($child).Current.Name
                if (-not [string]::IsNullOrWhiteSpace($childName)) { $childName }
            }
            if (($text -join "`n").Contains($Status)) {
                Write-Output "Windows Shell notification observed for the requested task and terminal status."
                exit 0
            }
            $candidate = [Windows.Automation.TreeWalker]::ControlViewWalker.GetParent($candidate)
        }
    }
    Start-Sleep -Milliseconds 250
}

throw "No Windows Shell notification exposed task name '$TaskName' and status '$Status' within $TimeoutSeconds seconds."

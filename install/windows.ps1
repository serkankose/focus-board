# Installs focus-board on Windows for the current user (owner, 2026-11-26, B45 "e"): the binary in
# cargo's bin folder, and the server as a scheduled task that starts at logon and restarts after a
# crash, logging to %LOCALAPPDATA%\focus-board\serve.log. Then the skill for each agent tool, Claude
# Code's hook entry (focus-board install-hook) and opencode's plugin.
# UNVERIFIED: written on Linux; never run on Windows.
#
# Copilot on Windows runs PowerShell for shell commands, and the board's identity rewrite puts a
# POSIX `export` in front, so Copilot's hook is not installed here. Claude Code's Bash tool on
# Windows runs Git Bash, where the rewrite works (DESIGN.md, "Platforms", P5).
#
# Usage: powershell -ExecutionPolicy Bypass -File install\windows.ps1 [-Remove]
param([switch]$Remove)
$ErrorActionPreference = "Stop"
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$task = "focus-board"

if ($Remove) {
    Unregister-ScheduledTask -TaskName $task -Confirm:$false -ErrorAction SilentlyContinue
    Write-Output "removed the task; the board itself is kept"
    exit 0
}

cargo install --path (Join-Path $here "..") --locked -q
$bin = Join-Path $env:USERPROFILE ".cargo\bin\focus-board.exe"
$logs = Join-Path $env:LOCALAPPDATA "focus-board"
New-Item -ItemType Directory -Force -Path $logs | Out-Null

# The server at logon, hidden; cmd appends its output to the log.
$action = New-ScheduledTaskAction -Execute "cmd.exe" `
    -Argument "/c `"`"$bin`" serve >> `"$logs\serve.log`" 2>&1`""
$trigger = New-ScheduledTaskTrigger -AtLogOn -User $env:USERNAME
$settings = New-ScheduledTaskSettingsSet -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries `
    -ExecutionTimeLimit ([TimeSpan]::Zero) -RestartCount 3 -RestartInterval (New-TimeSpan -Minutes 1)
Register-ScheduledTask -TaskName $task -Action $action -Trigger $trigger -Settings $settings -Force | Out-Null
Stop-ScheduledTask -TaskName $task -ErrorAction SilentlyContinue
Start-ScheduledTask -TaskName $task

# The skill, from the one template, for each agent tool on this machine.
function Write-Skill([string]$dir, [string]$tool) {
    New-Item -ItemType Directory -Force -Path $dir | Out-Null
    $text = (& $bin skill --tool $tool) -join "`n"
    [IO.File]::WriteAllText((Join-Path $dir "SKILL.md"), $text + "`n")
    Write-Output "skill: $dir\SKILL.md"
}
$claude = Join-Path $env:USERPROFILE ".claude"
if (Test-Path $claude) {
    Write-Skill (Join-Path $claude "skills\focus-board") "claude"
    & $bin install-hook
}
$copilot = Join-Path $env:USERPROFILE ".copilot"
if (Test-Path $copilot) {
    Write-Skill (Join-Path $copilot "skills\focus-board") "copilot"
    Write-Output "Copilot's hook is not installed on Windows: its shell is PowerShell (see the top)."
}
$opencode = Join-Path $env:USERPROFILE ".config\opencode"
if (Test-Path $opencode) {
    Write-Skill (Join-Path $opencode "skills\focus-board") "opencode"
    New-Item -ItemType Directory -Force -Path (Join-Path $opencode "plugins") | Out-Null
    Copy-Item (Join-Path $here "opencode\focus-board.js") (Join-Path $opencode "plugins\focus-board.js") -Force
    Write-Output "hook:  $opencode\plugins\focus-board.js"
}
Write-Output "board: $(& $bin where)"
Write-Output "page:  $(& $bin where --page)"

# LEGACY / DEPRECATED - not part of the active lifecycle.
#
# The PGC Host now launches, supervises, and stops FFmpeg natively
# (windows/service/src/ffmpeg.rs). MediaMTX no longer runs this script.
# Kept only as a reference and rollback path until native ownership has been
# validated hands-on. To roll back, restore the previous `gameplay` path
# section in mediamtx.yml and run a Host build from before native ownership.

$pidFile = "C:\MediaMTX\gameplay-ffmpeg.pid"
$lifecycleLog = "C:\MediaMTX\gameplay-lifecycle.log"

# How long to wait for FFmpeg to actually exit after requesting termination.
$exitTimeoutMs = 5000

# Diagnostics only: same timestamp format as the PGC Host log.
function Write-PgcLog($message) {
    $stamp = [DateTime]::UtcNow.ToString("HH:mm:ss.fff")
    $line = "[${stamp}Z][PGC][SCRIPT] stop-gameplay (PowerShell PID $PID): $message"
    Write-Host $line
    Add-Content -Path $lifecycleLog -Value $line -ErrorAction SilentlyContinue
}

function Remove-PidFile {
    Remove-Item $pidFile -Force -ErrorAction SilentlyContinue
    Write-PgcLog "PID file removed"
}

$runningFfmpeg = @(Get-Process -Name ffmpeg -ErrorAction SilentlyContinue | ForEach-Object { $_.Id })
Write-PgcLog "started; ffmpeg running: [$($runningFfmpeg -join ', ')]"

if (Test-Path $pidFile) {
    $ffmpegPid = Get-Content $pidFile | Select-Object -First 1

    if ($ffmpegPid) {
        $process = Get-Process -Id $ffmpegPid -ErrorAction SilentlyContinue

        if ($process -and $process.ProcessName -eq "ffmpeg") {
            Write-PgcLog "PID file reports FFmpeg PID $ffmpegPid; requesting termination"

            # Terminate and wait on the same process object, so a PID that is
            # reused after FFmpeg exits can never be killed or waited on.
            Stop-Process -InputObject $process -Force -ErrorAction SilentlyContinue

            $exited = $false
            try {
                $exited = $process.WaitForExit($exitTimeoutMs)
            } catch {
                Write-PgcLog "could not wait for FFmpeg PID $ffmpegPid : $($_.Exception.Message)"
            }

            if ($exited) {
                Write-PgcLog "FFmpeg PID $ffmpegPid confirmed exited"
                Remove-PidFile
                Write-PgcLog "cleanup complete"
            } else {
                # Keep the PID file: it is the only record that this FFmpeg
                # still exists, and the Host uses it for its own cleanup.
                Write-PgcLog "WARNING: FFmpeg PID $ffmpegPid still running after $exitTimeoutMs ms; PID file kept; cleanup NOT complete"
            }
        } elseif ($process) {
            Write-PgcLog "PID file reports PID $ffmpegPid, which is $($process.ProcessName), not ffmpeg; not stopping it"
            Remove-PidFile
        } else {
            Write-PgcLog "PID file reports PID $ffmpegPid, which is not running"
            Remove-PidFile
        }
    } else {
        Write-PgcLog "PID file is empty"
        Remove-PidFile
    }
} else {
    Write-PgcLog "no PID file; nothing to stop"
}

$remainingFfmpeg = @(Get-Process -Name ffmpeg -ErrorAction SilentlyContinue | ForEach-Object { $_.Id })
Write-PgcLog "finished; ffmpeg still running: [$($remainingFfmpeg -join ', ')]"

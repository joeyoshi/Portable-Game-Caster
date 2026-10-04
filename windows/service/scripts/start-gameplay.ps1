# LEGACY / DEPRECATED - not part of the active lifecycle.
#
# The PGC Host now launches, supervises, and stops FFmpeg natively
# (windows/service/src/ffmpeg.rs). MediaMTX no longer runs this script.
# Kept only as a reference and rollback path until native ownership has been
# validated hands-on. To roll back, restore the previous `gameplay` path
# section in mediamtx.yml and run a Host build from before native ownership.

$ffmpeg = "C:\ffmpeg\bin\ffmpeg.exe"
$pidFile = "C:\MediaMTX\gameplay-ffmpeg.pid"
$lifecycleLog = "C:\MediaMTX\gameplay-lifecycle.log"

# Diagnostics only: same timestamp format as the PGC Host log.
function Write-PgcLog($message) {
    $stamp = [DateTime]::UtcNow.ToString("HH:mm:ss.fff")
    $line = "[${stamp}Z][PGC][SCRIPT] start-gameplay (PowerShell PID $PID): $message"
    Write-Host $line
    Add-Content -Path $lifecycleLog -Value $line -ErrorAction SilentlyContinue
}

$existingFfmpeg = @(Get-Process -Name ffmpeg -ErrorAction SilentlyContinue | ForEach-Object { $_.Id })
$previousPid = "none"
if (Test-Path $pidFile) {
    $previousPid = Get-Content $pidFile -ErrorAction SilentlyContinue | Select-Object -First 1
}
Write-PgcLog "started; ffmpeg already running: [$($existingFfmpeg -join ', ')]; PID file before launch: $previousPid"

$args = @(
    "-stats",
    "-f", "dshow",
    "-rtbufsize", "64M",
    "-video_size", "1920x1080",
    "-framerate", "60",
    "-audio_buffer_size", "50",
    "-i", '"video=Game Capture HD60 S+:audio=Digital Audio Interface (Game Capture HD60 S+)"',
    "-vf", "format=nv12",
    "-c:v", "h264_nvenc",
    "-preset", "p2",
    "-tune", "ull",
    "-zerolatency", "1",
    "-rc", "cbr",
    "-b:v", "20M",
    "-maxrate", "20M",
    "-bufsize", "1M",
    "-g", "30",
    "-bf", "0",
    "-bsf:v", "dump_extra=freq=keyframe",
    "-r", "60",
    "-fps_mode", "cfr",
    "-af", "aresample=48000:async=1000:first_pts=0",
    "-c:a", "aac",
    "-b:a", "192k",
    "-ac", "2",
    "-flush_packets", "1",
    "-f", "mpegts",
    "srt://127.0.0.1:8890?streamid=publish:gameplay&pkt_size=1316&latency=20000&tlpktdrop=1"
)

$process = Start-Process `
    -FilePath $ffmpeg `
    -ArgumentList $args `
    -PassThru `
    -NoNewWindow

# Keep the process handle so ExitCode is available after exit.
$null = $process.Handle

$process.Id | Set-Content $pidFile

Write-PgcLog "launched FFmpeg PID $($process.Id); PID file written"

$process.WaitForExit()

Write-PgcLog "FFmpeg PID $($process.Id) exited with code $($process.ExitCode); script exiting"

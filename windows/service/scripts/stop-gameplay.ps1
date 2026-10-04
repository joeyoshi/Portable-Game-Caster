$pidFile = "C:\MediaMTX\gameplay-ffmpeg.pid"

if (Test-Path $pidFile) {
    $ffmpegPid = Get-Content $pidFile | Select-Object -First 1

    if ($ffmpegPid) {
        $process = Get-Process -Id $ffmpegPid -ErrorAction SilentlyContinue

        if ($process -and $process.ProcessName -eq "ffmpeg") {
            Stop-Process -Id $ffmpegPid -Force
        }
    }

    Remove-Item $pidFile -Force -ErrorAction SilentlyContinue
}
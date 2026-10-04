$ffmpeg = "C:\ffmpeg\bin\ffmpeg.exe"
$pidFile = "C:\MediaMTX\gameplay-ffmpeg.pid"

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

$process.Id | Set-Content $pidFile

$process.WaitForExit()
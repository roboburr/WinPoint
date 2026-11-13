Native image routes preserve the original `/icon`, `/thumb`, `/file` MIME checks,
encoded output, FDO reuse, cache keys and conditional ETags.

Video posters are an explicit extension: `/thumb?path=<local path>&size=256&video=1`.
Without `video=1`, video requests retain the original behavior (only an existing
freedesktop thumbnail can be used). The visible Explorer Details pane opts in;
opening folders or indexing files does not start a video decoder.

Existing fresh FDO thumbnails are preferred. Otherwise `win11-image-worker --video
<path> <size>` uses optional system `ffprobe`/`ffmpeg`, selects one frame around one
second (first-frame fallback for short clips), preserves aspect ratio and produces
PNG. Missing/unsupported/corrupt media returns 404 so the original file icon stays.
No player, audio, autoplay, or continuously running video process is created.

Decoding shares the existing two-worker gate. Bubblewrap uses a read-only root,
private temporary directory and separate network/IPC/PID namespaces when available.
The child tool protocol whitelist is `file,pipe`; demuxers are restricted to known
movie containers. Each ffmpeg/ffprobe child has 384 MiB virtual address space,
8 CPU seconds, an 8 MiB output/file-size cap, no core dump and one decoder thread.
A probe has a 3 second wall deadline and each frame attempt 5 seconds; the outer
worker has the existing 20 second deadline. Pixel dimensions are at most 1024.

Posters use `<cache>/thumbs-video-v1/<prefix>/<sha1>.png`, separate from image
thumbnails. Keys include canonical path, nanosecond mtime, source size, size bucket
and format version. Writes are atomic mode 0600. The cache target is 128 MiB with
60 day age eviction, checked after startup and every 32 writes. Concurrent requests
for the same poster share one generation. Image cache behavior remains unchanged.

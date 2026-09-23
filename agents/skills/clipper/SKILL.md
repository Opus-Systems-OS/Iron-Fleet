---
name: clipper
description: Cut a long-form YouTube video into vertical Shorts and queue them on the ClipperZero channel through Buffer — pick moments from the transcript, cut and encode on the rig's GPU, publish the file to the public clips bucket, then create a queued Buffer post with the YouTube metadata it requires. Use when asked to clip a video, make Shorts from something, fill the ClipperZero queue, or turn a podcast or long upload into short-form.
---

# Clipping for ClipperZero

ClipperZero is a YouTube clipping channel. The job: take a long-form video the
human pointed you at, find the moments worth watching on their own, cut them
vertical, and leave them in the Buffer queue with a title and description. A
human approves everything in Buffer before it reaches YouTube.

## Non-negotiables

- **Queue, never publish.** `mode: addToQueue`, `schedulingType: automatic`.
  `shareNow` and `shareNext` are off limits — they skip the human.
- **Only the sources you were given.** The task names the video or channel.
  Clipping anything else is out of scope, whatever the reason seems.
- **The clip URL has to outlive the post.** Buffer fetches the video *when the
  post publishes*, not when you create it. A presigned or expiring URL passes
  `createPost` and then fails silently days later. Public bucket, stable key,
  and never delete a clip that a queued post still references.
- **Dry-run first, every time.** `--dry-run` validates the payload locally,
  makes no API call, and doesn't touch the rate limit.
- **Disclose honestly.** Set `aiAssisted: true` — you wrote the copy. Leave
  `metadata.youtube.isAiGenerated` **false**: the footage is real, and claiming
  otherwise is its own kind of wrong. If you ever generate the video itself,
  flip it.

## Environment

Preset in the worker container; read them, don't invent them:

| Variable | What |
|---|---|
| `BUFFER_API_KEY` | read by the `buffer` CLI directly |
| `BUFFER_CHANNEL_ID` | the ClipperZero YouTube channel |
| `BUFFER_ORGANIZATION_ID` | the Buffer org, for `posts list` |
| `CLIPS_BUCKET` | the R2 bucket clips are written to |
| `CLIPS_PUBLIC_BASE_URL` | public base URL that serves that bucket |
| `RCLONE_CONFIG_CLIPS_*` | the `clips:` rclone remote (Cloudflare R2) |

`buffer doctor` tells you whether auth is healthy before you spend a session
finding out the hard way.

## Pipeline

### 1. Transcript before video

Subtitles are tiny and carry timestamps; the video is a gigabyte. Read first,
download second.

```bash
yt-dlp --write-auto-subs --sub-langs en --sub-format vtt --skip-download \
       -o '/workspace/src/%(id)s.%(ext)s' "$VIDEO_URL"
```

Read the `.vtt` and pick moments that stand on their own: a complete thought, a
strong first line, no missing setup. A clip that needs the previous ten minutes
to make sense is not a clip.

### 2. Cut only what you need

```bash
yt-dlp --download-sections "*00:12:30-00:13:18" --force-keyframes-at-cuts \
       -f 'bv*[height>=1080]+ba/b' --merge-output-format mp4 \
       -o '/workspace/src/%(id)s-clip.%(ext)s' "$VIDEO_URL"
```

`--force-keyframes-at-cuts` re-encodes the boundaries so the clip starts on the
frame you asked for instead of the previous keyframe.

### 3. Make it vertical

1080×1920, 9:16. Centre-crop when the subject is already centred:

```bash
ffmpeg -y -i in.mp4 \
  -vf "scale=-2:1920,crop=1080:1920" \
  -c:v h264_nvenc -preset p5 -cq 23 -b:v 0 -c:a aac -b:a 128k \
  -movflags +faststart out.mp4
```

Blurred backdrop when cropping would cut someone out of frame:

```bash
ffmpeg -y -i in.mp4 -filter_complex \
  "[0:v]split=2[bg][fg];[bg]scale=1080:1920:force_original_aspect_ratio=increase,crop=1080:1920,boxblur=40:8[bg2];[fg]scale=1080:-2[fg2];[bg2][fg2]overlay=(W-w)/2:(H-h)/2" \
  -c:v h264_nvenc -preset p5 -cq 23 -b:v 0 -c:a aac -b:a 128k \
  -movflags +faststart out.mp4
```

If `h264_nvenc` fails to load, fall back to `-c:v libx264 -crf 20 -preset
medium` and say so in the report — it means the container lost NVENC, which is
worth knowing about.

Burned-in captions are optional but usually worth it: slice the `.vtt` window
into an `.srt` with times rebased to the clip, then add a `subtitles=` filter
with `Alignment=2` and a `MarginV` high enough to clear the UI.

**Length:** YouTube treats a vertical video up to 3 minutes as a Short. Aim for
20–60 s unless the task says otherwise; anything longer needs a reason.

### 4. Publish the file

```bash
key="$(date -u +%Y-%m-%d)/${slug}-$(openssl rand -hex 4).mp4"
rclone copyto out.mp4 "clips:${CLIPS_BUCKET}/${key}" --s3-no-head \
  --header-upload "Content-Type: video/mp4"
clipUrl="${CLIPS_PUBLIC_BASE_URL}/${key}"
curl -sI "$clipUrl" | head -3     # 200 and video/mp4, or stop here
```

`--s3-no-head` is required against R2 — without it rclone logs a false
`NotImplemented` and retries. The random suffix means a re-run never overwrites
a file some queued post is still pointing at.

### 5. Queue the post

YouTube's minimum payload is a video asset plus `metadata.youtube.title` and
`metadata.youtube.categoryId`. Nested fields can only be set through `--json`.

```bash
payload=$(jq -nc \
  --arg ch  "$BUFFER_CHANNEL_ID" \
  --arg url "$clipUrl" \
  --arg title "$title" \
  --arg text  "$description" \
  '{channelId: $ch,
    schedulingType: "automatic",
    mode: "addToQueue",
    text: $text,
    aiAssisted: true,
    assets: [{video: {url: $url, metadata: {title: $title}}}],
    metadata: {youtube: {title: $title,
                         categoryId: "24",
                         privacy: "public",
                         madeForKids: false,
                         isAiGenerated: false}}}')

buffer posts create --json "$payload" --dry-run --output json
buffer posts create --json "$payload" --output json --fields post.id,post.status
```

Categories: 24 Entertainment, 23 Comedy, 20 Gaming, 17 Sports, 22 People &
Blogs, 25 News & Politics, 27 Education, 28 Science & Technology. Pick by
content.

**Title** ≤ 100 characters, the hook of the clip, and no promise the clip
doesn't pay off. **Description** carries the context plus credit to the source
channel and a link to the original video — a clipping account that doesn't
credit is a clipping account that gets struck.

## When it fails

| Exit | Meaning | Do |
|---|---|---|
| 2 | usage / validation | fix the payload; never retry as-is |
| 3 | API error | see below |
| 4 | auth | run `buffer doctor`, then stop and report — don't loop |

`posts create` has no idempotency key. On exit 3 the post may well have landed:

```bash
buffer posts list --organization-id "$BUFFER_ORGANIZATION_ID" --output json \
  | jq --arg ch "$BUFFER_CHANNEL_ID" --arg t "$description" \
      '.items[] | select(.channel.id == $ch and .text == $t) | .id'
```

Something comes back? It landed — don't retry. Nothing? Wait 5 s, try once more.

On 429 the message names the window (15m / 24h / 30d) and a `Retry-After`. The
CLI does not auto-retry: sleep that long, retry once, then double and retry
once more, then stop and report. Rate limits are per API key, so parallel
`buffer` calls only make it worse — stay serial.

Drafts (`--save-to-draft`) skip posting limits and never publish. Use one to
prove the pipeline end to end without touching the queue.

## Don't

- Don't `buffer posts delete` anything you didn't create this session.
- Don't tidy up the clips bucket. Old objects belong to queued or published
  posts; deleting one breaks a post that hasn't gone out yet.
- Don't upgrade yt-dlp, ffmpeg or the CLI from inside a session. If yt-dlp is
  broken on a video, say so — the image gets rebuilt on the rig.
- Don't invent the channel id. It's in the environment.

# Store screenshots

The iii app's Worker Store shows a worker's screenshots, one device at a time,
the way an app store does. The Registry stores them **per version**: each
published version has its own list, fixed once it is published. A worker
keeps its screenshots in this repository, next to its source, and every
release Release Control publishes takes them to the Registry with the
version's archives.

## The folder

```text
<worker>/
├── iii.worker.yaml
├── README.md
└── screenshots/
    ├── screenshots.yaml       # the list, in display order
    ├── android-board.webp
    ├── iphone-board.png
    └── mac/board.jpg          # sub-folders are fine
```

`screenshots.yaml` is exactly the list the Registry CLI reads with
`registry publish --screenshots` (iii-hq/registry `docs/SCREENSHOTS.md`):

```yaml
# The order is the order the Worker Store shows them in.
- file: android-board.webp
  platform: android            # iphone | ipad | mac | android | web
  caption: The board, one lane at a time
- file: iphone-board.png
  platform: iphone
- file: mac/board.jpg
  platform: mac
  caption: The board on a Mac
```

Each entry has `file` (a path inside `screenshots/`: no absolute path, no
`..`), `platform` and an optional `caption`. Nothing else is accepted.

The list is its own file, not a key of `iii.worker.yaml` (the iii engine reads
that file too and refuses keys it does not know) nor of `.deploy/workers.yaml`
(whose entries hold exactly `source`, `artifact` and `publish`).

## Keep, replace, clear

| `<worker>/screenshots/screenshots.yaml` | The next published version gets |
|---|---|
| absent | the screenshots of the version before it (Registry inheritance) |
| `[]` | none |
| entries | those images, in that order |

Removing screenshots is therefore explicit: an empty list. Deleting the file
never removes them; it lets the next version keep the previous one's.

Screenshots belong to a version. A change to the folder reaches the Registry
with the next version Release Control publishes; versions already published
keep what they were published with. CI treats `screenshots/` like `README.md`:
a change there alone is worker metadata, not a source change.

## Limits

The compiler checks every image against the Registry's own limits
(`api/src/lib/screenshots.ts` in iii-hq/registry), so a list the Registry
would refuse fails here, in the PR checks and when the descriptor index is
compiled, never in the middle of a release.

| | Limit |
|---|---|
| Format | PNG, JPEG or WebP (`.png`, `.jpg`, `.jpeg`, `.webp`); the file header must match the extension |
| Size | 8 MiB per image, 128 MiB for all the screenshots of a version |
| Pixels | each side between 320 and 8192, as stored in the file (a JPEG's EXIF orientation is not applied: export upright) |
| Count | 10 per platform, 50 per version |
| Caption | 200 characters (Unicode code points); trimmed; blank is none |
| Duplicates | an image once per list: the same bytes under two names are refused |
| Files | a regular file inside `screenshots/`: symbolic links, folders and devices are refused, and a file over the size limit is refused before it is read |

Every problem is reported at once, each with the index of its entry:

```text
kanban/screenshots/screenshots.yaml: 2 problem(s), the registry would refuse it:
  [1] iphone-board.png: 300x640 pixels; each side must be between 320 and 8192
  [2] mac/board.jpg: a PNG image with a JPEG name; rename it or convert it
```

**Image workers** (`artifact.kind: oci-image`) cannot carry screenshots: the
Registry hosts a version's screenshots on that version's release, and an image
version is published without one. Their list may only be `[]`, which clears
what a version would inherit.

Screenshots are public as soon as a version is published: check them for
hostnames, e-mail addresses, tokens and any other private data first. Prefer
WebP or JPEG to keep them small; the Registry serves the files as uploaded.

## What a release does with them

1. **Compile** ([`deployment_compiler.py`](../../.github/scripts/deployment_compiler.py),
   with [`worker_screenshots.py`](../../.github/scripts/worker_screenshots.py)):
   the descriptor gets a top-level `screenshots`, present only when the list
   exists. Each image is named `<platform>-<n>.<ext>`, the name the Registry
   CLI gives it (`n` counts from 1 per platform in display order; `ext` is
   `png`, `jpg` for any JPEG, or `webp`):

   ```json
   "screenshots": [
     { "name": "android-1.webp", "path": "kanban/screenshots/android-board.webp",
       "platform": "android", "caption": "The board, one lane at a time",
       "sha256": "<64 hex>", "size": 123456, "width": 1080, "height": 2424 }
   ]
   ```

   It is never part of `registry_projection`, and the descriptor digest covers
   it, so a changed image is a changed descriptor.
2. **Build** ([`build.yml`](../../.github/workflows/build.yml)): `assemble`
   copies each image from the source SHA into the prepared release, checked
   byte for byte against the descriptor; `upload` puts it on the shared
   `build-<source_sha>` prerelease as `<worker>-screenshot-<name>` with the
   same rule as the archives (identical bytes are skipped, different bytes fail);
   `manifest` checks the bytes again and writes them into `manifest.json`:

   ```json
   "screenshots": [
     { "name": "android-1.webp", "release_asset": "kanban-screenshot-android-1.webp",
       "platform": "android", "caption": "The board, one lane at a time",
       "sha256": "<64 hex>", "size": 123456, "width": 1080, "height": 2424,
       "url": "https://github.com/iii-hq/workers/releases/download/build-<source_sha>/kanban-screenshot-android-1.webp" }
   ]
   ```

   The manifest has `screenshots` exactly when the descriptor does (`[]`
   included).
3. **Publish** (Release Control): registers each image on the version's
   Registry release as a `kind: screenshot` asset, uploads it from `url`, lists
   them in `POST /publish` (absent field: the version inherits), and reads the
   version back before the deployment converges.

## Check a list locally

```bash
python3 .github/scripts/validate_worker.py \
  --worker <slug> --base-ref origin/main --source-changed '["<slug>"]'

python3 .github/scripts/deployment_compiler.py compile-index \
  --source-sha "$(git rev-parse HEAD)" --compiler-repository iii-hq/workers \
  --output-dir /tmp/deployment-descriptor-index
jq .screenshots /tmp/deployment-descriptor-index/descriptors/<slug>.json
```

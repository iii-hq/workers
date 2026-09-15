#!/usr/bin/env python3
"""Refresh the embedded Piper metadata snapshot, never download model weights.

Run explicitly after reviewing the upstream commit. Large-file SHA-256s come
from Hugging Face LFS metadata; small configs/cards are fetched at that exact
commit and verified against their Git blob ids before their SHA-256 is pinned.
The output is catalog DATA, not executable code. No network is needed at runtime
until the user explicitly requests a model download.
"""
import argparse
import concurrent.futures
import hashlib
import json
from pathlib import Path
import re
import urllib.parse
import urllib.request

REPO = 'rhasspy/piper-voices'
DEFAULT_REVISION = '1162a9173d0ce503555aed757976b7a9912eae4c'


def read(url):
    with urllib.request.urlopen(url, timeout=30) as response:
        return response.read(), response.headers


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--revision', default=DEFAULT_REVISION)
    parser.add_argument('--output', type=Path, default=Path(__file__).resolve().parents[1] / 'catalog/piper-catalog.json')
    args = parser.parse_args()
    if not re.fullmatch(r'[a-f0-9]{40}', args.revision):
        raise ValueError('revision must be an immutable commit SHA')
    if args.output.exists():
        raise FileExistsError(f'{args.output} exists; review the old snapshot before replacing it')
    base = f'https://huggingface.co/{REPO}/resolve/{args.revision}/'
    voices = json.loads(read(base + 'voices.json')[0])
    tree = {}
    url = f'https://huggingface.co/api/models/{REPO}/tree/{args.revision}?recursive=true&expand=false'
    while url:
        data, headers = read(url)
        tree.update((entry['path'], entry) for entry in json.loads(data) if entry['type'] == 'file')
        link = re.search(r'<([^>]+)>; rel="next"', headers.get('Link', ''))
        url = link.group(1) if link else None
        if url and not url.startswith(f'https://huggingface.co/api/models/{REPO}/tree/'):
            raise ValueError('unexpected pagination origin')

    def file_metadata(path):
        entry = tree[path]
        url = base + urllib.parse.quote(path, safe='/')
        if 'lfs' in entry:
            digest = entry['lfs']['oid']
        else:
            if entry['size'] > 1_000_000:
                raise ValueError(f'non-LFS metadata is too large: {path}')
            data, _ = read(url)
            blob = b'blob ' + str(len(data)).encode() + b'\0' + data
            if len(data) != entry['size'] or hashlib.sha1(blob).hexdigest() != entry['oid']:
                raise ValueError(f'Git blob verification failed: {path}')
            digest = hashlib.sha256(data).hexdigest()
        if not re.fullmatch(r'[a-f0-9]{64}', digest):
            raise ValueError(f'invalid SHA-256: {path}')
        return {'name': path.rsplit('/', 1)[-1], 'url': url, 'sha256': digest, 'size_bytes': entry['size']}

    paths = sorted({path for voice in voices.values() for path in voice['files']})
    with concurrent.futures.ThreadPoolExecutor(max_workers=12) as pool:
        files = dict(zip(paths, pool.map(file_metadata, paths)))
    records = []
    for key, voice in sorted(voices.items()):
        language = voice['language']
        ordered = sorted(voice['files'], key=lambda path: (not path.endswith('.onnx'), not path.endswith('.onnx.json'), path))
        directory = ordered[0].rsplit('/', 1)[0]
        assert ordered[0].endswith('.onnx')
        assert any(path.endswith('.onnx.json') for path in ordered)
        records.append({
            'id': 'piper-' + key.lower().replace('_', '-'),
            'name': f"Piper {voice['name']} · {language['name_english']} ({language['region']}) · {voice['quality']}",
            'languages': [language['code'].replace('_', '-')],
            'license': 'See model card (varies by voice)',
            'author': 'Piper voice contributors',
            'source': f'https://huggingface.co/{REPO}/blob/{args.revision}/{urllib.parse.quote(directory, safe="/")}/MODEL_CARD',
            'files': [files[path] for path in ordered],
        })
    args.output.parent.mkdir(parents=True, exist_ok=True)
    snapshot = {'revision': args.revision, 'source': base + 'voices.json', 'voices': records}
    with args.output.open('x', encoding='utf-8') as output:
        json.dump(snapshot, output, ensure_ascii=False, indent=2)
        output.write('\n')
    print(f"Saved metadata for {len(records)} voices, {len({v['languages'][0] for v in records})} locales, {len(files)} files. No weights downloaded.")


if __name__ == '__main__':
    main()

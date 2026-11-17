#!/usr/bin/env python3
"""Generate deterministic, valid and oversized protocol seeds for cargo-fuzz."""
from pathlib import Path
import struct
import zlib

ROOT = Path(__file__).resolve().parents[1]


def u32(value):
    return struct.pack('<I', value)


def string(value):
    encoded = value.encode()
    return u32(len(encoded)) + encoded


def directory_listing():
    return (
        u32(1)
        + string('music\\album')
        + u32(1)
        + b'\x01'
        + string('track.flac')
        + struct.pack('<Q', 4096)
        + string('')
        + u32(1)
        + u32(0)
        + u32(320)
    )


def write_corpus(target, values):
    directory = ROOT / 'fuzz' / 'corpus' / target
    directory.mkdir(parents=True, exist_ok=True)
    for name, value in values.items():
        (directory / name).write_bytes(value)


listing = directory_listing()
search_body = (
    b'\x03'
    + u32(49)
    + string('fixture-user')
    + u32(17)
    + string('ambient')
)
write_corpus(
    'protocol_parser',
    {
        'distributed-search': u32(len(search_body)) + search_body,
        'shared-listing': zlib.compress(listing),
        'folder-reply': zlib.compress(u32(23) + string('music\\album') + listing),
        'truncated-header': b'\xff\xff',
    },
)
write_corpus(
    'decompression',
    {
        'valid-zlib': zlib.compress(listing),
        'over-budget-zlib': zlib.compress(b'A' * (1024 * 1024 + 1)),
        'bad-checksum': zlib.compress(b'checksum')[:-1] + b'\0',
    },
)

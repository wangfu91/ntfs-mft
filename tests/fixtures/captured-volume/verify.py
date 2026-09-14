#!/usr/bin/env python3
"""Verify fixture hashes and independently decode its NTFS metadata (no device access)."""
import collections
import hashlib
import json
from pathlib import Path

def uint(data, offset, size):
    if offset < 0 or offset + size > len(data):
        raise ValueError('truncated field')
    return int.from_bytes(data[offset:offset+size], 'little')

def fixup(raw):
    data = bytearray(raw)
    if data[:4] != b'FILE':
        return None
    off, count = uint(data, 4, 2), uint(data, 6, 2)
    if count < 2 or len(data) % (count-1) or off + count*2 > len(data):
        raise ValueError('invalid USA geometry')
    stride = len(data)//(count-1)
    for i in range(1, count):
        end = i*stride
        if data[end-2:end] != data[off:off+2]:
            raise ValueError('USA mismatch')
        data[end-2:end] = data[off+i*2:off+i*2+2]
    return data

def attrs(data):
    pos, end = uint(data, 20, 2), uint(data, 24, 4)
    if end > len(data):
        raise ValueError('record used length exceeds slot')
    while pos + 4 <= end:
        kind = uint(data, pos, 4)
        if kind == 0xffffffff:
            return
        length = uint(data, pos+4, 4)
        if length < 24 or pos+length > end:
            raise ValueError('invalid attribute length')
        a = data[pos:pos+length]
        yield kind, a
        pos += length
    raise ValueError('missing attribute terminator')

def runs(a):
    pos = uint(a, 32, 2)
    vcn, lcn = uint(a, 16, 8), 0
    result = []
    while pos < len(a):
        header = a[pos]; pos += 1
        if header == 0:
            return result
        width, delta_width = header & 15, header >> 4
        if not 1 <= width <= 8 or delta_width > 8 or pos+width+delta_width > len(a):
            raise ValueError('invalid mapping pair')
        clusters = uint(a, pos, width); pos += width
        if not clusters:
            raise ValueError('empty run')
        physical = None
        if delta_width:
            lcn += int.from_bytes(a[pos:pos+delta_width], 'little', signed=True)
            if lcn < 0:
                raise ValueError('negative LCN')
            physical = lcn
        pos += delta_width
        result.append({'vcn': vcn, 'lcn': physical, 'clusters': clusters})
        vcn += clusters
    raise ValueError('unterminated mapping pairs')

def summary(blob, record_size):
    if len(blob) % record_size:
        raise ValueError('partial trailing MFT record')
    counts = collections.Counter()
    attribute_counts = collections.Counter()
    external_lists = []
    for number in range(len(blob)//record_size):
        data = fixup(blob[number*record_size:(number+1)*record_size])
        if data is None:
            counts['non_FILE_slots'] += 1
            continue
        counts['FILE_slots'] += 1
        in_use = bool(uint(data, 22, 2) & 1)
        counts['in_use_FILE_slots' if in_use else 'unused_FILE_slots'] += 1
        counts['extension_FILE_slots'] += bool(uint(data, 32, 8))
        counts['directory_FILE_slots'] += bool(uint(data, 22, 2) & 2)
        for kind, a in attrs(data):
            attribute_counts[hex(kind)] += 1
            if kind == 0x80 and not a[8]:
                counts['resident_DATA_attributes'] += 1
                counts['resident_DATA_bytes'] += uint(a, 16, 4)
                if not in_use:
                    counts['unused_resident_DATA_bytes'] += uint(a, 16, 4)
            if kind == 0x80 and a[9]:
                counts['named_DATA_attributes'] += 1
            if kind == 0x20 and a[8]:
                external_lists.append({'record': number, 'attribute_id': uint(a,14,2),
                    'lowest_vcn': uint(a,16,8), 'data_size': uint(a,48,8), 'runs': runs(a)})
    return {'record_slots': len(blob)//record_size, 'counts': dict(counts),
            'attribute_type_counts': dict(attribute_counts), 'external_attribute_lists': external_lists}


def main():
    directory = Path(__file__).resolve().parent
    manifest = json.loads((directory / "manifest.json").read_text())
    for item in manifest["ranges"]:
        data = (directory / item["file"]).read_bytes()
        assert len(data) == item["length"], item["file"]
        assert hashlib.sha256(data).hexdigest() == item["sha256"], item["file"]
    blob = (directory / "mft.bin").read_bytes()
    assert summary(blob, 1024) == manifest["inventory"]["mft_inventory"]
    expected = json.loads((directory / "expected.json").read_text())
    for item in expected:
        number = item["record"]
        data = fixup(blob[number*1024:(number+1)*1024])
        names, size = [], None
        for kind, attribute in attrs(data):
            if kind == 0x30:
                value = attribute[uint(attribute, 20, 2):]
                names.append((value[66:66+value[64]*2].decode("utf-16le"), uint(value, 0, 8)))
            if kind == 0x80 and not attribute[9]:
                size = uint(attribute, 48, 8) if attribute[8] else uint(attribute, 16, 4)
        assert names == [(item["name"], item["parent_reference"])], number
        assert uint(data, 16, 2) == item["sequence"], number
        assert bool(uint(data, 22, 2) & 2) == item["directory"], number
        assert size == item["data_size"], number
    print(f"Verified 4 captured ranges, 1280 record slots and {len(expected)} known records")

if __name__ == "__main__":
    main()

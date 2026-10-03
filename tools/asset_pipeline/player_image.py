"""Pinned TU3 static-image extractor. Unknown editions produce no image and no receipt.

Decrypts only the caller's own base XEX plus Title Update 3, using the public XEX2
retail key. Not a general loader and not a gameplay dependency.
"""
import ctypes, hashlib, os, shutil, struct, subprocess, sys
from pathlib import Path

# Public XEX2 retail key (Xenia). Used only to decrypt the caller's own executable.
RETAIL = bytes.fromhex('20B185A59D28FDC340583FBB0896BF91')
XEX_SHA = '1db39496585c521d17a2137804f42cf73ebed2b32cac166ec42dbf772f4dcf7f'
XEXP_SHA = 'eb9ef9109dfa6d940df2e156e7eaeda4603d2b2319ca6451f324b1c27f2b1f4c'
IMAGE_SHA = 'f4aa113eb541bfba03dbc108cf5ab43f58c965b20fa3b82f9c40938a0ad841c4'
IMAGE_SIZE = 0x11B0000
BASE = 0x82000000
TABLE = 0x8302D4A4
HANDLE = 0x8302EE28
PROFILE = 'skate3-tu3-player-image-1'
# xboxkrnl variable exports the loader patches. Names are the public ordinal table.
IMPORTS = {
    0x1C1: 'VdHSIOCalibrationLock', 0xAD: 'KeTimeStampBundle', 0x193: 'XexExecutableModuleHandle',
    0x1AE: 'ExLoadedCommandLine', 0x156: 'XboxHardwareInfo', 0x1C0: 'VdGpuClockInMHz',
    0x1B: 'ExThreadObjectType', 0x158: 'XboxKrnlVersion', 0x59: 'KeDebugMonitorData',
    0x266: 'KeCertMonitorData', 0x1BF: 'VdGlobalXamDevice', 0x1BE: 'VdGlobalDevice',
}
_LIB = None


class PlayerImageError(Exception):
    pass


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def sha_file(path):
    digest = hashlib.sha256()
    with path.open('rb') as stream:
        for chunk in iter(lambda: stream.read(1 << 20), b''):
            digest.update(chunk)
    return digest.hexdigest()


def fixture_path():
    return Path(__file__).with_name('player_objects.txt')


def load_fixture():
    slots = []
    for line in fixture_path().read_text().splitlines():
        if not line or line.startswith('#'):
            continue
        index, project, name_id, name = line.split(maxsplit=3)
        slots.append((int(index), int(project, 16), int(name_id, 16), name))
    if [s[0] for s in slots] != list(range(72)):
        raise PlayerImageError('player object fixture is not 72 ordered slots')
    return slots


def _native():
    global _LIB
    if _LIB:
        return _LIB
    here = Path(__file__).resolve().parent
    root = here.parents[1]
    # The upstream tree contains a Linux .so. Never load that cached ELF on
    # macOS: build a separate Mach-O library for the current Mac architecture.
    macos = sys.platform == 'darwin'
    out = here/'.native'/('libplayer_image.dylib' if macos else 'libplayer_image.so')
    sources = [here/'player_image_native.c', root/'tools/vendor/rijndael/rijndael-alg-fst.c',
               root/'tools/vendor/libmspack/lzxd.c']
    if not out.is_file() or any(src.stat().st_mtime > out.stat().st_mtime for src in sources):
        out.parent.mkdir(parents=True, exist_ok=True)
        compiler = shutil.which('cc') or shutil.which('gcc')
        if not compiler:
            raise PlayerImageError('no C compiler for the player-image helper; audio image not extracted')
        cmd = [compiler, '-O2', '-fPIC', '-dynamiclib' if macos else '-shared', '-I', str(root/'tools/vendor/libmspack'),
               '-I', str(root/'tools/vendor/rijndael'), '-o', str(out), *map(str, sources)]
        run = subprocess.run(cmd, capture_output=True, text=True)
        if run.returncode:
            raise PlayerImageError('player-image helper failed to build; audio image not extracted')
    lib = ctypes.CDLL(str(out))
    lib.aes_cbc_decrypt.argtypes = [ctypes.c_char_p, ctypes.c_void_p, ctypes.c_size_t, ctypes.c_void_p]
    lib.lzx_decompress.argtypes = [ctypes.c_void_p, ctypes.c_size_t, ctypes.c_void_p, ctypes.c_size_t,
                                   ctypes.c_uint32, ctypes.c_void_p, ctypes.c_size_t]
    lib.lzx_decompress.restype = ctypes.c_int
    _LIB = lib
    return lib


def _u32(b, at):
    if at + 4 > len(b):
        raise PlayerImageError(f'truncated at {at:#x}')
    return struct.unpack_from('>I', b, at)[0]


def _opts(b):
    out = {}
    off = 0x18
    for _ in range(_u32(b, 0x14)):
        out[_u32(b, off)] = _u32(b, off + 4)
        off += 8
    return out


def _aes(key, data):
    if len(data) % 16:
        raise PlayerImageError('encrypted region is not block-aligned')
    out = ctypes.create_string_buffer(len(data))
    _native().aes_cbc_decrypt(key, data, len(data), out)
    return out.raw


def _lzx(src, dest_len, window, window_data=None):
    dest = ctypes.create_string_buffer(dest_len)
    wd = None if window_data is None else ctypes.c_char_p(window_data)
    rc = _native().lzx_decompress(src, len(src), dest, dest_len, window, wd, 0 if window_data is None else len(window_data))
    if rc:
        raise PlayerImageError(f'LZX failed ({rc})')
    return dest.raw


def _deblock(blob, size, digest):
    out = bytearray()
    p = 0
    while size:
        if p + size > len(blob) or size < 24:
            raise PlayerImageError('compressed block overruns payload')
        block = blob[p:p + size]
        if hashlib.sha1(block).digest() != digest:
            raise PlayerImageError('compressed block hash mismatch')
        nxt = struct.unpack_from('>I', block, 0)[0]
        ndig = block[4:24]
        q = 24
        while True:
            if q + 2 > len(block):
                raise PlayerImageError('chunk header overruns block')
            chunk = struct.unpack_from('>H', block, q)[0]
            q += 2
            if not chunk:
                break
            if q + chunk > len(block):
                raise PlayerImageError('chunk overruns block')
            out += block[q:q + chunk]
            q += chunk
        p += size
        size, digest = nxt, ndig
    return bytes(out)


def apply_delta(patch, patch_len, window, dest):
    """One LZX-delta record stream. dest is a bytearray. Fails closed on a short record."""
    p = 0
    while p + 12 <= patch_len:
        old, new, ulen, clen = struct.unpack_from('>IIHH', patch, p)
        if clen == ulen == new == old == 0:
            break
        if new + ulen < new or old + ulen < old:
            raise PlayerImageError('delta length overflow')
        end = max(new + ulen, old + ulen)
        if end > len(dest):
            dest.extend(b'\0' * (end - len(dest)))
        if clen == 0:
            dest[new:new + ulen] = b'\0' * ulen
        elif clen == 1:
            dest[new:new + ulen] = dest[old:old + ulen]
        else:
            if p + 12 + clen > len(patch):
                raise PlayerImageError('short delta')
            ref = bytes(dest[old:old + ulen])
            dest[new:new + ulen] = _lzx(patch[p + 12:p + 12 + clen], ulen, window, ref)
        p += 12 if clen <= 1 else 12 + clen
    return dest


def _file_format(b, opts):
    ff = opts[0x3FF]
    window = _u32(b, ff + 8)
    return window, _u32(b, ff + 12), b[ff + 16:ff + 36]


def extract_image(xex, xexp):
    """Return the static image. Caller must already have matched the pinned hashes."""
    base = xex if isinstance(xex, (bytes, bytearray)) else Path(xex).read_bytes()
    patch = xexp if isinstance(xexp, (bytes, bytearray)) else Path(xexp).read_bytes()
    if base[:4] != b'XEX2' or patch[:4] != b'XEX2':
        raise PlayerImageError('not an XEX2 pair')
    header_size = _u32(base, 8)
    sec = _u32(base, 0x10)
    session = _aes(RETAIL, base[sec + 0x150:sec + 0x160])
    window, block, block_hash = _file_format(base, _opts(base))
    packed = _deblock(_aes(session, base[header_size:]), block, block_hash)
    image_field = _u32(base, sec + 4)
    image = bytearray(_lzx(packed, image_field, window))
    if image[:2] != b'MZ':
        raise PlayerImageError('base image is not a PE; refusing to patch it')
    ph = _u32(patch, 8)
    popts = _opts(patch)
    desc = popts[0x5FF]
    pwindow, pblock, phash = _file_format(patch, popts)
    target_headers = _u32(patch, desc + 0x30)
    src_off = _u32(patch, desc + 0x34)
    src_size = _u32(patch, desc + 0x38)
    tgt_off = _u32(patch, desc + 0x3C)
    header = bytearray(base[:header_size])
    if len(header) < target_headers:
        header.extend(b'\0' * (target_headers - len(header)))
    if src_off:
        header[tgt_off:tgt_off + src_size] = header[src_off:src_off + src_size]
    clen = struct.unpack_from('>H', patch, desc + 0x4C + 10)[0]
    apply_delta(patch[desc + 0x4C:], clen + 0xC, pwindow, header)
    del header[target_headers:]
    nsec = _u32(header, 0x10)
    new_field = _u32(header, nsec + 4)
    if new_field != IMAGE_SIZE:
        raise PlayerImageError(f'patched image size {new_field:#x}, pinned profile is {IMAGE_SIZE:#x}')
    new_session = _aes(RETAIL, bytes(header[nsec + 0x150:nsec + 0x160]))
    psec = _u32(patch, 0x10)
    if _aes(new_session, patch[desc + 0x20:desc + 0x30]) != session:
        raise PlayerImageError('title-update image key does not match the base executable')
    plain = _aes(_aes(new_session, patch[psec + 0x150:psec + 0x160]), patch[ph:])
    img_off, img_size, img_tgt = _u32(patch, desc + 0x40), _u32(patch, desc + 0x44), _u32(patch, desc + 0x48)
    if img_off:
        image[img_tgt:img_tgt + img_size] = image[img_off:img_off + img_size]
    p = 0
    size, digest = pblock, phash
    while size:
        if p + size > len(plain):
            raise PlayerImageError('patch block overruns payload')
        block = plain[p:p + size]
        if hashlib.sha1(block).digest() != digest:
            raise PlayerImageError('patch block hash mismatch')
        apply_delta(block[24:], size - 24, pwindow, image)
        p += size
        size, digest = struct.unpack_from('>I', block, 0)[0], block[4:24]
    if len(image) > IMAGE_SIZE:
        raise PlayerImageError('patched image exceeds the pinned size')
    image.extend(b'\0' * (IMAGE_SIZE - len(image)))
    return bytes(image)


def read_slots(image):
    slots = []
    for i in range(72):
        at = TABLE + i * 8 - BASE
        ptr = _u32(image, at)
        project = int.from_bytes(image[at + 4:at + 6], 'big')
        name_id = int.from_bytes(image[at + 6:at + 8], 'big')
        off = ptr - BASE
        if off < 0 or off >= len(image):
            raise PlayerImageError(f'object {i} name pointer is outside the image')
        end = image.find(b'\0', off, off + 128)
        if end < 0:
            raise PlayerImageError(f'object {i} has no terminated name')
        slots.append((i, project, name_id, image[off:end].decode()))
    return slots


def code_covers_slots(image):
    """Each slot index is a dedicated lis/addi pair in code, independent of the name table."""
    words = [int.from_bytes(image[i:i + 4], 'big') for i in range(0, len(image) - 4, 4)]
    pairs = []
    for i, word in enumerate(words):
        if word >> 26 != 15 or (word & 0xFFFF) not in (0x8302, 0x8303) or (word >> 16) & 31:
            continue
        hi, rd = word & 0xFFFF, (word >> 21) & 31
        for nxt in words[i + 1:i + 6]:
            if nxt >> 26 != 14 or ((nxt >> 16) & 31) != rd:
                continue
            imm = nxt & 0xFFFF
            val = ((hi << 16) + (imm - 0x10000 if imm & 0x8000 else imm)) & 0xFFFFFFFF
            if TABLE <= val < TABLE + 72 * 8 or HANDLE <= val < HANDLE + 72 * 8:
                pairs.append((BASE + i * 4, val))
                break
    seen = []
    j = 0
    while j + 1 < len(pairs):
        _, va = pairs[j]
        addr_b, vb = pairs[j + 1]
        if TABLE <= va < TABLE + 72 * 8 and HANDLE <= vb < HANDLE + 72 * 8 and (va - TABLE) // 8 == (vb - HANDLE) // 8 and addr_b - pairs[j][0] <= 16:
            seen.append((va - TABLE) // 8)
            j += 2
        else:
            j += 1
    return seen


def _projects(archive):
    if archive[:2] != b'EB':
        raise PlayerImageError('audio archive is not EB')
    count = _u32(archive, 4)
    shift = archive[0x0A]
    if shift not in (4, 6):
        raise PlayerImageError('unexpected archive offset shift')
    names_at = _u32(archive, 0x0C)
    entries = []
    for i in range(count):
        at = 0x30 + i * 16
        entries.append((_u32(archive, at) << shift, _u32(archive, at + 4), _u32(archive, at + 8)))
    at = names_at
    named = []
    for offset, compressed, uncompressed in entries:
        while at < len(archive) and not 0x20 <= archive[at] <= 0x7E:
            at += 1
        start = at
        while at < len(archive) and archive[at]:
            at += 1
        named.append((offset, compressed, uncompressed, archive[start:at].decode('ascii', 'replace')))
        at += 1
    projects = {}
    for offset, compressed, uncompressed, name in named:
        if not name.endswith('.csi'):
            continue
        if compressed:
            raise PlayerImageError(f'{name} is compressed')
        member = archive[offset:offset + uncompressed]
        if len(member) != uncompressed or member[:4] != b'MOIR':
            raise PlayerImageError(f'{name} is not a MOIR project')
        counts = struct.unpack_from('>HHH', member, 0x0A)
        project = struct.unpack_from('>H', member, 0x10)[0]
        symbols = {}
        cursor = 0x28
        for group, n in enumerate(counts):
            stride = 16 if group == 2 else 12
            for _ in range(n):
                if stride == 12:
                    name_at, name_id = _u32(member, cursor + 4), struct.unpack_from('>H', member, cursor + 8)[0]
                else:
                    name_at, name_id = _u32(member, cursor + 8), struct.unpack_from('>H', member, cursor + 12)[0]
                end = member.find(b'\0', name_at)
                if end < 0:
                    raise PlayerImageError('unterminated CSI name')
                symbols[name_id] = member[name_at:end].decode()
                cursor += stride
        if project in projects:
            raise PlayerImageError(f'duplicate CSI project {project:#x}')
        projects[project] = symbols
    return projects


def slots_agree(image, archive, fixture=None):
    fixture = fixture or load_fixture()
    got = read_slots(image)
    if got != fixture:
        raise PlayerImageError('object slots do not match the pinned order')
    projects = _projects(archive)
    for index, project, name_id, name in got:
        if projects.get(project, {}).get(name_id) != name:
            raise PlayerImageError(f'object {index} {name} is not CSI {project:#x}:{name_id:#x}')
    covered = code_covers_slots(image)
    if sorted(covered) != list(range(72)) or len(covered) != 72:
        raise PlayerImageError('code does not address every object slot exactly once')
    return got


def _entry(image):
    pe = int.from_bytes(image[0x3C:0x40], 'little')
    return BASE + int.from_bytes(image[pe + 40:pe + 44], 'little')


def startup_stores(image):
    """-1 stores in the guest entry, the second of which returns into sub_82EE7828."""
    entry = _entry(image)
    regs = {}
    found = []
    for n in range(40):
        addr = entry + n * 4
        word = _u32(image, addr - BASE)
        op, rd, ra = word >> 26, (word >> 21) & 31, (word >> 16) & 31
        imm = word & 0xFFFF
        simm = imm - 0x10000 if imm & 0x8000 else imm
        if op == 15 and ra == 0:
            regs[rd] = imm << 16
        elif op == 14 and ra == 0 and simm == -1:
            regs[rd] = -1
        elif op == 36 and regs.get(ra) and regs.get(rd) == -1:
            found.append((regs[ra] + simm) & 0xFFFFFFFF)
    return found


def import_slots(xex, image):
    opts = _opts(xex)
    imp = opts[0x103FF]
    size, str_size, str_count = struct.unpack_from('>III', xex, imp)
    strings, i, names = xex[imp + 12:imp + 12 + str_size], 0, []
    while i < str_size and len(names) < str_count:
        end = strings.find(b'\0', i)
        names.append(strings[i:end].decode())
        i = end + 1
        if i % 4:
            i += 4 - i % 4
    lib = imp + 12 + str_size
    slots = []
    while lib + 0x28 <= imp + size:
        lsize = _u32(xex, lib)
        if not lsize:
            break
        name_index, count = struct.unpack_from('>HH', xex, lib + 0x24)
        lib_name = names[name_index & 0xFF]
        for n in range(count):
            addr = _u32(xex, lib + 0x28 + 4 * n)
            if BASE <= addr < BASE + len(image):
                value = _u32(image, addr - BASE)
                if (value >> 24) == 0:
                    slots.append((addr, lib_name, value & 0xFFFF, value))
        lib += lsize
    return slots


def explain_live_delta(static, live, xex):
    """Why the early live capture differs from the static image. Does not patch either."""
    if len(static) != IMAGE_SIZE or len(live) < IMAGE_SIZE:
        raise PlayerImageError('capture lengths do not match the pinned image')
    diffs = [i for i in range(0, IMAGE_SIZE, 4) if static[i:i + 4] != live[i:i + 4]]
    imports = {addr: (lib, ordinal, static_value) for addr, lib, ordinal, static_value in import_slots(xex, static)}
    stores = set(startup_stores(static))
    explained = []
    for at in diffs:
        addr = BASE + at
        live_word = _u32(live, at)
        if addr in imports:
            lib, ordinal, static_value = imports[addr]
            name = IMPORTS.get(ordinal)
            if lib != 'xboxkrnl.exe' or name is None or static_value != (ordinal | 0x10000):
                raise PlayerImageError(f'unexplained import diff at {addr:#x}')
            explained.append(f'{addr:#x} xboxkrnl {name} ordinal {ordinal:#x} patched after the static dump')
        elif addr in stores and live_word == 0xFFFFFFFF and _u32(static, at) == 0:
            explained.append(f'{addr:#x} guest entry stores -1 before the live-capture breakpoint')
        else:
            raise PlayerImageError(f'unexplained live difference at {addr:#x}')
    if not explained:
        raise PlayerImageError('live capture is identical to the static image')
    return explained


def _regions(image):
    pe = int.from_bytes(image[0x3C:0x40], 'little')
    nsec, opt = struct.unpack_from('<HH', image, pe + 6)[0], pe + 24
    optsize = struct.unpack_from('<H', image, pe + 20)[0]
    regions = [{'name': 'object_table', 'guest': TABLE, 'offset': TABLE - BASE, 'length': 72 * 8}]
    sec = opt + optsize
    for i in range(nsec):
        raw = image[sec + i * 40:sec + i * 40 + 40]
        name = raw[:8].split(b'\0', 1)[0].decode('ascii', 'replace')
        vsize, va = struct.unpack_from('<II', raw, 8)
        chars = struct.unpack_from('<I', raw, 36)[0]
        if chars & 0x80000000:
            continue
        regions.append({'name': name, 'guest': BASE + va, 'offset': va, 'length': vsize, 'readonly': True})
    return regions


def _pages(image, page_bytes=0x200000):
    """The runtime's `g_XXXX.bin` region convention: pages of the image from its base.

    `PlayerAudioCatalog::load_guest_image` maps `g_XXXX.bin` at `XXXX << 16` and accepts pages
    0x8200..=0x8320, so the static image is published as whole pages, the last one short.
    """
    pages = []
    for offset in range(0, len(image), page_bytes):
        page = (BASE + offset) >> 16
        pages.append((page, image[offset:offset + page_bytes]))
    return pages


def publish(dest, image, xex_sha, xexp_sha, archive_sha):
    if sha256(image) != IMAGE_SHA:
        raise PlayerImageError('refusing to publish an image that is not the pinned static digest')
    dest = Path(dest)
    if dest.exists():
        shutil.rmtree(dest)
    dest.mkdir(parents=True)
    pages = []
    for page, chunk in _pages(image):
        blob = dest/('g_%04X.bin.part' % page)
        blob.write_bytes(chunk)
        if sha_file(blob) != sha256(chunk):
            blob.unlink()
            raise PlayerImageError('image changed while writing')
        final = dest/('g_%04X.bin' % page)
        blob.replace(final)
        pages.append({'page': page, 'name': final.name, 'guest': page << 16,
                      'bytes': len(chunk), 'sha256': sha256(chunk)})
    receipt = {
        'profile': PROFILE, 'xex_sha256': xex_sha, 'xexp_sha256': xexp_sha,
        'image_sha256': IMAGE_SHA, 'archive_sha256': archive_sha,
        'fixture_sha256': sha_file(fixture_path()), 'base': BASE, 'size': IMAGE_SIZE,
        'layout': 'g_pages', 'pages': pages, 'regions': _regions(image),
        'mutable_after_load': 'not applied; see explain_live_delta',
    }
    path = dest/'receipt.json'
    tmp = path.with_suffix('.json.part')
    tmp.write_text(__import__('json').dumps(receipt, separators=(',', ':')), encoding='utf-8')
    tmp.replace(path)
    return path


def _published_image(dest, page_bytes=0x200000):
    """Reassemble the published pages, or `None` when the layout is not the pinned one."""
    dest = Path(dest)
    parts = []
    for offset in range(0, IMAGE_SIZE, page_bytes):
        path = dest/('g_%04X.bin' % ((BASE + offset) >> 16))
        if not path.is_file():
            return None
        parts.append(path.read_bytes())
    image = b''.join(parts)
    return image if len(image) == IMAGE_SIZE else None


def valid_receipt(dest):
    dest = Path(dest)
    path = dest/'receipt.json'
    try:
        import json
        receipt = json.loads(path.read_text())
        image = _published_image(dest)
        return (receipt.get('profile') == PROFILE and receipt.get('layout') == 'g_pages'
                and receipt.get('image_sha256') == IMAGE_SHA and receipt.get('xex_sha256') == XEX_SHA
                and receipt.get('xexp_sha256') == XEXP_SHA
                and receipt.get('fixture_sha256') == sha_file(fixture_path())
                and image is not None and sha256(image) == IMAGE_SHA
                and not (dest/'reason.json').exists())
    except (OSError, ValueError, KeyError, TypeError):
        return False


def _reason(dest, text):
    dest = Path(dest)
    if dest.exists():
        shutil.rmtree(dest)
    dest.mkdir(parents=True)
    (dest/'reason.json').write_text(__import__('json').dumps({'enabled': False, 'reason': text}), encoding='utf-8')
    return text


def install_player_image(game_root, dest, xexp=None):
    """Extract or explain why not. Never raises. Does not write a receipt for a failed gate.

    The Title Update is looked for where an installed copy keeps it: beside the executable, one
    directory up (an installation's `game/` beside the disc root), an explicit path, or the
    setup-time `SKATE_TITLE_UPDATE` convenience. A missing update is a reported capability,
    never a setup failure.
    """
    try:
        game_root = Path(game_root)
        xex = game_root/'default.xex'
        archive = game_root/'data/audio/audiofiles.big'
        candidates = ([Path(xexp)] if xexp else []) + [
            game_root/'default.xexp', game_root.parent/'default.xexp']
        if os.environ.get('SKATE_TITLE_UPDATE'):
            candidates.append(Path(os.environ['SKATE_TITLE_UPDATE']))
        update = next((path for path in candidates if path.is_file()), None)
        if not xex.is_file() or update is None:
            return _reason(dest, 'Title Update 3 default.xexp was not found beside default.xex; player audio image not extracted')
        xex_sha, xexp_sha = sha_file(xex), sha_file(update)
        if xex_sha != XEX_SHA or xexp_sha != XEXP_SHA:
            return _reason(dest, 'executable edition is not the pinned Title Update 3 profile; player audio left disabled')
        if not archive.is_file():
            return _reason(dest, 'audiofiles.big is missing; player audio image not published')
        image = extract_image(xex.read_bytes(), update.read_bytes())
        if sha256(image) != IMAGE_SHA:
            return _reason(dest, 'extracted image does not match the pinned static digest; no receipt published')
        slots_agree(image, archive.read_bytes())
        publish(dest, image, xex_sha, xexp_sha, sha_file(archive))
        return 'Player audio image extracted from the pinned Title Update 3 executable'
    except PlayerImageError as error:
        return _reason(dest, str(error) + '; player audio left disabled')
    except OSError as error:
        return _reason(dest, f'{error}; player audio left disabled')

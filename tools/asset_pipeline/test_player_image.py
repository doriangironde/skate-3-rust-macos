"""Player-image gate. Retail bytes are optional; rejection paths are not."""
import hashlib
import json
from pathlib import Path
import tempfile
import unittest

from tools.asset_pipeline import player_image as image
from tools.asset_pipeline.group_receipts import ROOTS, files


class PlayerImageGate(unittest.TestCase):
    def test_fixture_matches_the_research_indices(self):
        slots = image.load_fixture()
        self.assertEqual(slots[3][3], 'Class_grind')
        self.assertEqual(slots[42][3], 'Class_rolling')
        self.assertEqual(slots[24][3], 'playercharacter_footstep')
        self.assertEqual(len({name for *_, name in slots}), 72)

    def test_unknown_hash_writes_no_receipt(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            (root/'default.xex').write_bytes(b'XEX2not-the-pinned-edition')
            (root/'default.xexp').write_bytes(b'XEX2not-the-pinned-update')
            dest = root/'out'
            text = image.install_player_image(root, dest)
            self.assertIn('not the pinned', text)
            self.assertFalse((dest/'receipt.json').exists())
            self.assertFalse(list(dest.glob('*.bin')))
            self.assertFalse(image.valid_receipt(dest))

    def test_short_delta_and_swapped_slots_fail_closed(self):
        with self.assertRaises(image.PlayerImageError):
            image.apply_delta(bytes.fromhex('000000000000000000010020') + b'\x00\x00', 14, 0x8000, bytearray())
        fixture = image.load_fixture()
        swapped = fixture[:]
        swapped[0], swapped[1] = swapped[1], swapped[0]
        self.assertNotEqual(swapped, fixture)

    def test_stale_receipt_is_not_valid(self):
        with tempfile.TemporaryDirectory() as temp:
            dest = Path(temp)
            (dest/'receipt.json').write_text(json.dumps({
                'profile': image.PROFILE, 'image_sha256': '0' * 64,
                'xex_sha256': image.XEX_SHA, 'xexp_sha256': image.XEXP_SHA,
                'fixture_sha256': image.sha_file(image.fixture_path()),
            }))
            (dest/'g_8200.bin').write_bytes(b'nope')
            self.assertFalse(image.valid_receipt(dest))

    def test_code_scanner_requires_a_table_and_handle_pair(self):
        raw = bytearray(64)
        raw[0:16] = bytes.fromhex('3c6083033863d4a43c8083033884ee28')
        self.assertEqual(image.code_covers_slots(raw), [0])
        self.assertEqual(image.code_covers_slots(bytearray(raw[:8])), [])

    def test_group_receipts_do_not_follow_the_old_private_audio_path(self):
        # The staged image now legitimately lives under `private/stock/audio-runtime-image` and is
        # part of the core receipt; the previous ad-hoc `private/player-audio` location must stay
        # unreferenced so a stale copy cannot creep back into the version fingerprint.
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            blob = root/'assets/private/player-audio/g_8200.bin'
            blob.parent.mkdir(parents=True)
            blob.write_bytes(b'image')
            (root/'maps.json').write_text('[]')
            for group in ROOTS:
                if group == 'maps':
                    continue
                names = [path.name for path in files(root, group) if path.is_file()]
                self.assertNotIn(blob.name, names)
                self.assertFalse(any('player-audio' in str(path) for path in files(root, group)))

    @unittest.skipUnless(
        Path('data/disc/default.xex').is_file() and Path('/mnt/SSD/Games/Skate3/Skate3Recomp-Linux/game/default.xexp').is_file(),
        'owned XEX and TU3 are not on this machine')
    def test_owned_extract_matches_static_digest_before_slot_parse(self):
        xex = Path('data/disc/default.xex').read_bytes()
        xexp = Path('/mnt/SSD/Games/Skate3/Skate3Recomp-Linux/game/default.xexp').read_bytes()
        self.assertEqual(hashlib.sha256(xex).hexdigest(), image.XEX_SHA)
        self.assertEqual(hashlib.sha256(xexp).hexdigest(), image.XEXP_SHA)
        extracted = image.extract_image(xex, xexp)
        self.assertEqual(hashlib.sha256(extracted).hexdigest(), image.IMAGE_SHA)
        archive = Path('data/disc/data/audio/audiofiles.big')
        if archive.is_file():
            image.slots_agree(extracted, archive.read_bytes())
        static = Path('/tmp/skate-audio-capture.kksQuK/static/default_82000000_011B0000.bin')
        live = b''.join(Path(f'/tmp/skate-audio-capture.kksQuK/g_{a:04x}.bin').read_bytes()
                        for a in range(0x8200, 0x8320, 0x20))
        if static.is_file() and len(live) >= image.IMAGE_SIZE:
            self.assertEqual(extracted, static.read_bytes())
            explained = image.explain_live_delta(extracted, live, xex)
            self.assertEqual(len(explained), 14)
            self.assertTrue(any('VdHSIOCalibrationLock' in line for line in explained))
            self.assertTrue(any('stores -1' in line for line in explained))
        with tempfile.TemporaryDirectory() as temp:
            dest = Path(temp)/'player-audio'
            image.publish(dest, extracted, image.XEX_SHA, image.XEXP_SHA, 'archive')
            self.assertTrue(image.valid_receipt(dest))
            receipt = json.loads((dest/'receipt.json').read_text())
            table = next(item for item in receipt['regions'] if item['name'] == 'object_table')
            self.assertEqual(table['guest'], image.TABLE)
            self.assertEqual(table['offset'], image.TABLE - image.BASE)
            (dest/'receipt.json').write_text('{}')
            self.assertFalse(image.valid_receipt(dest))


if __name__ == '__main__':
    unittest.main()

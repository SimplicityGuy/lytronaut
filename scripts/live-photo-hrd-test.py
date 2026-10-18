#!/usr/bin/env python3
"""Adversarial tests of the narrow external HRD experiment, not product code."""
import importlib.util
import pathlib
import sys
import struct
import unittest

sys.dont_write_bytecode = True
spec = importlib.util.spec_from_file_location('live_photo_hrd', pathlib.Path(__file__).with_name('live-photo-hrd.py'))
hrd = importlib.util.module_from_spec(spec)
spec.loader.exec_module(hrd)


class ScheduleTests(unittest.TestCase):
    def test_inferred_vbr_schedule_with_actual_size_variation(self):
        for count, macroblocks in [(2, 1), (3, 8352), (64, 256)]:
            bound = macroblocks * 580 + 64
            hrd.admitted_bound(macroblocks, count)
            sizes = [macroblocks * 386 + 8 + i % 2 for i in range(count)]
            for rate, capacity in [(135000000, 135000000), (162000000, 162000000)]:
                result = hrd.schedule(sizes, macroblocks, rate, capacity)
                self.assertEqual(result['inferredCbrFlag'], 0)
                self.assertEqual(len(result['rows']), count)
                self.assertEqual(result['externalCpbRemovalDelays'], [2] * count)
                self.assertTrue(all(size <= bound for size in sizes))

    def test_every_idr_buffering_period_initialization(self):
        for count, macroblocks in [(2, 1), (3, 8352), (64, 256)]:
            sizes = [macroblocks * 386 + 8 + i % 2 for i in range(count)]
            for rate in [135000000, 162000000]:
                runs = hrd.every_initialization(sizes, macroblocks, rate, rate)
                self.assertEqual(len(runs), count)
                for start, run in enumerate(runs):
                    self.assertEqual(run['hrdInitializedAtOriginalAu'], start)
                    self.assertEqual(run['externalTimeScale'], 2 * count)
                    self.assertEqual(run['externalBufferingPeriodIndices'], list(range(start, count)))
                    self.assertEqual(len(run['rows']), count - start)
                    self.assertEqual(run['externalInitialDelayOffset90k'], 0)

    def test_all_admitted_geometry_and_count_margins(self):
        largest = 0
        for width in range(2, 2049, 2):
            max_height = min(2048, 2097152 // width)
            for height in range(2, max_height + 1, 2):
                w, h = (width + 15) // 16, (height + 15) // 16
                largest = max(largest, w * h)
                self.assertLessEqual(w * h, hrd.MAX_FS)
                self.assertLessEqual(max(w * w, h * h), 8 * hrd.MAX_FS)
        self.assertEqual(largest, 8352)
        # Every margin gets harder as count increases at fixed macroblock size;
        # checking the largest permitted count covers every smaller count.
        for macroblocks in range(1, largest + 1):
            max_count = min(64, (hrd.CAP - 8192) // (macroblocks * 580 + 64))
            self.assertGreaterEqual(max_count, 2)
            hrd.admitted_bound(macroblocks, max_count)

    def test_invalid_fields_timing_and_counts_fail_closed(self):
        for sizes, macroblocks, rate, capacity in [([1], 1, 135000000, 135000000), ([1, 1], 1, 135000000, 135000000), ([400, 400], 1, 135000001, 135000000), ([400, 400], 1, 135000000, 135000001), ([400, 400], 22081, 135000000, 135000000)]:
            with self.assertRaises(ValueError):
                hrd.schedule(sizes, macroblocks, rate, capacity)
        with self.assertRaises(ValueError):
            hrd.admitted_bound(8352, 64)

    def test_corrupt_or_truncated_movie_cannot_supply_counts(self):
        for data in [b'', b'\x00\x00\x00\x07ftyp', b'\x00\x00\x00\x20mdat', b'\x00\x00\x00\x08ftyp']:
            with self.assertRaises(ValueError):
                hrd.movie_samples(data)

    def test_invalid_prevention_and_rbsp_bits_are_rejected(self):
        for nal in [b'\x65\x00\x00\x00', b'\x65\x00\x00\x03', b'\x65\x00\x00\x03\x04']:
            with self.assertRaises(ValueError):
                hrd.Bits(nal, 0x65)
        with self.assertRaises(ValueError):
            hrd.pcm_picture(b'\x65\x80', 1, 0)
        with self.assertRaises(ValueError):
            hrd.parameter_sets(b'\x67\x80', b'\x68\x80')

    def test_composition_offsets_and_video_edits_fail_closed(self):
        # Minimal table-reader fixture, not a valid encoded SPS/slice fixture.
        # Its only purpose is to ensure unsupported presentation transforms are
        # rejected before the codec verifier can claim uniform movie spacing.
        def atom(kind, payload):
            return struct.pack('>I4s', 8 + len(payload), kind) + payload
        def words(*values):
            return struct.pack('>' + 'I' * len(values), *values)
        sample = b'\x65synthetic'
        framed = words(len(sample)) + sample
        def movie(ctts=False, edits=False):
            entry = bytearray(78)
            entry[24:28] = struct.pack('>HH', 16, 16)
            config = bytes([1, 66, 192, 50, 255, 225, 0, 2, 103, 128, 1, 0, 2, 104, 128])
            desc = atom(b'stsd', words(0, 1) + atom(b'avc1', bytes(entry) + atom(b'avcC', config)))
            tables = desc + atom(b'stts', words(0, 1, 2, 3000)) + atom(b'stsc', words(0, 1, 1, 1, 1)) + atom(b'stsz', words(0, 0, 2, len(framed), len(framed))) + atom(b'stco', words(0, 2, 16, 16 + len(framed)))
            if ctts:
                tables += atom(b'ctts', words(0, 2, 1, 3000, 1, 0))
            mdia = atom(b'mdhd', words(0, 0, 0, 2000, 6000)) + atom(b'hdlr', words(0, 0) + b'vide') + atom(b'minf', atom(b'stbl', tables))
            track = atom(b'mdia', mdia) + (atom(b'edts', b'') if edits else b'')
            return atom(b'ftyp', b'') + atom(b'mdat', framed * 2) + atom(b'moov', atom(b'trak', track))
        self.assertEqual(len(hrd.movie_samples(movie())[2]), 2)
        with self.assertRaisesRegex(ValueError, 'CTTS'):
            hrd.movie_samples(movie(ctts=True))
        with self.assertRaisesRegex(ValueError, 'video edit'):
            hrd.movie_samples(movie(edits=True))


if __name__ == '__main__':
    unittest.main()

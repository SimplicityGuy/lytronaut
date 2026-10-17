#!/usr/bin/env python3
"""Independent, deliberately narrow public I_PCM conformance experiment.

Primary specification: ITU-T H.264 (08/2024), A.3.1/Table A-1, C.1-C.3,
D.2.2-D.2.3, E.2.1-E.2.2. QTFF sample tables supply the actual access units.
This supplies missing parameters OUTSIDE the stream under E.2.1 notes 14/15.
It does not show that Apple consumes this schedule, replace decoder/Photos
tests, or establish general H.264/QuickTime conformance. See UNCOVERED below.
"""
import argparse
from fractions import Fraction as Q
import hashlib
import json
import pathlib

CAP = 16 * 1024 * 1024
MBPS = 589824
MAX_FS = 22080
MAX_DPB = 110400
UNCOVERED = [
    'Independent review of this verifier and external-parameter interpretation',
    'Apple consumption or delivery timing of externally supplied HRD parameters',
    'General H.264 syntax/profiles, other NAL types, multiple slices, or other containers',
    'QuickTime pairing, metadata, color reconstruction, JPEG and Photos GUI import',
    'Actual physical CPB/DPB implementation, memory budgets or physical presentation',
]


def require(condition, message):
    if not condition:
        raise ValueError(message)


class Bits:
    def __init__(self, nal, header):
        require(nal and nal[0] == header, 'unexpected NAL header')
        output = bytearray()
        zeros = 0
        i = 1
        while i < len(nal):
            byte = nal[i]
            if zeros == 2:
                if byte == 3:
                    require(i + 1 < len(nal) and nal[i + 1] <= 3, 'invalid prevention byte')
                    zeros = 0
                    i += 1
                    continue
                require(byte > 2, 'missing prevention byte')
            output.append(byte)
            zeros = zeros + 1 if byte == 0 else 0
            i += 1
        self.data = bytes(output)
        self.bit = 0

    def n(self, count):
        require(0 <= count <= 32 and self.bit + count <= len(self.data) * 8, 'RBSP extent')
        value = 0
        for _ in range(count):
            value = (value << 1) | ((self.data[self.bit // 8] >> (7 - self.bit % 8)) & 1)
            self.bit += 1
        return value

    def ue(self):
        zeros = 0
        while self.n(1) == 0:
            zeros += 1
            require(zeros <= 31, 'Exp-Golomb field extent')
        return (1 << zeros) - 1 + self.n(zeros)

    def alignment(self):
        while self.bit % 8:
            require(self.n(1) == 0, 'nonzero alignment')

    def end(self):
        require(self.n(1) == 1, 'missing stop bit')
        self.alignment()
        require(self.bit == len(self.data) * 8, 'extra RBSP data')


def parameter_sets(sps, pps):
    r = Bits(sps, 0x67)
    require([r.n(8), r.n(8), r.n(8)] == [66, 192, 50], 'expected constrained Baseline Level 5')
    require([r.ue(), r.ue(), r.ue(), r.ue(), r.n(1)] == [0, 0, 2, 0, 0], 'unsupported SPS frame/POC/reference structure')
    w, h = r.ue() + 1, r.ue() + 1
    require([r.n(1), r.n(1)] == [1, 1], 'progressive picture required')
    crop = [r.ue() for _ in range(4)] if r.n(1) else [0, 0, 0, 0]
    width, height = w * 16 - 2 * sum(crop[:2]), h * 16 - 2 * sum(crop[2:])
    require([r.n(1), r.n(1), r.n(8), r.n(1)] == [1, 1, 1, 0], 'unsupported VUI aspect/overscan')
    require([r.n(1), r.n(3), r.n(1), r.n(1), r.n(8), r.n(8), r.n(8)] == [1, 5, 0, 1, 1, 13, 6], 'unsupported color declaration')
    require([r.n(1) for _ in range(5)] == [0] * 5, 'timing, HRD and pic_struct must be absent')
    require(r.n(1) == 1, 'explicit VUI restrictions required')
    require([r.n(1)] + [r.ue() for _ in range(6)] == [0, 0, 1, 0, 0, 0, 1], 'unsupported restrictions')
    r.end()
    p = Bits(pps, 0x68)
    require([p.ue(), p.ue(), p.n(1), p.n(1)] == [0, 0, 0, 0], 'unsupported PPS')
    require([p.ue(), p.ue(), p.ue(), p.n(1), p.n(2)] == [0, 0, 0, 0, 0], 'unsupported slice groups/references/prediction')
    require([p.ue(), p.ue(), p.ue(), p.n(1), p.n(1), p.n(1)] == [0, 0, 0, 1, 0, 0], 'unsupported PPS quantization/deblocking')
    p.end()
    require(0 < width <= 2048 and 0 < height <= 2048 and width % 2 == height % 2 == 0 and width * height <= 2097152, 'unadmitted visible geometry')
    require(w == (width + 15) // 16 and h == (height + 15) // 16, 'unexpected crop extent')
    require(w * h <= MAX_FS and w * w <= 8 * MAX_FS and h * h <= 8 * MAX_FS, 'A.3.1(e-g)')
    require(1 <= min(MAX_DPB // (w * h), 16), 'A.3.1(h)')
    return width, height, w * h


def pcm_picture(nal, macroblocks, expected_id):
    r = Bits(nal, 0x65)
    require([r.ue(), r.ue(), r.ue(), r.n(4), r.ue()] == [0, 2, 0, 0, expected_id], 'unexpected IDR slice header')
    require([r.n(1), r.n(1), r.ue(), r.ue()] == [0, 0, 0, 1], 'unexpected IDR marking/QP/deblocking')
    peak = 0
    for _ in range(macroblocks):
        start = r.bit
        require(r.ue() == 25, 'only actual I_PCM macroblocks are supported')
        r.alignment()
        r.bit += 384 * 8
        require(r.bit < len(r.data) * 8, 'PCM payload extent')
        peak = max(peak, r.bit - start)
        require(r.bit - start <= 3088 <= 3200, 'actual PCM syntax/A.3.1(n)/E.2.1 macroblock bound')
    r.end()
    return peak


def movie_samples(data):
    require(len(data) <= CAP, 'movie cap')
    def word(offset, end, size=4):
        require(0 <= offset and offset + size <= end <= len(data), 'table extent')
        return int.from_bytes(data[offset:offset + size], 'big')
    def boxes(start, end):
        result = []
        while start < end:
            size = word(start, end)
            require(size >= 8 and start + size <= end and len(result) < 128, 'atom extent')
            result.append((data[start + 4:start + 8], start + 8, start + size))
            start += size
        require(start == end, 'atom framing')
        return result
    def one(items, name):
        found = [b for b in items if b[0] == name]
        require(len(found) == 1, 'expected unique atom')
        return found[0]
    def children(box, skip=0):
        return boxes(box[1] + skip, box[2])
    def at(box, offset):
        return word(box[1] + offset, box[2])
    root = boxes(0, len(data))
    require([b[0] for b in root] == [b'ftyp', b'mdat', b'moov'], 'unsupported movie layout')
    mdat = root[1]
    tracks = []
    for track in children(root[2]):
        if track[0] != b'trak':
            continue
        track_boxes = children(track)
        mdia = children(one(track_boxes, b'mdia'))
        handler = one(mdia, b'hdlr')
        if data[handler[1] + 8:handler[1] + 12] == b'vide':
            require(not any(box[0] == b'edts' for box in track_boxes), 'video edit lists unsupported')
            tracks.append(mdia)
    require(len(tracks) == 1, 'one video track required')
    mdia = tracks[0]
    tables = children(one(children(one(mdia, b'minf')), b'stbl'))
    require(sorted(box[0] for box in tables) == sorted([b'stsd', b'stts', b'stsc', b'stsz', b'stco']), 'unsupported or duplicate video sample table; CTTS is not supported')
    offsets, sizes = one(tables, b'stco'), one(tables, b'stsz')
    n = at(offsets, 4)
    require(at(offsets, 0) == at(sizes, 0) == 0, 'sample table version/flags')
    require(2 <= n <= 64 and at(sizes, 4) == 0 and at(sizes, 8) == n, 'sample count/size table')
    require(offsets[2] - offsets[1] == 8 + 4 * n and sizes[2] - sizes[1] == 12 + 4 * n, 'exact sample table lengths')
    mdhd, stts, stsc = one(mdia, b'mdhd'), one(tables, b'stts'), one(tables, b'stsc')
    require(at(mdhd, 0) == 0 and at(mdhd, 12) == n * 1000 and at(mdhd, 16) == n * 3000, 'three-second movie timing')
    require([at(stts, k) for k in [0, 4, 8, 12]] == [0, 1, n, 3000], 'uniform sample timing required')
    require([at(stsc, k) for k in [0, 4, 8, 12, 16]] == [0, 1, 1, 1, 1], 'one sample per chunk required')
    desc = one(tables, b'stsd')
    require(at(desc, 0) == 0 and at(desc, 4) == 1, 'one sample description required')
    entry = one(children(desc, 8), b'avc1')
    avcc = one(children(entry, 78), b'avcC')
    config = data[avcc[1]:avcc[2]]
    require(config[:6] == bytes([1, 66, 192, 50, 255, 225]), 'unsupported AVC configuration')
    position = 6
    def nal_parameter():
        nonlocal position
        require(position + 2 <= len(config), 'parameter length extent')
        size = int.from_bytes(config[position:position + 2], 'big')
        position += 2
        require(size > 0 and position + size <= len(config), 'parameter extent')
        result = config[position:position + size]
        position += size
        return result
    sps = nal_parameter()
    require(position < len(config) and config[position] == 1, 'one PPS required')
    position += 1
    pps = nal_parameter()
    require(position == len(config), 'extra configuration data')
    samples, extents = [], []
    for i in range(n):
        start, size = at(offsets, 8 + 4 * i), at(sizes, 12 + 4 * i)
        require(mdat[1] <= start and size >= 5 and start + size <= mdat[2], 'sample outside mdat')
        require(all(start + size <= a or start >= b for a, b in extents), 'overlapping samples')
        extents.append((start, start + size))
        require(word(start, start + size) == size - 4, 'exactly one length-prefixed NAL per sample required')
        samples.append(data[start + 4:start + size])
    dimensions = [word(entry[1] + offset, entry[2], 2) for offset in [24, 26]]
    return sps, pps, samples, dimensions


def schedule(byte_counts, macroblocks, rate, capacity, original_count=None, init_index=0):
    """E.2.2 inferred VBR parameters; external BP/timing values, never stream edits."""
    n = len(byte_counts)
    original_count = n if original_count is None else original_count
    require(1 <= n <= original_count <= 64 and original_count >= 2, 'picture count')
    require(init_index >= 0 and init_index + n == original_count, 'initialization suffix identity')
    require(1 <= macroblocks <= MAX_FS and all(isinstance(size, int) and size > 0 for size in byte_counts), 'picture geometry/byte counts')
    tick = Q(3, 2 * original_count)
    initial = Q(1, 90000)
    require(0 < 3 < 2**32 and 0 < 2 * original_count < 2**32, 'external clock field widths')
    require(rate % 64 == 0 and 0 <= rate // 64 - 1 <= 2**32 - 2, 'E-81 representation')
    require(capacity % 16 == 0 and 0 <= capacity // 16 - 1 <= 2**32 - 2, 'E-82 representation')
    require(1 <= 90000 * Q(capacity, rate), 'D.2.2 initial delay bound')
    # D.2.2 requires an external BP at EVERY IDR once BP presence is asserted.
    # Each uses initial_delay=1, offset=0, SPS0; picture timing delay=2 ticks.
    # C-8 advances nominal time from the preceding BP. C-5, not C-4, applies.
    require(2 < 2**24, '24-bit delay fields')
    initial_arrival, final_arrival, actual, output = [], [], [], []
    rows = []
    for i, count in enumerate(byte_counts):
        nominal = initial + tick * 2 * i  # C-7/C-8 with every-AU BP
        start = Q(0) if i == 0 else max(final_arrival[-1], nominal - initial)  # C-3/C-5
        finish = start + Q(count * 8, rate)  # C-6
        late = max(Q(0), (finish - nominal) / tick)
        rounded = (late.numerator + late.denominator - 1) // late.denominator
        removed = nominal + tick * rounded  # C-10/C-11; inferred low_delay=1
        require(removed == initial + tick * (1 + 2 * i), 'derived schedule does not match prospective algorithm')
        require(removed >= finish, 'complete AU required at actual removal')
        if i == 0:
            limit = 192 * (max(Q(macroblocks), Q(MBPS, 172)) + MBPS * (removed - nominal))
        else:
            bp_interval = 90000 * (nominal - final_arrival[-1])
            maximum_initial_delay = (bp_interval.numerator + bp_interval.denominator - 1) // bp_interval.denominator
            require(1 <= maximum_initial_delay, 'C.3(1) subsequent buffering-period initial delay')
            require(nominal - actual[-1] >= max(Q(macroblocks, MBPS), Q(1, 172)), 'A.3.1(a) removal spacing')
            limit = 192 * MBPS * (removed - actual[-1])
            require(removed - output[-1] >= max(Q(macroblocks, MBPS), Q(1, 172)), 'A.3.1(b)/C.3(7) output spacing')
        require(count <= limit, 'A.3.1(c/d) access-unit byte bound')
        initial_arrival.append(start)
        final_arrival.append(finish)
        actual.append(removed)
        output.append(removed)  # external dpb_output_delay=0; no reordering
        rows.append({'bytes': count, 'initialArrival': str(start), 'finalArrival': str(finish), 'nominalRemoval': str(nominal), 'actualRemoval': str(removed), 'output': str(removed), 'byteLimit': str(limit)})
    # Between arrivals/removals occupancy is piecewise linear; maxima occur just
    # before removals or at final arrivals. Include both sides of coincident events.
    peak = Q(0)
    for moment in sorted(set(final_arrival + actual)):
        arrived = sum(min(Q(count * 8), max(Q(0), (moment - start) * rate)) for count, start in zip(byte_counts, initial_arrival))
        departed_before = sum(count * 8 for count, removal in zip(byte_counts, actual) if removal < moment)
        before = arrived - departed_before
        require(0 <= before <= capacity, 'C.3(2) CPB overflow/negative occupancy')
        require(before >= sum(count * 8 for count, removal in zip(byte_counts, actual) if removal == moment), 'CPB departure exceeds available bits')
        peak = max(peak, before)
    # Parsed IDR/I_PCM pictures need no inter-picture prediction. IDR marking
    # discards the previous reference before storing the current one; the
    # Max(max_num_ref_frames,1) rule permits this even with declared count zero
    # (7.4.3.3/8.2.5). Output at removal and zero reordering need one DPB frame.
    require(1 <= min(MAX_DPB // macroblocks, 16), 'C.3(5) DPB limit')
    return {'rateBitsPerSecond': rate, 'cpbBits': capacity, 'inferredCbrFlag': 0, 'inferredLowDelay': 1, 'externalNumUnitsInTick': 3, 'externalTimeScale': 2 * original_count, 'hrdInitializedAtOriginalAu': init_index, 'externalBufferingPeriodIndices': list(range(init_index, init_index + n)), 'externalBufferingPeriodSpsId': 0, 'externalInitialDelay90k': 1, 'externalInitialDelayOffset90k': 0, 'externalCpbRemovalDelays': [2] * n, 'externalDpbOutputDelay': 0, 'peakCpbBits': str(peak), 'rows': rows}


def every_initialization(byte_counts, macroblocks, rate, capacity):
    # The same externally supplied messages and clock must work when the HRD
    # initializes at ANY BP, including the final IDR with only one AU remaining.
    n = len(byte_counts)
    return [schedule(byte_counts[start:], macroblocks, rate, capacity, n, start) for start in range(n)]


def admitted_bound(macroblocks, pictures):
    """Sufficient symbolic margins for every Plan-admitted count at this MB size."""
    frame = macroblocks * 580 + 64
    # <=3088 actual RBSP bits/MB =386 bytes, plus a bounded slice header.
    # At most one prevention byte per two input bytes; one NAL header remains.
    require(3 * (macroblocks * 386 + 16) <= 2 * (frame - 1), 'conservative escape/header bound')
    require(pictures * frame <= CAP - 8192, 'count exceeds conservative Plan admission')
    first = frame + 8192  # all out-of-band parameter bytes conservatively charged
    tick = Q(3, 2 * pictures)
    # All admitted frames have at least384 PCM bytes: arrival after initial
    # nominal removal, and even the conservative first AU arrives within one tick.
    require(Q(384 * 8, 162000000) > Q(1, 90000), 'minimum first AU arrival')
    require(Q(first * 8, 135000000) < tick < 1, 'first AU/earliest arrival margin')
    # Every BP yields tai(i)=2*i*tc, taf(i)<tai(i)+tc and
    # tr(i)=initial+(2*i+1)*tc. Thus each next nominal gap from the prior
    # actual removal is tc (not the output spacing2tc), and C.3(1) has
    # nominal(i)-taf(i-1)>initial+tc. These hold for every suffix/init point.
    require(tick >= max(Q(macroblocks, MBPS), Q(1, 172)), 'every-BP nominal/actual spacing')
    require(90000 * tick >= 1, 'every-BP initial delay margin')
    require(first <= 192 * (max(Q(macroblocks), Q(MBPS, 172)) + MBPS * tick), 'conservative first AU limit')
    require(frame <= 192 * MBPS * 2 * tick, 'conservative later AU limit')
    require(2 * tick >= max(Q(macroblocks, MBPS), Q(1, 172)), 'conservative output/removal spacing')
    require(CAP * 8 < 135000000, 'whole-stream CPB upper bound')


def verify(data):
    sps, pps, pictures, dimensions = movie_samples(data)
    width, height, macroblocks = parameter_sets(sps, pps)
    require(dimensions == [width, height], 'sample entry/SPS visible geometry mismatch')
    require(len(sps) + len(pps) <= 8192, 'parameter byte allowance')
    peaks = [pcm_picture(nal, macroblocks, i % 2) for i, nal in enumerate(pictures)]
    admitted_bound(macroblocks, len(pictures))
    require(all(len(nal) <= macroblocks * 580 + 64 for nal in pictures), 'actual NAL exceeds admission bound')
    vcl = [len(nal) for nal in pictures]
    nal = vcl.copy()
    nal[0] += len(sps) + len(pps)  # avcC parameters supplied before first IDR
    require(sum(nal) <= len(data), 'exact reconstructed NAL stream cap')
    return {'status': 'experiment passed; acceptance gate remains open pending independent review', 'standard': 'ITU-T H.264 (08/2024)', 'verifierSha256': hashlib.sha256(pathlib.Path(__file__).read_bytes()).hexdigest(), 'movieSha256': hashlib.sha256(data).hexdigest(), 'width': width, 'height': height, 'macroblocks': macroblocks, 'pictures': len(pictures), 'actualPeakMacroblockSyntaxBits': max(peaks), 'spsBytes': len(sps), 'ppsBytes': len(pps), 'externalPresence': {'NalHrdBpPresentFlag': 1, 'VclHrdBpPresentFlag': 1, 'CpbDpbDelaysPresentFlag': 1}, 'typeIAllInitializations': every_initialization(vcl, macroblocks, 135000000, 135000000), 'typeIIAllInitializations': every_initialization(nal, macroblocks, 162000000, 162000000), 'externalParametersEmbeddedInStream': False, 'externallyConveyedSeiBytesCountedInNalStream': False, 'appleConsumesThisScheduleVerified': False, 'uncoveredObligations': UNCOVERED}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--public-synthetic-movie', required=True, type=pathlib.Path)
    args = parser.parse_args()
    require(args.public_synthetic_movie.stat().st_size <= CAP, 'input cap')
    print(json.dumps(verify(args.public_synthetic_movie.read_bytes()), indent=2))

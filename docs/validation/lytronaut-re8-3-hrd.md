# Candidate external HRD conformance experiment

Status: **not accepted as a completed release gate**. The implementation and
interpretation require fresh independent review, followed by execution on the
largest padded generated resources. A small public fixture trial is useful
candidate evidence only. No production timing syntax is changed to suit this
experiment, and no overall Level 5 conformance pass is claimed.

Independent review rejected two parts of checkpoint `f55300a`: the single
external buffering period violated the every-IDR requirement, and composition
offsets/video edits were not rejected. That checkpoint and its original trial
remain historical candidate evidence, not a valid proof. The revised model
below supplies a buffering period at every IDR, checks every initialization
point, and rejects CTTS and video edit lists. Fresh review remains required.

The primary reference is [ITU-T H.264 (08/2024)](https://www.itu.int/rec/T-REC-H.264-202408-S/en),
especially Annex A.3.1/Table A-1; C.1–C.3; D.2.2–D.2.3; E.2.1–E.2.2.
The full primary document used for source review is retained with the external
acceptance evidence. No reference encoder implementation was used.

## Scope and accounting

`scripts/live-photo-hrd.py` is independent Python validation tooling. It reads
the actual public synthetic MOV, bounds atom/table extents, extracts the one
video track's sample descriptions, offsets, sizes and uniform timing, and reads
its `avcC` SPS/PPS. Video edit lists and any extra/duplicate video sample table,
including CTTS composition offsets, fail closed. Each sample must contain exactly one four-byte-length-prefixed
IDR NAL. MP4 length prefixes, box bytes and the separate metadata track do not
become video NAL bytes.

The supported SPS case is deliberately narrow: profile 66 with constraint byte
192, level 50, progressive 8-bit 4:2:0, picture-order type 2, no inter prediction,
the current explicit VUI restrictions, absent timing/HRD/pic_struct fields, and
the writer's exact PPS structure. Unsupported structures fail closed. The
parser removes and validates emulation-prevention bytes, reads every actual
slice header and I_PCM macroblock, bounds the complete pixel payload, and checks
all RBSP stop/alignment bits. It counts actual syntax bits rather than treating
the declared macroblock denominator as proof.

Type I counts the actual VCL NAL bytes in each picture. Type II reconstructs
the NAL unit stream by supplying the actual `avcC` SPS/PPS once before the first
IDR, then counts all those NAL bytes. This parameter insertion is explicit;
it is not an assertion that the configuration record appears inside an MP4
sample. It conservatively charges more first-AU bytes than the IDR alone.
There are no filler or additional supported NAL types.

## External schedule

E.2.1 notes 14/15 require missing HRD and buffering/picture-timing parameters to
be provided by means outside the bitstream for conformance checking. This
experiment supplies them in its report. **Apple's consumption of this external
schedule is not established.** Stored MOV presentation time and observed Apple
decoding are separate evidence.

An earlier CBR candidate was discarded during primary-source review. E.2.2
infers `cbr_flag=0`, so the revised model uses VBR arrival equations C-3/C-5.
It also respects the separately inferred rates and CPB sizes: 135,000,000 bits/s
and bits for VCL; 162,000,000 bits/s and bits for NAL. E.2.2/Table A-1 give those
values for Baseline Level 5. The model verifies that these parameters can be
represented by E-81/E-82 with scale zero and values in the permitted range.

For `N` pictures, the prospective external parameters are:

| Parameter | Value |
| --- | --- |
| Clock | `num_units_in_tick=3`, `time_scale=2*N`, hence `tc=3/(2*N)` seconds |
| Buffering periods | One at **every IDR/AU**; one CPB specification |
| Initial nominal removal delay | One tick of the **90 kHz delay clock**, hence `1/90000` second; this is distinct from `tc` |
| Initial delay offset | Zero; delay plus offset is constant at every buffering period |
| Picture removal delays | Two clock ticks at every AU, relative to the preceding buffering period; the first initialized AU's field is not used by C-7 |
| DPB output delay | Zero; output immediately at actual removal |
| Fixed frame rate / low delay | Inferred zero / one under E.2.1 |

Annex C's opening definition treats external conveyance as presence for C/D/E
semantics, although externally conveyed non-VCL bytes are not counted as actual
stream bytes. D.2.2 then requires a buffering period at every IDR. The model
therefore uses C-7 for initialization and C-8 at every subsequent AU: local
nominal removal is `1/90000+2*n*tc`. VBR arrival starts at the later of previous
final arrival and **nominal removal minus that AU's initial delay** (C-3/C-5),
not the earlier candidate's C-4 one-second window. Duration is actual AU bits
divided by the applicable inferred rate (C-6). C-10/C-11 determine actual
removal with exact upward clock quantization; supported streams must derive
`1/90000+(2*n+1)*tc`. Output spacing equals the MOV's `3/N` seconds, with a
common initial offset. Fractions stay exact throughout.

C.3 permits HRD initialization at any buffering period. Both Type I and Type II
are therefore simulated again at **every** original AU, including initialization
at the final IDR with one picture remaining. Every run uses the same original
clock, unchanged external message values and correct suffix byte counts. Only
the relative time origin resets. This is not a different parameter choice for
each favorable initialization.

## Obligations checked

| Obligation | Candidate check |
| --- | --- |
| A.3.1(a,b), C.3(4,7) | Every nominal/actual removal gap and output gap satisfies the coded macroblock processing interval and `1/172` floor |
| A.3.1(c,d) | Exact actual Type I/II first and later AU bytes compared with their own derived actual/nominal removal times and `MinCR=2` |
| A.3.1(e–g) | Parsed padded macroblock count and each coded axis satisfy frame-size limits; visible geometry and crop match the admitted bounds and sample entry |
| A.3.1(h), C.3(5) | Declared one decoded frame fits `min(110400/PicSizeInMbs,16)` |
| A.3.1(i,j), E.2.2 | Separate inferred VCL/NAL rate and CPB limits, field ranges and representability |
| A.3.1(k–m) | Actual slices contain only I_PCM; no inter-prediction motion vectors exist |
| A.3.1(n), E.2.1 restrictions | Every actual macroblock's syntax length is at most 3088 bits, below its declared and Level 5 ceiling of 3200; picture denominator zero is explicit |
| C.3(1), D.2.2 | Every IDR has an external buffering period. Each subsequent initial delay is checked against the ceiling of `90000*(nominalRemoval-finalArrivalOfPreviousAU)` in every initialization run. Delay is positive, fits its field and does not exceed CPB/rate time |
| C.3(2) | Piecewise-linear CPB occupancy checked just before departures and at final arrivals, including coincident events; departure cannot consume absent bits |
| C.3(3) | Inferred low-delay mode permits nominal underflow; actual removal still requires the complete AU |
| C.3(6), C.2, 7.4.3.3/8.2.5 | Parsed independent IDR/I_PCM pictures require no prediction from prior pictures. Each IDR discards the earlier reference, stores the current picture under `max(max_num_ref_frames,1)`, and outputs immediately; one DPB slot suffices |
| D.2.2–D.2.3 / E.2.1 | Every-AU external buffering/picture-timing messages, SPS id zero, identical delay/offset values and two-tick removal delays fit inferred 24-bit fields; no wrap, reordered or suppressed output; absent timing is handled explicitly |

The CPB scan covers all potential maxima: arrivals are sequential linear
segments, no overlapping delivery; occupancy can increase only during an
arrival and drops at removal. Both sides of coincident events are checked.
The DPB argument is specific to the independently parsed all-IDR/I_PCM subset,
not an implementation of arbitrary reference marking or frame reordering.

## Conservative admission margins

The independent geometry test enumerates every admitted even visible shape
and finds the maximum 8352 padded macroblocks. For every macroblock count up
to that maximum it checks the largest admitted picture count
`min(64,floor((16MiB-8192)/(580*M+64)))`. Smaller picture counts only increase
the clock and spacing margins at that macroblock count.

The complete actual I_PCM parser connects the bound to encoded data: at most
3088 RBSP syntax bits per macroblock, a bounded slice header, and at most one
emulation-prevention byte per two RBSP bytes fit the conservative `580*M+64`
NAL allowance. The first AU charges an additional whole 8192-byte parameter
allowance. The model checks actual SPS/PPS size against that allowance and each
actual NAL against its bound.

These inequalities are tested with exact fractions for all count/geometry
margins. An AU arrives after its nominal delay but within one `tc` of its
arrival start. By induction, starts are `2*n*tc`, final arrivals are earlier
than `(2*n+1)*tc`, and removals are `1/90000+(2*n+1)*tc`. The gap from previous
actual removal to next nominal removal is **one** `tc`, while output/actual
removal spacing is two `tc`. Both minima and the subsequent-buffering-period
delay inequality are checked. These arguments hold for every suffix. First/
later AU byte limits and whole-stream CPB occupancy retain sufficient margin.
The actual per-stream event simulation remains required; a passing conservative
calculation alone does not attest an unparsed movie or a future writer.

## Results and remaining review

Seven source-level adversarial/model tests pass in about 1.6 seconds, including
all admitted geometry/count margins and rejected malformed field/timing,
prevention, RBSP and container cases, every-IDR initialization and fail-closed
CTTS/video-edit mutations. A bounded trial on the frozen `c1b480f`
320×240 public MOV parses 300 macroblocks per picture and three pictures.
Actual VCL sizes are `[115804,115804,115804]`; reconstructed Type II sizes are
`[115823,115804,115804]`, including 15-byte SPS and 4-byte PPS. Both candidate
simulations pass; actual peak macroblock syntax is 3088 bits.

Still required: a fresh independent normative review and source-bound runs on
newly generated 1138×1842 / 1842×1138 largest padded cases. The public native
example now accepts optional `width height` after its output directory, while
the ordinary recipe remains 320×240. That test-only argument addition has not
been built or executed yet and needs canonical validation at final source.

This checker intentionally does **not** claim general H.264 syntax/semantics
conformance outside its supported subset, arbitrary profiles/NAL types/slices,
decoder implementation conformance, QuickTime pairing/metadata/color/JPEG
correctness, Apple schedule consumption, isolated Photos GUI recognition,
physical CPB/DPB memory behavior, latency, or presentation. The existing
platform decoder, pair verifier, owner import and performance gates remain
independent. Any uncovered applicable normative obligation discovered in review
must be addressed before this candidate can close the targeted size gate.

Below is a gap-analysis of the **current Rust port of the JB2 *encoder*** (the files you uploaded) versus the original DjVuLibre C++ implementation, followed by a concrete work plan.  I ignored the data-structure scaffolding you said is already “good enough” and looked only at *behavioural* differences.

---

## 1 · Missing or Incomplete Functional Units

| Area                            | What is still missing / only stubbed                                                                                                                           | Where it lives in C++                                                                            | Status in Rust                                                                                                                                  |
| ------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------- |
| **Adaptive binary contexts**    | `shift_direct_context`, `shift_cross_context`, plus full initialisation of the `bitdist[1024]` and `cbitdist[2048]` arrays.                                    | see static helpers in *JB2Image.h* lines around `get_direct_context` and `shift_direct_context`  | `get_*` functions ported, but the two *shift* helpers and array initialisation are `TODO`.                                                      |
| **Cross-coding encoder**        | Whole routine that refines a new bitmap against a library bitmap (`code_bitmap_by_cross_coding`).                                                              | *JB2EncodeCodec.cpp* → `code_bitmap_by_cross_coding` (encode side)                               | Block left as “placeholder” in `encoder.rs`                                                                                                     |
| **Direct bitmap encoder**       | Row-by-row adaptive coding (`code_bitmap_directly`).                                                                                                           | *JB2EncodeCodec.cpp* → `code_bitmap_directly` (encode side)                                      | Only the outer wrapper is there; inner loop marked TODO.                                                                                        |
| **Record generator**            | Emit all nine record types exactly as in C++ state-machine, incl. the *image-only / library-only* variants and *RequiredDictOrReset* injections.               | Large switch in *JB2EncodeCodec.cpp* `code_record(...)`                                          | Rust enum `RecordType` is correct, but encoder currently handles just a subset (no NEW\_MARK\_IMAGE\_ONLY, MATCHED\_REFINE\_IMAGE\_ONLY, etc.). |
| **Relative-location predictor** | Short-list and row/column predictors (`fill_short_list`, `update_short_list`, `code_relative_location`).                                                       | helpers in *JB2Image.h/cpp*                                                                      | `encoder.rs` keeps some `last_*` fields, but does not implement the short-list logic or the two-phase “same row / new row” predictor.           |
| **NumCoder**                    | – Golomb-like arithmetic coder with auto-allocation of new contexts; – `reset_numcoder()`; – automatic *RequiredDictOrReset* insertion when `cur_ncell>20000`. | `CodeNum`, `reset_numcoder` and threshold logic in *JB2Image.cpp*                                | Skeleton exists in `num_coder.rs` (BIG\_POSITIVE/NEGATIVE), but context tree growth, three-phase coding and reset threshold are not finished.   |
| **Library maintenance**         | Precise bounding-box computation (`LibRect::compute_bounding_box`) and centring maths for cross-coding.                                                        | helper at bottom of *JB2Image.cpp*                                                               | `LibRect::from_bitmap` is a “quick” box; lacks per-row / per-col scan logic and therefore breaks cross-coding alignment.                        |
| **Lossless-refinement flag**    | One-bit toggle coded just after *START\_OF\_DATA*.                                                                                                             | `code_eventual_lossless_refinement`                                                              | Not present.                                                                                                                                    |
| **Bitmap border handling**      | Calls to `bm.minborder()` and explicit spare rows above/below. Important because context windows read pixels at `[-2,+2]`.                                     | see `code_bitmap_*` helpers                                                                      | Not in Rust yet – may seg-fault on small shapes.                                                                                                |
| **Shape/bitmap compression**    | Final call to `bitmap.compress()` so the produced `GBitmap` stays in RLE form.                                                                                 | end of C++ `code_record` blocks                                                                  | No equivalent; memory footprint grows.                                                                                                          |
| **Error policy & range checks** | Hundreds of *G\_THROW(“bad\_number”)* assertions.                                                                                                              | everywhere                                                                                       | Rust has `Jb2Error`, but many range checks (`BIG_NEGATIVE..BIG_POSITIVE`) are commented out.                                                    |

---

## 2 · Plan to Finish the Port

1. **Finish the low-level context helpers**

   * Port `shift_direct_context` and `shift_cross_context` exactly; they are pure bit-twiddling with constant masks.
   * Replace the C++ signed/unsigned indexing with safe `get_unchecked()` reads inside a short `unsafe` block to avoid bounds checks but keep safety guarantees.

2. **Complete `NumCoder`**

   * Implement the *three-phase* coding loop from `JB2Codec::CodeNum` (phase 1 sign bit, phase 2 Golomb prefix, phase 3 binary search) .
   * Store contexts in `Vec<BitContext>`; allocate new ones lazily as in C++ (beware of `LEFT`/`RIGHT` child relationships).
   * Expose `needs_reset()` – true when `cur_ncell > 20000` – so the encoder can emit `RequiredDictOrReset` and call `reset()`.

3. **Implement bitmap writers**

   * **Direct-mode**: translate the row loop from `Encode::code_bitmap_directly` one-to-one, swapping `zp.encoder(bit, ctx)` for your `ZpEncoder::emit(bit, ctx)`.
   * **Cross-coding**: same strategy; keep `xd2c`/`yd2c` centring maths and double-pointer walk.  Important: call the **Rust** version of `shift_cross_context` every pixel.

4. **Relative-location predictor**

   * Port the `short_list[3]` ring buffer and `update_short_list` function.
   * Re-implement `code_relative_location` with the two branches (“same row” vs “new row”) and the four adaptive contexts (`rel_loc_x_current`, `rel_loc_x_last`, …).

5. **Record-type state machine**

   * Mirror the big C++ `switch` exactly, emitting all record types.
   * For image-only variants just skip the `add_to_library` call; for library-only variants skip the blit emission.

6. **Library bookkeeping**

   * Replace `LibRect::from_bitmap` with a faithful port of `compute_bounding_box`  (four directional scans).
   * In `add_to_library`, compute and push that rectangle so cross-coding has the correct offsets.

7. **Lossless-refinement flag**

   * Add a boolean field `refinement` to the encoder; right after the *START\_OF\_DATA* record call `CodeBit(refinement, dist_refinement_flag)`.

8. **Border guards**

   * Before coding any bitmap call `ensure_border(bm, min_left=2, min_right=2)` that pads with zeros so the context window never under-/over-runs.

9. **Compression of finished bitmaps**

   * If you store shapes as `GrayImage`, add a dummy `compress_if_needed()` that RLE-packs; or skip but document higher memory use.

10. **Error handling & fuzz safety**

    * Translate all `G_THROW` sites into `Err(Jb2Error::BadNumber)` etc.
    * Keep the original range limits (`BIG_POSITIVE = 262142`, `BIG_NEGATIVE = -262143`) to prevent malicious files from allocating huge vectors.

11. **Extensive Round-trip Tests**

    * Use the *decoder* side of DjVuLibre (or your own Rust decoder once ready) to round-trip-encode bitmap sets and compare MD5 of reconstructed images.
    * Add property-based tests (e.g. quickcheck) on the NumCoder: encode random ints and verify you can decode them back.

12. **Idiomatic Clean-ups (after parity)**

    * Replace raw `u8` pixel access loops with iterators or `bytemuck::cast_slice` where safe.
    * Leverage `Option<NonZeroUsize>` for context indices instead of sentinel `0`.
    * Gate the 20 000-context reset with a `const CELLCHUNK: usize = 20_000;`.

---

### Deliverable Checklist

| Task                                                                                   | Done when                        |
| -------------------------------------------------------------------------------------- | -------------------------------- |
| `shift_*` helpers ported & unit-tested                                                 | Context arrays update correctly  |
| `NumCoder` passes encode-decode quickcheck                                             | 1 000 000 random ints round-trip |
| `code_bitmap_directly` produces identical bitstreams to C++ for 50 random 64×64 shapes | byte-for-byte                    |
| Cross-coding bitstreams match on parent/child shape pairs                              |                                  |
| All record types emitted in correct order for sample page                              | decoder reads it without error   |
| Stress-test >20 000 contexts triggers automatic reset                                  | output has extra record 9        |
| Fuzz corpus of 10 000 bitmaps round-trips losslessly                                   | visual diff empty                |

Follow this order and you will converge quickly: finish context helpers → NumCoder → direct bitmap coding → small image encode → add cross-coding → full page encode → stress/reset path.

Good luck—once these pieces are in place the rest is polishing and ergonomic Rust!

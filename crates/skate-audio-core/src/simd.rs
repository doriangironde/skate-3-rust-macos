//! Keep the recovered audio kernels' lane layout across Intel and ARM64.
//!
//! Most ARM operations use `sse2neon`. The overrides below preserve SSE's
//! second-operand min/max, integer-indefinite conversion, and pairwise dot
//! product reduction, which differ from that crate's default NEON behavior.
#![allow(non_snake_case, unused_imports)]

#[cfg(target_arch = "aarch64")]
pub use arm::{_mm_cvttps_epi32, _mm_dp_ps, _mm_max_ps, _mm_min_ps};
#[cfg(target_arch = "x86_64")]
pub use core::arch::x86_64::*;
#[cfg(target_arch = "aarch64")]
pub use sse2neon::*;

#[cfg(target_arch = "aarch64")]
mod arm {
    use sse2neon::*;

    #[inline]
    pub fn _mm_min_ps(a: __m128, b: __m128) -> __m128 {
        _mm_blendv_ps(b, a, _mm_cmplt_ps(a, b))
    }

    #[inline]
    pub fn _mm_max_ps(a: __m128, b: __m128) -> __m128 {
        _mm_blendv_ps(b, a, _mm_cmpgt_ps(a, b))
    }

    #[inline]
    pub fn _mm_cvttps_epi32(a: __m128) -> __m128i {
        // SSE returns 0x80000000 for NaN and either direction of overflow.
        let valid = _mm_and_ps(
            _mm_cmpge_ps(a, _mm_set1_ps(-2147483648.0)),
            _mm_cmplt_ps(a, _mm_set1_ps(2147483648.0)),
        );
        _mm_castps_si128(_mm_blendv_ps(
            _mm_castsi128_ps(_mm_set1_epi32(i32::MIN)),
            _mm_castsi128_ps(sse2neon::_mm_cvttps_epi32(a)),
            valid,
        ))
    }

    #[inline]
    pub fn _mm_dp_ps<const IMM: i32>(a: __m128, b: __m128) -> __m128 {
        const { assert!(IMM >= 0 && IMM < 256) };
        let selected = _mm_set_epi32(
            if IMM & 0x80 != 0 { -1 } else { 0 },
            if IMM & 0x40 != 0 { -1 } else { 0 },
            if IMM & 0x20 != 0 { -1 } else { 0 },
            if IMM & 0x10 != 0 { -1 } else { 0 },
        );
        let product = _mm_and_ps(_mm_mul_ps(a, b), _mm_castsi128_ps(selected));
        let swapped = _mm_castsi128_ps(_mm_shuffle_epi32::<0xB1>(_mm_castps_si128(product)));
        let pairs = _mm_add_ps(product, swapped);
        let other_pair = _mm_castsi128_ps(_mm_shuffle_epi32::<0x4E>(_mm_castps_si128(pairs)));
        let sum = _mm_add_ps(pairs, other_pair);
        // Broadcast lane 0 so even exceptional lanes follow one reduction.
        let sum = _mm_castsi128_ps(_mm_shuffle_epi32::<0>(_mm_castps_si128(sum)));
        let output = _mm_set_epi32(
            if IMM & 8 != 0 { -1 } else { 0 },
            if IMM & 4 != 0 { -1 } else { 0 },
            if IMM & 2 != 0 { -1 } else { 0 },
            if IMM & 1 != 0 { -1 } else { 0 },
        );
        _mm_and_ps(sum, _mm_castsi128_ps(output))
    }
}

#[cfg(all(test, target_arch = "aarch64"))]
mod tests {
    use super::*;

    fn bits(v: __m128) -> [u32; 4] {
        let mut out = [0f32; 4];
        unsafe { _mm_storeu_ps(out.as_mut_ptr(), v) };
        out.map(f32::to_bits)
    }

    #[test]
    fn min_max_keep_the_second_operand_on_nan_or_equal_zero() {
        let a = _mm_setr_ps(f32::NAN, 2.0, -0.0, 0.0);
        let b = _mm_setr_ps(1.0, f32::from_bits(0x7fc01234), 0.0, -0.0);
        let expected = [1.0f32.to_bits(), 0x7fc01234, 0, 0x80000000];
        assert_eq!(bits(_mm_min_ps(a, b)), expected);
        assert_eq!(bits(_mm_max_ps(a, b)), expected);
    }

    #[test]
    fn float_to_integer_uses_sse_indefinite_for_invalid_lanes() {
        let a = _mm_setr_ps(f32::NAN, f32::INFINITY, 2147483648.0, -2147483648.0);
        assert_eq!(bits(_mm_castsi128_ps(_mm_cvttps_epi32(a))), [0x80000000; 4]);
        let a = _mm_setr_ps(-1.9, 1.9, -2147483904.0, 2147483520.0);
        assert_eq!(
            bits(_mm_castsi128_ps(_mm_cvttps_epi32(a))),
            [(-1i32) as u32, 1, 0x80000000, 2147483520]
        );
    }

    #[test]
    fn dot_product_rounds_each_pair_before_the_final_add() {
        // A sequential reduction loses the 3 before cancellation and yields 4.
        let a = _mm_setr_ps(1.0e20, 3.0, -1.0e20, 4.0);
        let b = _mm_set1_ps(1.0);
        assert_eq!(bits(_mm_dp_ps::<0xff>(a, b)), [0; 4]);
        assert_eq!(bits(_mm_dp_ps::<0x21>(a, b)), [3.0f32.to_bits(), 0, 0, 0]);
    }
}

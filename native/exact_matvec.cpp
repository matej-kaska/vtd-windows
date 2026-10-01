#include "exact_matvec.h"

#include <cstdint>
#include <cstring>
#include <immintrin.h>

namespace {
// Match the pinned AVX2/FMA ggml_vec_dot_f32: four independent accumulators,
// then the same reduction tree. Expand each original Q8_0 block in registers;
// neither weights nor activations undergo any additional quantization.
void exact_matvec(ggml_tensor *dst, int ith, int nth, void *) {
  const auto *weights = dst->src[0];
  const auto *input = dst->src[1];
  const int64_t begin = weights->ne[1] * ith / nth;
  const int64_t end = weights->ne[1] * (ith + 1) / nth;
  const int64_t block_count = weights->ne[0] / 32;
  for (int64_t i = begin; i < end; ++i) {
    __m256 sums[4] = {_mm256_setzero_ps(), _mm256_setzero_ps(),
                      _mm256_setzero_ps(), _mm256_setzero_ps()};
    const auto *blocks =
        static_cast<const uint8_t *>(weights->data) + i * weights->nb[1];
    const auto *x = static_cast<const float *>(input->data);
    for (int64_t b = 0; b < block_count; ++b) {
      uint16_t half;
      std::memcpy(&half, blocks + b * 34, sizeof(half));
      const auto scale =
          _mm256_broadcastss_ps(_mm_cvtph_ps(_mm_cvtsi32_si128(half)));
      for (int j = 0; j < 4; ++j) {
        const auto integers = _mm256_cvtepi8_epi32(_mm_loadl_epi64(
            reinterpret_cast<const __m128i *>(blocks + b * 34 + 2 + j * 8)));
        const auto values = _mm256_mul_ps(scale, _mm256_cvtepi32_ps(integers));
        sums[j] = _mm256_fmadd_ps(values, _mm256_loadu_ps(x + b * 32 + j * 8),
                                  sums[j]);
      }
    }
    const auto total = _mm256_add_ps(_mm256_add_ps(sums[0], sums[2]),
                                     _mm256_add_ps(sums[1], sums[3]));
    const auto low = _mm_add_ps(_mm256_castps256_ps128(total),
                                _mm256_extractf128_ps(total, 1));
    const auto half = _mm_hadd_ps(low, low);
    static_cast<float *>(dst->data)[i] = _mm_cvtss_f32(_mm_hadd_ps(half, half));
  }
}
} // namespace

bool vtd_exact_q8_supported(const ggml_tensor *weights) noexcept {
  return weights != nullptr && weights->type == GGML_TYPE_Q8_0 &&
         weights->ne[0] > 0 && weights->ne[0] % 32 == 0 &&
         weights->ne[2] == 1 && weights->ne[3] == 1 &&
         ggml_is_contiguous(weights);
}

ggml_tensor *vtd_exact_q8_mul_mat(ggml_context *ctx, ggml_tensor *weights,
                                  ggml_tensor *input) {
  GGML_ASSERT(vtd_exact_q8_supported(weights));
  GGML_ASSERT(input->type == GGML_TYPE_F32 && ggml_is_contiguous(input));
  GGML_ASSERT(input->ne[0] == weights->ne[0] &&
              ggml_nelements(input) == input->ne[0]);
  ggml_tensor *args[] = {weights, input};
  return ggml_custom_4d(ctx, GGML_TYPE_F32, weights->ne[1], 1, 1, 1, args, 2,
                        exact_matvec, GGML_N_TASKS_MAX, nullptr);
}
